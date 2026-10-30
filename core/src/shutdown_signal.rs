//! Termination signals, owned by the host.
//!
//! [`arm`] installs a handler that only sets a flag, and the host awaits [`wait`] to run its
//! ordered teardown: makers, then the taker, then Tor. Tor runs as a child process in its own
//! process group, so a signal meant for Portal reaches Portal alone and Tor is halted last, by
//! that teardown, rather than going down under the makers that still route through it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static TRIPPED: AtomicBool = AtomicBool::new(false);

/// Poll interval for [`wait`]. Invisible against a teardown measured in seconds.
const POLL: Duration = Duration::from_millis(150);

#[cfg(unix)]
extern "C" fn on_signal(_sig: libc::c_int) {
    // The only thing a handler is allowed to do. No allocation, no logging, no locks.
    TRIPPED.store(true, Ordering::SeqCst);
}

/// Claims `SIGTERM` and `SIGINT`. Only for a host that awaits [`wait`].
#[cfg(unix)]
pub fn arm() {
    install();
}

#[cfg(not(unix))]
pub fn arm() {}

#[cfg(unix)]
fn install() {
    unsafe {
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGTERM, handler);
        libc::signal(libc::SIGINT, handler);
    }
}

/// Resolves when the process has been asked to stop.
#[cfg(unix)]
pub async fn wait() {
    while !TRIPPED.load(Ordering::SeqCst) {
        tokio::time::sleep(POLL).await;
    }
}

/// No POSIX signals to take, so nothing would ever set the flag; fall back to the runtime's.
#[cfg(not(unix))]
pub async fn wait() {
    let _ = tokio::signal::ctrl_c().await;
}
