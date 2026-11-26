//! The desktop half of the in-app update: which installs Portal can replace on its own — a macOS
//! `.app` or a Linux AppImage in a writable folder — and how. The rest are pointed at the release
//! page. Fetching, verifying and staging live in `portal_core::update`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::{AppHandle, Manager, WebviewWindow};

use portal_core::error::{AppError, ErrorCode};
use portal_core::state::AppState;
use portal_core::types::QuitBlockers;
use portal_core::update::{self, Changes, Placement, UpdateInfo};

enum Install {
    MacBundle(PathBuf),
    AppImage(PathBuf),
}

impl Install {
    fn path(&self) -> &Path {
        match self {
            Self::MacBundle(path) | Self::AppImage(path) => path,
        }
    }

    /// Where this process was launched from, if Portal may swap it. A `tauri dev` binary, a
    /// translocated (still-quarantined) bundle, a package-manager install or a read-only folder
    /// all return `None`.
    fn current() -> Option<Self> {
        let install = if cfg!(target_os = "macos") {
            let exe = std::env::current_exe().ok()?;
            let bundle = exe
                .ancestors()
                .find(|p| p.extension().is_some_and(|e| e == "app"))?;
            if bundle.to_string_lossy().contains("/AppTranslocation/") {
                return None;
            }
            Self::MacBundle(bundle.to_path_buf())
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Self::AppImage(PathBuf::from(std::env::var_os("APPIMAGE")?))
        } else {
            return None;
        };
        update::replaceable(install.path()).then_some(install)
    }
}

#[tauri::command]
pub async fn check_update() -> Result<UpdateInfo, AppError> {
    tauri::async_runtime::spawn_blocking(|| update::check(Install::current().is_some()))
        .await
        .map_err(AppError::internal)?
}

#[tauri::command]
pub async fn update_changes() -> Result<Changes, AppError> {
    tauri::async_runtime::spawn_blocking(update::changes)
        .await
        .map_err(AppError::internal)?
}

/// What has to stop before an update may install. Blocking: a wallet read can wait out a sync.
#[tauri::command]
pub async fn update_blockers(app: AppHandle) -> Result<QuitBlockers, AppError> {
    let state = Arc::clone(&app.state::<Arc<AppState>>());
    tauri::async_runtime::spawn_blocking(move || state.running_work())
        .await
        .map_err(AppError::internal)
}

/// Downloads, verifies and swaps in the latest build, then quits and relaunches through the
/// normal teardown.
#[tauri::command]
pub async fn install_update(app: AppHandle, window: WebviewWindow) -> Result<(), AppError> {
    crate::native::ensure_main_window(&window)?;
    let install = Install::current().ok_or_else(|| {
        AppError::new(
            ErrorCode::UpdateFailed,
            "this copy of Portal can't replace itself; download the update instead",
        )
    })?;
    let state = Arc::clone(&app.state::<Arc<AppState>>());
    tauri::async_runtime::spawn_blocking(move || match install {
        Install::MacBundle(bundle) => update::install(
            &state,
            Placement {
                target: bundle.clone(),
                asset: "portal-macos-universal.app.tar.gz",
                unpack: |download: &Path, staging: &Path| {
                    update::untar(download, staging)?;
                    if !staging.join("Portal.app/Contents/MacOS").is_dir() {
                        return Err(AppError::new(
                            ErrorCode::UpdateFailed,
                            "the downloaded archive holds no Portal.app",
                        ));
                    }
                    Ok(())
                },
                commit: |_: &Path, staging: &Path| {
                    exchange(&staging.join("Portal.app"), &bundle)
                        .map_err(update::io_failed("couldn't put the new app in place"))
                },
            },
        ),
        Install::AppImage(image) => update::install(
            &state,
            Placement {
                target: image.clone(),
                asset: "portal-linux-x86_64.AppImage",
                unpack: |download: &Path, _: &Path| {
                    fs::set_permissions(download, fs::Permissions::from_mode(0o755))
                        .map_err(update::io_failed("couldn't mark the update executable"))
                },
                // A rename gives the file a new inode, so the running AppImage's mount stays
                // intact.
                commit: |download: &Path, _: &Path| {
                    fs::rename(download, &image)
                        .map_err(update::io_failed("couldn't put the new AppImage in place"))
                },
            },
        ),
    })
    .await
    .map_err(AppError::internal)??;
    super::shutdown::relaunch(app);
    Ok(())
}

/// Swaps two paths in one step, so no moment leaves the install path empty. The old bundle ends
/// up in the staging folder, which the next launch deletes; the running process keeps its files.
#[cfg(target_os = "macos")]
fn exchange(a: &Path, b: &Path) -> std::io::Result<()> {
    let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_encoded_bytes());
    let (a, b) = (c(a)?, c(b)?);
    // SAFETY: both are valid NUL-terminated paths for the duration of the call.
    if unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_SWAP) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn exchange(_: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Removes what earlier updates left beside the install.
pub fn sweep_staging() {
    if let Some(install) = Install::current() {
        update::sweep_staging(install.path());
    }
}
