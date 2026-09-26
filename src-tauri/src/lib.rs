mod commands;
mod native;

use commands::{
    auth, chain_backend, logs, maker, maker_reports, maker_settings, maker_wallet, market, setup,
    shutdown, taker_reports, taker_swap, taker_wallet,
};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use std::sync::Arc;

use portal_core::events::Envelope;
use portal_core::state::AppState;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver;

/// Label of the window declared in `tauri.conf.json`.
const MAIN_WINDOW: &str = "main";

pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Forwards core's domain events onto Tauri's per-window event channel. Core publishes without
/// knowing which host is listening; this is the desktop half of that contract.
fn bridge_events(app: AppHandle, mut events: Receiver<Envelope>) {
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                // One window, one session: every wallet event is this window's.
                Ok(envelope) => {
                    let _ = app.emit(envelope.event.name(), envelope.event.payload());
                }
                // Keep serving: a lagging bridge has missed notifications, not the state
                // itself, and every page reconciles from a fresh read on its next poll.
                Err(RecvError::Lagged(skipped)) => {
                    log::warn!("event bridge fell behind and dropped {skipped} events")
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

/// Commands that run before signing in: the sign-in itself, and quitting.
const OPEN_COMMANDS: &[&str] = &["auth_session", "auth_claim", "auth_login", "quit_app"];

/// Refuses every other command until the owner password has been given, in Rust rather than
/// only by hiding screens — the same line the web host draws with its session check.
fn require_sign_in<R: tauri::Runtime>(
    handler: impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        let open = OPEN_COMMANDS.contains(&invoke.message.command());
        if !open && !invoke.message.webview().state::<Arc<auth::DesktopAuth>>().signed_in() {
            invoke.resolver.reject(portal_core::error::AppError::new(
                portal_core::error::ErrorCode::AuthorizationDenied,
                "sign in first",
            ));
            return true;
        }
        handler(invoke)
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default();

    // Closing the window only hides it, so a launch while an earlier instance is still alive
    // would otherwise leave two of everything — two Tor instances, two trays, two makers
    // fighting over the same ports. The second launch surfaces the first instead.
    //
    // Release only, and the whole plugin rather than just its callback: the second process
    // exits either way, so in development this would hand `tauri dev` back the *old* running
    // binary and silently discard the rebuild, making every fix look like it did nothing.
    #[cfg(not(debug_assertions))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }));
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // Arc, not a bare AppState: commands take `State<Arc<AppState>>` so background work
        // can own a handle that outlives the request. Managing the wrong shape compiles and
        // then fails every stateful command at runtime.
        .manage(Arc::new(portal_core::state::AppState::default()))
        .manage(Arc::new(auth::DesktopAuth::load()))
        .invoke_handler(require_sign_in(tauri::generate_handler![
            // signing in with the owner password
            auth::auth_session,
            auth::auth_claim,
            auth::auth_login,
            auth::auth_logout,
            // setup / connectivity
            setup::check_tor,
            setup::restart_tor_bootstrap,
            // chain backend selection
            chain_backend::get_chain_backend,
            chain_backend::get_electrum_presets,
            chain_backend::set_chain_backend,
            chain_backend::check_backend,
            // taker wallet lifecycle
            taker_wallet::list_wallets,
            taker_wallet::init_taker,
            taker_wallet::shutdown_taker,
            taker_wallet::get_paths,
            taker_wallet::get_session_state,
            taker_wallet::get_wallet_info,
            taker_wallet::choose_restore_backup,
            taker_wallet::restore_wallet,
            taker_wallet::backup_wallet,
            // taker wallet operations
            taker_wallet::get_balances,
            taker_wallet::validate_address,
            taker_wallet::get_new_address,
            taker_wallet::verify_last_address,
            taker_wallet::get_transactions,
            taker_wallet::list_utxos,
            taker_wallet::send_to_address,
            taker_wallet::sync_wallet,
            taker_wallet::estimate_fees,
            taker_wallet::get_btc_price,
            // market / offerbook
            market::get_offers,
            market::sync_offerbook,
            market::poll_maker,
            market::remove_maker,
            // taker swap
            taker_swap::prepare_swap,
            taker_swap::estimate_swap_funding,
            taker_swap::start_swap,
            taker_swap::get_swap_progress,
            taker_swap::get_swap_tracker,
            taker_swap::get_swap_preparation,
            taker_swap::recover_swap,
            taker_swap::get_recovery_status,
            taker_swap::list_recoveries,
            // taker reports
            taker_reports::list_swap_reports,
            taker_reports::get_swap_report,
            taker_reports::get_incoming_swap_utxo,
            taker_reports::verify_deniability,
            // taker logs
            logs::get_logs,
            // maker lifecycle
            maker::init_maker,
            maker::update_maker_settings,
            maker::start_maker,
            maker::stop_maker,
            maker::get_maker_status,
            maker::get_maker_info,
            // maker reports
            maker_reports::list_maker_swap_reports,
            maker_reports::get_maker_swap_report,
            maker_reports::verify_maker_deniability,
            // maker's own wallet
            maker_wallet::get_maker_balances,
            maker_wallet::list_maker_utxos,
            maker_wallet::get_maker_new_address,
            maker_wallet::get_maker_transactions,
            maker_wallet::send_maker_to_address,
            maker_wallet::sync_maker_wallet,
            maker_wallet::list_maker_fidelity_bonds,
            // maker settings (persisted, non-secret config)
            maker_settings::list_makers,
            maker_settings::get_saved_maker_settings,
            maker_settings::list_dashboard_imports,
            maker_settings::import_dashboard_makers,
            maker_settings::clear_maker_settings,
            maker::get_router_defaults,
            maker_settings::get_suggested_maker_ports,
            maker_settings::check_maker_ports,
            // maker logs
            logs::get_maker_logs,
            // app lifecycle
            shutdown::quit_app,
        ]))
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Closing the UI must not tear down a running swap or an active maker,
                // so the window is hidden instead of destroyed. Quitting is explicit:
                // the tray menu, the app menu, or Cmd+Q.
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            // Before anything can publish: the setup hook runs ahead of the window, so no
            // command has been able to emit yet and the bridge cannot miss a startup event.
            bridge_events(app.handle().clone(), app.state::<Arc<AppState>>().events.subscribe());

            // Before anything logs, so startup, Tor and a restore that runs before any wallet is
            // open all reach the app's `debug.log` rather than the terminal.
            if let Ok(root) = portal_core::storage::openswap_root() {
                portal_core::logging::set_log_dir(root);
            }
            portal_core::tor::sweep_stale_tor_dirs();

            // Earlier versions persisted the backend, RPC password included. Ceasing to
            // write it is not enough — the old file has to go.
            portal_core::ops::chain_backend::remove_legacy_config();

            // Tor first, before any wallet or maker exists: everything downstream binds to
            // its ports, and a cold start needs ~45s that overlaps the user picking a wallet.
            std::thread::spawn(|| {
                if let Err(e) = portal_core::tor::ensure_tor() {
                    log::error!("Portal's Tor failed to start: {e}");
                }
            });

            // Replaces the predefined Quit, whose native terminate lands in `RunEvent::Exit`
            // already inside the OS termination watchdog — too late to stop a maker's closing
            // wallet sync properly. Tauri builds the macOS app submenu first with Quit last.
            let menu = Menu::default(app.handle())?;
            // macOS only: it is the platform with an application submenu holding a predefined
            // Quit, and the only one where the menu bar is the usual way out. Everywhere else
            // the tray item below is that route, so building this item off macOS would leave
            // it unattached — dead on Linux and Windows, and a build failure under
            // `-D warnings`.
            #[cfg(target_os = "macos")]
            {
                let app_quit =
                    MenuItem::with_id(app, "quit", "Quit Portal", true, Some("CmdOrCtrl+Q"))?;
                if let Some(app_menu) = menu.items()?.first().and_then(|item| item.as_submenu()) {
                    if let Some(predefined_quit) = app_menu.items()?.last() {
                        app_menu.remove(predefined_quit)?;
                    }
                    app_menu.append(&app_quit)?;
                }
            }
            app.set_menu(menu)?;
            app.on_menu_event(|app, event| {
                if event.id.as_ref() == "quit" {
                    shutdown::begin_quit(app);
                }
            });

            let open = MenuItem::with_id(app, "open", "Open Portal", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "tray-quit", "Quit Portal", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("Portal")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main_window(app),
                    "tray-quit" => shutdown::begin_quit(app),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        show_main_window(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| match event {
            // `code: None` means the last window went away, which here is not a quit
            // request — the tray keeps the app reachable with no window open.
            RunEvent::ExitRequested {
                code: None, api, ..
            } => api.prevent_exit(),
            RunEvent::Exit => shutdown::shutdown_on_exit(app),
            #[cfg(target_os = "macos")]
            RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } => show_main_window(app),
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
        text.split_once(start)
            .and_then(|(_, rest)| rest.split_once(end))
            .map(|(body, _)| body)
            .unwrap_or_else(|| panic!("{start} … {end} not found"))
    }

    /// The `command` half of every `module::command` path, ignoring comment lines — the bodies
    /// being scanned carry prose that would otherwise parse as command names.
    fn registered(body: &str) -> BTreeSet<String> {
        let ident = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        body.lines()
            .map(|line| line.trim())
            .filter(|line| !line.starts_with("//"))
            .flat_map(|line| line.split(',').map(str::trim).collect::<Vec<_>>())
            .filter_map(|token| token.split_once("::"))
            .map(|(_, command)| command.trim().to_string())
            .filter(|command| ident(command))
            .collect()
    }

    /// The managed type and every lookup of it must agree. They are matched by type at
    /// runtime, not compile time, so a mismatch builds cleanly and then panics on the first
    /// stateful command — which the connection gate does not reach.
    #[test]
    fn managed_state_is_the_type_commands_look_up() {
        let lib = read(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"));
        // Only the code, not this module: the assertions below contain the very pattern
        // they forbid.
        let code = lib.split("mod tests {").next().unwrap_or_default();
        assert!(
            code.contains(".manage(Arc::new(portal_core::state::AppState::default()))"),
            "AppState must be managed as Arc<AppState>"
        );
        let bare = format!("state::<{}>()", "AppState");
        assert!(
            !code.contains(&bare),
            "every lookup must ask for Arc<AppState>, not the bare type"
        );
    }

    /// Three hand-maintained lists have to name the same commands: `build.rs` generates one
    /// permission per entry, `generate_handler!` registers the implementations, and the
    /// capability grants them. A command missing from `build.rs` has no permission for the
    /// capability to grant, and one missing from the capability is rejected at the IPC
    /// boundary before it reaches Rust — surfacing in the UI as whatever the caller's `catch`
    /// happens to say. Both drift silently past the compiler, so they are asserted here.
    #[test]
    fn command_permissions_match_registered_commands() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));

        let lib = read(&root.join("src/lib.rs"));
        let handler = registered(between(&lib, "tauri::generate_handler![", "])"));
        assert!(handler.len() > 50, "parsed too few commands: {handler:?}");

        let build = read(&root.join("build.rs"));
        let generated: BTreeSet<String> = between(&build, "const COMMANDS: &[&str] = &[", "];")
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect();
        assert_eq!(
            handler, generated,
            "build.rs COMMANDS and generate_handler! disagree"
        );

        // The inventory is what the web host's route table will be checked against too, so a
        // command added on one side and forgotten in the other has to fail here.
        let contracts = read(&root.parent().unwrap().join("contracts/operations.json"));
        let inventory: BTreeSet<String> = contracts
            .split("\"name\": \"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .map(str::to_string)
            .collect();
        assert_eq!(
            handler, inventory,
            "contracts/operations.json and generate_handler! disagree"
        );

        let capability = read(&root.join("capabilities/default.json"));
        let missing: Vec<String> = handler
            .iter()
            .map(|c| format!("allow-{}", c.replace('_', "-")))
            .filter(|p| !capability.contains(&format!("\"{p}\"")))
            .collect();
        assert!(missing.is_empty(), "capability is missing: {missing:?}");
    }
}
