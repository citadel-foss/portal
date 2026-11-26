//! The web half of the in-app update: a downloaded `portal-server` replaces its own file and
//! starts again in the same process. Fetching, verifying and staging live in
//! `portal_core::update`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use portal_core::error::{AppError, ErrorCode};
use portal_core::state::AppState;
use portal_core::update::{self, Placement, UpdateInfo};

/// Set in the process an update started, so it does not open a second browser tab.
pub const RELAUNCHED_ENV: &str = "PORTAL_RELAUNCHED";

/// Read once at startup: after the rename, Linux reports the running file as `… (deleted)`.
static EXE: OnceLock<PathBuf> = OnceLock::new();
static RESTART: AtomicBool = AtomicBool::new(false);

/// `external_assets`: the UI comes from `--assets-dir`, which an update would leave as it is
/// and keep serving over the new binary's own, so such a server is never updated in place.
pub fn remember_exe(external_assets: bool) {
    if external_assets {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = EXE.set(exe);
        sweep_staging();
    }
}

/// The release asset for this platform, if CI builds one.
fn asset() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("portal-server-linux-x86_64.tar.gz")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("portal-server-linux-arm64.tar.gz")
    } else if cfg!(target_os = "macos") {
        Some("portal-server-macos-universal.tar.gz")
    } else {
        None
    }
}

/// The binary to replace, if this one may replace itself: a release build serving its own
/// embedded frontend (only CI embeds one; a development build or the container image carries
/// none), on a platform with a release asset, in a folder this process can write.
fn target() -> Option<&'static Path> {
    let exe = EXE.get()?;
    (cfg!(unix) && crate::assets::has_embedded() && asset().is_some() && update::replaceable(exe))
        .then_some(exe.as_path())
}

pub fn check() -> Result<UpdateInfo, AppError> {
    update::check(target().is_some())
}

/// Puts the latest build in place, then stops the server through the same ordered teardown a
/// signal runs; `main` starts the new binary once it is done.
pub fn install(state: &AppState) -> Result<(), AppError> {
    let (Some(target), Some(asset)) = (target(), asset()) else {
        return Err(AppError::new(
            ErrorCode::UpdateFailed,
            "this copy of Portal can't replace itself; download the update instead",
        ));
    };
    update::install(
        state,
        Placement {
            target: target.to_path_buf(),
            asset,
            unpack: |download: &Path, staging: &Path| {
                update::untar(download, staging)?;
                if !staging.join("portal-server").is_file() {
                    return Err(AppError::new(
                        ErrorCode::UpdateFailed,
                        "the downloaded archive holds no portal-server",
                    ));
                }
                Ok(())
            },
            // A rename, so the running binary keeps its own inode until the process execs.
            commit: |_: &Path, staging: &Path| {
                std::fs::rename(staging.join("portal-server"), target)
                    .map_err(update::io_failed("couldn't put the new server in place"))
            },
        },
    )?;
    RESTART.store(true, Ordering::SeqCst);
    #[cfg(unix)]
    portal_core::shutdown_signal::trip();
    Ok(())
}

/// Called once teardown has finished. Replaces this process with the updated binary, keeping
/// its PID, arguments, environment and terminal, so whatever supervises it sees no exit.
pub fn relaunch_if_requested() -> Result<(), Box<dyn std::error::Error>> {
    if !RESTART.load(Ordering::SeqCst) {
        return Ok(());
    }
    let exe = EXE.get().ok_or("the server's own path is unknown")?;
    #[cfg(unix)]
    let error = {
        use std::os::unix::process::CommandExt;
        std::process::Command::new(exe)
            .args(std::env::args_os().skip(1))
            .env(RELAUNCHED_ENV, "1")
            .exec()
    };
    #[cfg(not(unix))]
    let error = format!("no way to start {} in place", exe.display());
    Err(format!("couldn't start the updated server: {error}").into())
}

fn sweep_staging() {
    if let Some(target) = target() {
        update::sweep_staging(target);
    }
}
