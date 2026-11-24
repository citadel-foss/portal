//! In-app update from the rolling `0.1.0-beta` release. The release keeps one package version, so
//! builds are told apart by the CI build counter in `build.json`, and the download is trusted on
//! HTTPS plus the checksum that manifest lists. Only installs Portal can replace on its own — a
//! macOS `.app` or a Linux AppImage in a writable folder — update in place; the rest are pointed at
//! the release page.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use portal_core::error::{AppError, ErrorCode};
use portal_core::security::operation::{SensitiveOperation, SensitiveOperationGuard};
use portal_core::state::AppState;

const RELEASE_URL: &str = "https://github.com/citadel-foss/portal/releases/tag/0.1.0-beta";
const MANIFEST_URL: &str =
    "https://github.com/citadel-foss/portal/releases/download/0.1.0-beta/build.json";
/// Siblings of the install named this way are a download in progress or the replaced copy.
const STAGING_PREFIX: &str = ".portal-update-";

static INSTALLING: AtomicBool = AtomicBool::new(false);

#[derive(serde::Deserialize)]
struct Manifest {
    display_version: String,
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    url: String,
    sha256: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    current: &'static str,
    latest: String,
    available: bool,
    /// False when Portal cannot replace this install itself; the UI offers the release page.
    in_place: bool,
    release_url: &'static str,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

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

    fn asset(&self) -> &'static str {
        match self {
            Self::MacBundle(_) => "portal-macos-universal.app.tar.gz",
            Self::AppImage(_) => "portal-linux-x86_64.AppImage",
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
        // Renaming a directory also rewrites its `..`, so the bundle itself must be writable.
        let writable = |path: &Path| {
            let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
                return false;
            };
            // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
            unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
        };
        let path = install.path();
        (writable(path) && path.parent().is_some_and(writable)).then_some(install)
    }

    fn staging(&self) -> Option<PathBuf> {
        let parent = self.path().parent()?;
        Some(parent.join(format!("{STAGING_PREFIX}{}", std::process::id())))
    }
}

fn failed(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::UpdateFailed, message)
}

/// Per address, so one that black-holes the connection falls through to the next.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

fn agent(read_timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(read_timeout)
        .build()
}

fn get(url: &str, read_timeout: Duration) -> Result<ureq::Response, AppError> {
    agent(read_timeout).get(url).call().map_err(|e| match e {
        ureq::Error::Status(code, _) => failed(format!("GitHub answered {code}")),
        ureq::Error::Transport(e) => failed(format!("couldn't reach GitHub: {e}")),
    })
}

fn fetch_manifest() -> Result<Manifest, AppError> {
    serde_json::from_reader(get(MANIFEST_URL, Duration::from_secs(30))?.into_reader())
        .map_err(|e| failed(format!("the release's build.json is unreadable: {e}")))
}

/// Precedence ignores the `+YYYYMMDD` metadata, so only the counter (and the base version once
/// it moves on to 0.2.0) decides which build is newer.
fn is_newer(latest: &str, current: &str) -> bool {
    match (
        semver::Version::parse(latest),
        semver::Version::parse(current),
    ) {
        (Ok(latest), Ok(current)) => latest.cmp_precedence(&current).is_gt(),
        _ => false,
    }
}

#[tauri::command]
pub async fn check_update() -> Result<UpdateInfo, AppError> {
    let manifest = tauri::async_runtime::spawn_blocking(fetch_manifest)
        .await
        .map_err(|e| failed(e.to_string()))??;
    let current = crate::display_version();
    Ok(UpdateInfo {
        current,
        available: is_newer(&manifest.display_version, current),
        latest: manifest.display_version,
        in_place: Install::current().is_some(),
        release_url: RELEASE_URL,
    })
}

/// Downloads, verifies and swaps in the latest build, then quits and relaunches through the
/// normal teardown. Refused while a swap, a recovery or a router is running: the relaunch would
/// cut it off exactly as a quit would.
#[tauri::command]
pub async fn install_update(app: AppHandle, window: WebviewWindow) -> Result<(), AppError> {
    crate::native::ensure_main_window(&window)?;
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err(failed("an update is already being installed"));
    }
    let result = tauri::async_runtime::spawn_blocking({
        let app = app.clone();
        move || install(&app)
    })
    .await
    .map_err(|e| failed(e.to_string()))
    .and_then(|r| r);
    if let Err(e) = &result {
        log::error!("update failed: {}", e.message);
        INSTALLING.store(false, Ordering::SeqCst);
    }
    result
}

fn busy() -> AppError {
    AppError::new(
        ErrorCode::SensitiveOperationInProgress,
        "Updating restarts Portal. Wait for the swap or recovery to finish and stop every router first.",
    )
}

fn ensure_idle(app: &AppHandle) -> Result<(), AppError> {
    let blockers = super::shutdown::quit_blockers(&app.state::<Arc<AppState>>());
    if blockers.swap_running || blockers.recovery_running || !blockers.running_routers.is_empty() {
        return Err(busy());
    }
    Ok(())
}

fn install(app: &AppHandle) -> Result<(), AppError> {
    let install = Install::current().ok_or_else(|| {
        failed("this copy of Portal can't replace itself; download the update instead")
    })?;
    ensure_idle(app)?;
    let manifest = fetch_manifest()?;
    if !is_newer(&manifest.display_version, crate::display_version()) {
        return Err(failed("Portal is already on the latest build"));
    }
    let asset = manifest
        .assets
        .iter()
        .find(|a| a.name == install.asset())
        .ok_or_else(|| failed(format!("the release has no {}", install.asset())))?;
    let staging = install
        .staging()
        .ok_or_else(|| failed("no folder to stage the update in"))?;
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir(&staging)
        .map_err(|e| failed(format!("couldn't create {}: {e}", staging.display())))?;
    let result = stage_and_swap(app, &install, asset, &staging, &manifest.display_version);
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn stage_and_swap(
    app: &AppHandle,
    install: &Install,
    asset: &Asset,
    staging: &Path,
    version: &str,
) -> Result<(), AppError> {
    let download = staging.join(&asset.name);
    fetch_verified(asset, &download, |progress| {
        let _ = app.emit("update://progress", progress);
    })?;
    fn io(what: &'static str) -> impl Fn(std::io::Error) -> AppError {
        move |e| failed(format!("{what}: {e}"))
    }
    if let Install::MacBundle(_) = install {
        let status = std::process::Command::new("/usr/bin/tar")
            .arg("-xzf")
            .arg(&download)
            .arg("-C")
            .arg(staging)
            .status()
            .map_err(io("couldn't unpack the update"))?;
        if !status.success() || !staging.join("Portal.app/Contents/MacOS").is_dir() {
            return Err(failed("the downloaded archive holds no Portal.app"));
        }
    }
    // Held from the last idle check until the process exits, so a swap cannot start between
    // the check and the relaunch; one already being admitted holds it and refuses the update.
    let state = app.state::<Arc<AppState>>();
    let guard = SensitiveOperationGuard::acquire(
        &state.sensitive_operation_active,
        SensitiveOperation::InstallUpdate,
    )
    .map_err(|_| busy())?;
    ensure_idle(app)?;
    match install {
        Install::MacBundle(bundle) => {
            exchange(&staging.join("Portal.app"), bundle)
                .map_err(io("couldn't put the new app in place"))?;
        }
        Install::AppImage(image) => {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&download, fs::Permissions::from_mode(0o755))
                .map_err(io("couldn't mark the update executable"))?;
            // A rename gives the file a new inode, so the running AppImage's mount stays intact.
            fs::rename(&download, image).map_err(io("couldn't put the new AppImage in place"))?;
        }
    }
    std::mem::forget(guard);
    log::info!(
        "update {version} installed at {}; relaunching",
        install.path().display()
    );
    super::shutdown::relaunch(app.clone());
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

fn fetch_verified(
    asset: &Asset,
    to: &Path,
    mut progress: impl FnMut(Progress),
) -> Result<(), AppError> {
    let response = get(&asset.url, Duration::from_secs(60))?;
    let total = response
        .header("content-length")
        .and_then(|v| v.parse().ok());
    let mut response = response.into_reader();
    let mut file =
        fs::File::create(to).map_err(|e| failed(format!("couldn't save the update: {e}")))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 1 << 16];
    let mut downloaded = 0u64;
    let mut reported = 0u64;
    loop {
        let n = response
            .read(&mut buf)
            .map_err(|e| failed(format!("the download broke off: {e}")))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .map_err(|e| failed(format!("couldn't save the update: {e}")))?;
        downloaded += n as u64;
        if downloaded - reported >= 1 << 20 {
            reported = downloaded;
            progress(Progress { downloaded, total });
        }
    }
    file.sync_all()
        .map_err(|e| failed(format!("couldn't save the update: {e}")))?;
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if !digest.eq_ignore_ascii_case(&asset.sha256) {
        return Err(failed("the download doesn't match the release's checksum"));
    }
    Ok(())
}

/// Removes what earlier updates left beside the install: the replaced copy, or a download that
/// never finished.
pub fn sweep_staging() {
    let Some(dir) = Install::current().and_then(|i| i.path().parent().map(Path::to_path_buf))
    else {
        return;
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(STAGING_PREFIX)
        {
            if let Err(e) = fs::remove_dir_all(entry.path()) {
                log::warn!("couldn't remove {}: {e}", entry.path().display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn newer_is_decided_by_the_build_counter() {
        assert!(is_newer("0.1.0-beta.72+20261125", "0.1.0-beta.71+20261124"));
        assert!(is_newer(
            "0.1.0-beta.100+20261201",
            "0.1.0-beta.99+20261130"
        ));
        assert!(!is_newer(
            "0.1.0-beta.71+20261125",
            "0.1.0-beta.71+20261124"
        ));
        assert!(!is_newer(
            "0.1.0-beta.70+20261125",
            "0.1.0-beta.71+20261124"
        ));
        assert!(is_newer("0.2.0-beta.1+20270101", "0.1.0-beta.300+20261231"));
        assert!(!is_newer("garbage", "0.1.0-beta.71+20261124"));
    }
}
