//! In-app updates. Every build on `main` replaces the files on the rolling `0.1.0-beta` release
//! and describes them in its `build.json`, so the build counter there (`display_version`) is
//! what says whether this build is behind — the app's own version stays the same between
//! releases.
//!
//! Only the macOS app and the Linux AppImage can replace themselves: they are one bundle or one
//! file the user owns. A `.deb`/`.rpm` install belongs to the package manager.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use openswap::bitcoin::hashes::{sha256, Hash, HashEngine};
use semver::Version;
use tauri::{AppHandle, Emitter, Manager};

use portal_core::error::{AppError, ErrorCode};
use portal_core::state::AppState;

const MANIFEST_URL: &str =
    "https://github.com/citadel-foss/portal/releases/download/0.1.0-beta/build.json";
const RELEASE_PAGE: &str = "https://github.com/citadel-foss/portal/releases/tag/0.1.0-beta";
// A total deadline, not an idle one: minreq has no other kind.
const DOWNLOAD_TIMEOUT_SECS: u64 = 30 * 60;

static INSTALLING: AtomicBool = AtomicBool::new(false);

#[derive(serde::Deserialize)]
struct Manifest {
    display_version: String,
    built_at: String,
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    sha256: String,
    url: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    version: String,
    built_at: String,
    /// False when this install cannot replace itself; the user downloads from `release_page`.
    installable: bool,
    release_page: &'static str,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    downloaded: u64,
    total: Option<u64>,
}

/// What is running, and the release file that replaces it.
fn current_install() -> Option<(PathBuf, &'static str)> {
    #[cfg(target_os = "macos")]
    {
        // `Portal.app/Contents/MacOS/portal`
        let exe = std::env::current_exe().ok()?;
        let app = exe.ancestors().nth(3)?;
        (app.extension()? == "app").then(|| (app.to_path_buf(), "portal-macos-universal.app.tar.gz"))
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        // Set by the AppImage runtime to the file it was launched from; absent for a .deb/.rpm.
        std::env::var_os("APPIMAGE").map(|path| (PathBuf::from(path), "portal-linux-x86_64.AppImage"))
    }
    #[cfg(not(any(target_os = "macos", all(target_os = "linux", target_arch = "x86_64"))))]
    None
}

/// The new copy is staged beside the old one and renamed over it, which needs the folder to be
/// writable. It is not when the app runs from the mounted `.dmg` or from macOS's App
/// Translocation copy, both read-only.
fn writable_dir(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(dir) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    unsafe { libc::access(dir.as_ptr(), libc::W_OK) == 0 }
}

fn fetch_manifest() -> Result<Manifest, AppError> {
    let response = minreq::get(MANIFEST_URL)
        .with_timeout(20)
        .send()
        .map_err(|e| AppError::new(ErrorCode::Internal, format!("could not reach GitHub: {e}")))?;
    if response.status_code != 200 {
        return Err(AppError::new(
            ErrorCode::Internal,
            format!("GitHub returned HTTP {} for the build list", response.status_code),
        ));
    }
    response.json().map_err(AppError::internal)
}

fn newer(latest: &str, running: &str) -> bool {
    // Precedence ignores the `+date` build metadata, so the counter alone decides within a
    // release, and a new release (0.2.0-beta.1) outranks any build of the last one.
    match (Version::parse(latest), Version::parse(running)) {
        (Ok(latest), Ok(running)) => latest.cmp_precedence(&running).is_gt(),
        _ => false,
    }
}

#[tauri::command]
pub async fn check_for_update() -> Result<Option<AvailableUpdate>, AppError> {
    // A dev build's `version.txt` is whatever was last checked in, and there is no bundle to
    // replace.
    if cfg!(debug_assertions) {
        return Ok(None);
    }
    let manifest = tauri::async_runtime::spawn_blocking(fetch_manifest)
        .await
        .map_err(AppError::internal)??;
    if !newer(&manifest.display_version, crate::build_version()) {
        return Ok(None);
    }
    let installable = current_install().is_some_and(|(path, asset)| {
        path.parent().is_some_and(writable_dir) && manifest.assets.iter().any(|a| a.name == asset)
    });
    Ok(Some(AvailableUpdate {
        version: manifest.display_version,
        built_at: manifest.built_at,
        installable,
        release_page: RELEASE_PAGE,
    }))
}

/// Downloads the newer build over this one, then quits through the normal teardown and starts
/// the new copy once this process is gone. Refused while anything a quit would interrupt is
/// running, rather than asking: a restart the user did not need is never worth a swap.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), AppError> {
    let blockers = crate::commands::shutdown::quit_blockers(&app.state::<Arc<AppState>>());
    if blockers.swap_running || blockers.recovery_running || !blockers.running_routers.is_empty() {
        return Err(AppError::new(
            ErrorCode::SensitiveOperationInProgress,
            "Finish the running swap, recovery or router first: updating restarts Portal.",
        ));
    }
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err(AppError::new(
            ErrorCode::SensitiveOperationInProgress,
            "an update is already downloading",
        ));
    }
    let staged = {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            stage(|progress| {
                let _ = app.emit("update://progress", progress);
            })
        })
        .await
    };
    let relaunch = match staged {
        Ok(Ok(relaunch)) => relaunch,
        Ok(Err(e)) => {
            INSTALLING.store(false, Ordering::SeqCst);
            return Err(e);
        }
        Err(e) => {
            INSTALLING.store(false, Ordering::SeqCst);
            return Err(AppError::internal(e));
        }
    };
    relaunch_after_exit(relaunch)?;
    crate::commands::shutdown::run_quit(app);
    Ok(())
}

/// What to start once this process has exited, and the staging folder to clear first.
struct Relaunch {
    target: PathBuf,
    staging: PathBuf,
}

fn stage(on_progress: impl Fn(DownloadProgress)) -> Result<Relaunch, AppError> {
    let (target, asset_name) = current_install().ok_or_else(|| {
        AppError::new(ErrorCode::InvalidInput, "this install cannot update itself")
    })?;
    let parent = target
        .parent()
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "install has no parent folder"))?;
    let manifest = fetch_manifest()?;
    if !newer(&manifest.display_version, crate::build_version()) {
        return Err(AppError::new(ErrorCode::InvalidInput, "Portal is already up to date"));
    }
    let asset = manifest
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| {
            AppError::new(ErrorCode::Internal, format!("{asset_name} is missing from the release"))
        })?;

    // Beside the install, so the final step is a rename on one filesystem. Named per process so
    // a leftover from a failed attempt never collides with this one.
    let staging = parent.join(format!(".portal-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir(&staging)?;
    let result = download(asset, &staging.join(asset_name), on_progress)
        .and_then(|file| swap_in(&file, &target, &staging));
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result?;
    log::info!("update {} staged over {}", manifest.display_version, target.display());
    Ok(Relaunch { target, staging })
}

fn download(
    asset: &Asset,
    dest: &Path,
    on_progress: impl Fn(DownloadProgress),
) -> Result<PathBuf, AppError> {
    let failed = |e: &dyn std::fmt::Display| {
        AppError::new(ErrorCode::Internal, format!("download failed: {e}"))
    };
    let mut response = minreq::get(&asset.url)
        .with_timeout(DOWNLOAD_TIMEOUT_SECS)
        .send_lazy()
        .map_err(|e| failed(&e))?;
    if response.status_code != 200 {
        return Err(failed(&format!("HTTP {}", response.status_code)));
    }
    let total = response
        .headers
        .get("content-length")
        .and_then(|len| len.parse::<u64>().ok());
    let mut file = std::fs::File::create(dest)?;
    let mut engine = sha256::HashEngine::default();
    let mut buf = vec![0u8; 256 * 1024];
    let (mut downloaded, mut reported) = (0u64, 0u64);
    loop {
        let n = response.read(&mut buf).map_err(|e| failed(&e))?;
        if n == 0 {
            break;
        }
        engine.input(&buf[..n]);
        file.write_all(&buf[..n])?;
        downloaded += n as u64;
        if downloaded - reported >= 1 << 20 {
            reported = downloaded;
            on_progress(DownloadProgress { downloaded, total });
        }
    }
    file.sync_all()?;
    // Catches a truncated or corrupted download; it is not a signature, and says nothing about
    // who published the file.
    if sha256::Hash::from_engine(engine).to_string() != asset.sha256 {
        return Err(failed(&"the file did not match its checksum"));
    }
    Ok(dest.to_path_buf())
}

#[cfg(target_os = "macos")]
fn swap_in(archive: &Path, target: &Path, staging: &Path) -> Result<(), AppError> {
    // The system tar, because the archive carries the bundle's symlinks and executable bits.
    let status = std::process::Command::new("/usr/bin/tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(staging)
        .status()?;
    let fresh = staging.join("Portal.app");
    if !status.success() || !fresh.is_dir() {
        return Err(AppError::new(ErrorCode::Internal, "could not unpack the update"));
    }
    // Moved aside rather than deleted: this process is still running from it until it quits.
    let old = staging.join("old.app");
    std::fs::rename(target, &old)?;
    if let Err(e) = std::fs::rename(&fresh, target) {
        let _ = std::fs::rename(&old, target);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn swap_in(file: &Path, target: &Path, _staging: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755))?;
    // The running AppImage stays mounted from the old file's inode, so replacing the path is safe.
    std::fs::rename(file, target)?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn swap_in(_: &Path, _: &Path, _: &Path) -> Result<(), AppError> {
    Err(AppError::new(ErrorCode::InvalidInput, "this install cannot update itself"))
}

/// The new copy can only start once this one has exited: the single-instance guard would
/// otherwise hand its launch straight back to this process. A detached shell waits for that.
fn relaunch_after_exit(relaunch: Relaunch) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    const START: &str = r#"open "$2""#;
    #[cfg(not(target_os = "macos"))]
    const START: &str = r#""$2" >/dev/null 2>&1 &"#;
    let script = format!(
        r#"while kill -0 "$1" 2>/dev/null; do sleep 0.5; done; rm -rf "$3"; {START}"#
    );
    let mut command = std::process::Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(script)
        .arg("sh")
        .arg(std::process::id().to_string())
        .arg(&relaunch.target)
        .arg(&relaunch.staging)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Set by this AppImage's runtime and pointing into its mount, which is gone by the time the
    // new one starts; the new runtime sets its own.
    for var in ["APPIMAGE", "APPDIR", "ARGV0", "OWD"] {
        command.env_remove(var);
    }
    command.spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn build_counter_orders_updates() {
        assert!(newer("0.1.0-beta.36+20261017", "0.1.0-beta.35+20261016"));
        // Numeric, not lexical.
        assert!(newer("0.1.0-beta.100+20261201", "0.1.0-beta.99+20261130"));
        assert!(newer("0.2.0-beta.1+20261231", "0.1.0-beta.140+20261230"));
        // The same build rebuilt on another day is not an update.
        assert!(!newer("0.1.0-beta.35+20261017", "0.1.0-beta.35+20261016"));
        assert!(!newer("0.1.0-beta.34+20261015", "0.1.0-beta.35+20261016"));
        assert!(!newer("garbage", "0.1.0-beta.35+20261016"));
    }
}
