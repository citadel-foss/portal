//! Quit orchestration. Quitting is deliberate — the app menu, Cmd+Q, or the tray — and it
//! tears the process down in dependency order rather than letting the OS drop it:
//! makers, then the taker, then Tor last, because both of the first two route through it.

use std::sync::Arc;

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter, Manager};

use portal_core::error::AppError;
use portal_core::state::{AppState, TakerInstance};
use portal_core::types::{MakerPhase, QuitBlockers};

/// Latched by whichever path starts the teardown, so a second Quit while one is already
/// running is ignored and `RunEvent::Exit` does not repeat the work.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Held while `begin_quit` waits on the blocker check, so repeated quit gestures during a slow
/// wallet sync share one answer instead of each showing the dialog or quitting on its own.
static CHECKING: AtomicBool = AtomicBool::new(false);

/// Work that a quit would interrupt rather than finish, so the user is asked first.
fn quit_blockers(state: &AppState) -> QuitBlockers {
    // Read from the wallet rather than `active_swap`: recovery outlives the swap that started it
    // and survives a restart, and quitting mid-recovery is the one thing that actually stalls it.
    let recovering = |taker: &Arc<TakerInstance>| {
        taker
            .wallet
            .read()
            .is_ok_and(|w| !w.list_live_contract_spend_info().is_empty())
    };
    // Until a snapshot holds no wallet left to read: each read can wait out a sync while the window
    // stays usable, and a wallet opened, or a swap or router started, meanwhile must still count.
    let mut checked: Vec<Arc<TakerInstance>> = Vec::new();
    let mut recovery_running = false;
    let takers = loop {
        let takers = state.open_takers();
        let unseen: Vec<_> = takers
            .iter()
            .filter(|taker| !checked.iter().any(|seen| Arc::ptr_eq(seen, taker)))
            .cloned()
            .collect();
        if unseen.is_empty() {
            break takers;
        }
        recovery_running = recovery_running || unseen.iter().any(recovering);
        checked.extend(unseen);
    };
    let swap_running = takers.iter().any(|taker| taker.swap_running());
    // Keyed on phase, not on `runtime` being present: a server thread that exits on its own
    // sets Stopped or Failed but leaves its runtime in place, and quitting would then warn
    // about a maker that finished long ago.
    let running_routers = state
        .makers
        .lock()
        .map(|makers| {
            makers
                .values()
                .filter(|entry| {
                    matches!(
                        entry.phase,
                        MakerPhase::Initializing
                            | MakerPhase::Starting
                            | MakerPhase::Running
                            | MakerPhase::Stopping
                    )
                })
                .map(|entry| entry.settings.router_id.clone())
                .collect()
        })
        .unwrap_or_default();
    QuitBlockers {
        swap_running,
        recovery_running,
        running_routers,
    }
}

/// Entry point for every deliberate quit gesture. Asks first when something is mid-flight,
/// otherwise goes straight to teardown.
pub fn begin_quit(app: &AppHandle) {
    if QUITTING.load(Ordering::SeqCst) || CHECKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    // Off the main thread: a wallet sync holds the wallet's write lock for its whole run, and
    // the recovery check waiting behind it would freeze every window until the sync ended.
    std::thread::spawn(move || {
        let blockers = quit_blockers(&app.state::<Arc<AppState>>());
        CHECKING.store(false, Ordering::SeqCst);
        if blockers.swap_running
            || blockers.recovery_running
            || !blockers.running_routers.is_empty()
        {
            crate::show_main_window(&app);
            let _ = app.emit("app://quit-blocked", blockers);
            return;
        }
        run_quit(app);
    });
}

/// Routes the Dock's Quit, and any other user-initiated `terminate:`, through `begin_quit`. tao's
/// app delegate has no `applicationShouldTerminate:`, so without this the terminate goes straight
/// to `RunEvent::Exit` and the whole teardown runs on the main thread with nothing on screen.
/// tao's own exit stops the run loop and calls `process::exit`, never `terminate:`, so
/// cancelling here cannot also cancel the exit that ends `run_quit`.
#[cfg(target_os = "macos")]
pub fn route_native_terminate(app: &AppHandle) {
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{sel, MainThreadMarker};
    use objc2_app_kit::NSApplication;
    use objc2_foundation::NSAppleEventManager;
    use std::sync::OnceLock;

    static APP: OnceLock<AppHandle> = OnceLock::new();
    /// `kAEQuitReason`, which the system sets on the quit event for a logout, restart or shutdown.
    const QUIT_REASON: u32 = u32::from_be_bytes(*b"why?");
    const TERMINATE_CANCEL: usize = 0;
    const TERMINATE_NOW: usize = 1;

    extern "C-unwind" fn should_terminate(_: &AnyObject, _: Sel, _: &AnyObject) -> usize {
        // Cancelling would abort the whole logout or restart, so those keep the native path.
        let by_system = NSAppleEventManager::sharedAppleEventManager()
            .currentAppleEvent()
            .is_some_and(|event| {
                // SAFETY: `attributeDescriptorForKeyword:` takes an `AEKeyword` (u32) and returns
                // a nullable descriptor.
                let reason: Option<objc2::rc::Retained<AnyObject>> = unsafe {
                    objc2::msg_send![&*event, attributeDescriptorForKeyword: QUIT_REASON]
                };
                reason.is_some()
            });
        if by_system {
            return TERMINATE_NOW;
        }
        if let Some(app) = APP.get() {
            begin_quit(app);
        }
        TERMINATE_CANCEL
    }

    let _ = APP.set(app.clone());
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(delegate) = NSApplication::sharedApplication(mtm).delegate() else {
        log::warn!("no app delegate; the Dock's Quit will skip the quit dialog");
        return;
    };
    let class: *const AnyClass = AsRef::<AnyObject>::as_ref(&*delegate).class();
    // SAFETY: the signature matches `applicationShouldTerminate:` (`Q@:@` on 64-bit), and the
    // delegate class lives for the whole process.
    let added = unsafe {
        let imp: Imp = std::mem::transmute(
            should_terminate as extern "C-unwind" fn(&AnyObject, Sel, &AnyObject) -> usize,
        );
        objc2::ffi::class_addMethod(
            class as *mut AnyClass,
            sel!(applicationShouldTerminate:),
            imp,
            c"Q@:@".as_ptr(),
        )
    };
    if !added.as_bool() {
        log::warn!("app delegate already answers applicationShouldTerminate:; left as is");
    }
}

/// Confirms a quit the user was warned about.
#[tauri::command]
pub fn quit_app(app: AppHandle) -> Result<(), AppError> {
    run_quit(app);
    Ok(())
}

fn run_quit(app: AppHandle) {
    if QUITTING.swap(true, Ordering::SeqCst) {
        return;
    }
    // Shown so the teardown is visible rather than looking like a hung quit; a maker's
    // final wallet sync can run for a while and is deliberately not cut short.
    crate::show_main_window(&app);
    let _ = app.emit("app://quitting", ());
    std::thread::spawn(move || {
        shutdown_runtime(&app);
        app.exit(0);
    });
}

/// Best-effort teardown for a quit that skipped `begin_quit` and landed straight in
/// `RunEvent::Exit`, already inside the termination watchdog. `begin_quit` has normally
/// finished by then and this is a no-op.
pub fn shutdown_on_exit(app: &AppHandle) {
    if QUITTING.swap(true, Ordering::SeqCst) {
        return;
    }
    shutdown_runtime(app);
}

fn shutdown_runtime(app: &AppHandle) {
    let state = app.state::<Arc<AppState>>();

    // Makers first: each one finishes its in-flight connections and a closing wallet sync,
    // and all of that traffic is still riding on Tor.
    // Logged as well as emitted: the progress events reach a window that is closing, so the
    // log is the only place the sequence can be checked afterwards.
    let _ = app.emit("app://quit-progress", "Stopping routers");
    log::info!("shutdown phase start: routers");
    portal_core::ops::maker::shutdown_all(&state);
    log::info!("shutdown phase done: routers");

    // Then the wallets, waiting for each one's final save. Dropping a Taker is the graceful
    // path: the crate's `Drop` saves the wallet and stops the recovery loop and breach detector.
    // A running swap holds its taker mutex for the whole swap, so that wallet is left to the swap
    // thread — the coins the swap commits to the wallet are what the next launch recovers from.
    let _ = app.emit("app://quit-progress", "Stopping wallet");
    log::info!("shutdown phase start: wallet");
    portal_core::ops::taker_wallet::shutdown_all(&state);
    log::info!("shutdown phase done: wallet");

    let _ = app.emit("app://quit-progress", "Stopping Tor");
    log::info!("shutdown phase start: tor");
    portal_core::tor::shutdown();
    log::info!("shutdown complete");
}
