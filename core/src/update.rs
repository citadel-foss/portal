//! In-app update from the rolling `0.1.0-beta` release, shared by both hosts. The release keeps
//! one package version, so builds are told apart by the CI build counter in `build.json`, and a
//! download is trusted on HTTPS plus the checksum that manifest lists. Each host decides what it
//! can replace and how; this module fetches, verifies, stages and serializes the swap.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::error::{AppError, ErrorCode};
use crate::events::AppEvent;
use crate::security::operation::{SensitiveOperation, SensitiveOperationGuard};
use crate::state::AppState;

pub const RELEASE_URL: &str = "https://github.com/citadel-foss/portal/releases/tag/0.1.0-beta";
const MANIFEST_URL: &str =
    "https://github.com/citadel-foss/portal/releases/download/0.1.0-beta/build.json";
const CHANGES_URL: &str =
    "https://github.com/citadel-foss/portal/releases/download/0.1.0-beta/changes.json";
/// Siblings of the install named this way are a download in progress or the replaced copy.
const STAGING_PREFIX: &str = ".portal-update-";
/// Per address, so one that black-holes the connection falls through to the next. minreq gives
/// the first address the whole request timeout, and one of GitHub's release-asset addresses
/// has been seen dropping connections.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

static INSTALLING: AtomicBool = AtomicBool::new(false);

/// The CI build counter (`0.1.0-beta.N+YYYYMMDD`); the package version stays fixed between
/// releases.
pub fn current_version() -> &'static str {
    include_str!("../../version.txt").trim()
}

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
    /// False when the host cannot replace this install itself; the UI offers the release page.
    in_place: bool,
    release_url: &'static str,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

fn failed(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::UpdateFailed, message)
}

/// For a host's own file steps: `what` failed, and why.
pub fn io_failed(what: &'static str) -> impl Fn(std::io::Error) -> AppError {
    move |e| failed(format!("{what}: {e}"))
}

fn busy() -> AppError {
    AppError::new(
        ErrorCode::SensitiveOperationInProgress,
        "Updating restarts Portal. Wait for the swap or recovery to finish and stop every router first.",
    )
}

fn get(url: &str, read_timeout: Duration) -> Result<ureq::Response, AppError> {
    ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(read_timeout)
        .build()
        .get(url)
        .call()
        .map_err(|e| match e {
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

/// Blocking: one request to GitHub.
pub fn check(in_place: bool) -> Result<UpdateInfo, AppError> {
    let manifest = fetch_manifest()?;
    let current = current_version();
    Ok(UpdateInfo {
        current,
        available: is_newer(&manifest.display_version, current),
        latest: manifest.display_version,
        in_place,
        release_url: RELEASE_URL,
    })
}

/// One build's entry in the release's `changes.json`, which CI extends on every publish.
#[derive(serde::Deserialize)]
struct Build {
    version: String,
    /// The build published before this one; its changes are everything merged since.
    since: String,
    portal: Vec<Merged>,
    openswap: Vec<Merged>,
}

#[derive(serde::Deserialize)]
struct Merged {
    number: u64,
    title: String,
}

/// A pull request merged after this build.
#[derive(serde::Serialize)]
pub struct Change {
    number: u64,
    title: String,
    url: String,
}

#[derive(serde::Serialize)]
pub struct Changes {
    portal: Vec<Change>,
    openswap: Vec<Change>,
    /// False when the release's history starts after this build.
    complete: bool,
}

/// Blocking: one request to GitHub.
pub fn changes() -> Result<Changes, AppError> {
    #[derive(serde::Deserialize)]
    struct Log {
        builds: Vec<Build>,
    }
    let log: Log = get(CHANGES_URL, Duration::from_secs(30))
        .and_then(|response| {
            serde_json::from_reader(response.into_reader())
                .map_err(|e| failed(format!("changes.json is unreadable: {e}")))
        })
        .map_err(|e| failed(format!("Couldn't load what changed: {}.", e.message)))?;
    Ok(changes_since(log.builds, current_version()))
}

/// Newest first, each pull request once.
fn changes_since(mut builds: Vec<Build>, current: &str) -> Changes {
    builds.retain(|build| is_newer(&build.version, current));
    builds.sort_by(|a, b| {
        match (
            semver::Version::parse(&a.version),
            semver::Version::parse(&b.version),
        ) {
            (Ok(a), Ok(b)) => b.cmp_precedence(&a),
            _ => std::cmp::Ordering::Equal,
        }
    });
    // Each build names the one before it, so the history reaches back to this build once one
    // of them was published on top of it or of something older.
    let complete = builds.iter().any(|build| !is_newer(&build.since, current));
    let mut changes = Changes {
        portal: Vec::new(),
        openswap: Vec::new(),
        complete,
    };
    for build in builds {
        for (repo, merged, into) in [
            ("portal", build.portal, &mut changes.portal),
            ("openswap", build.openswap, &mut changes.openswap),
        ] {
            for Merged { number, title } in merged {
                if !into.iter().any(|change| change.number == number) {
                    into.push(Change {
                        number,
                        title,
                        url: format!("https://github.com/citadel-foss/{repo}/pull/{number}"),
                    });
                }
            }
        }
    }
    changes
}

/// Whether this process may replace `path` by renaming over it: a directory's rename also
/// rewrites its `..`, so the path itself must be writable as well as its parent.
pub fn replaceable(path: &Path) -> bool {
    let writable = |path: &Path| {
        let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
            return false;
        };
        // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
        unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
    };
    writable(path) && path.parent().is_some_and(writable)
}

/// What a host does with a verified download. `unpack` runs before the swap is admitted and
/// may take its time; `commit` runs while no swap can start and must be one quick rename.
pub struct Placement<U, C> {
    /// The installed file or bundle being replaced. Staging happens beside it, so the final
    /// rename stays on one filesystem.
    pub target: PathBuf,
    /// The release asset this platform installs from.
    pub asset: &'static str,
    /// Given the download and the staging folder.
    pub unpack: U,
    /// Given the download and the staging folder.
    pub commit: C,
}

/// Downloads, verifies and puts the latest build in place, returning its version. Refused while
/// a swap, a recovery or a router is running: the host restarts next, which would cut it off
/// exactly as a quit would. On success the sensitive-operation guard stays held until the
/// process exits, so no swap can start between the last check and the restart.
///
/// Blocking: downloads tens of megabytes.
pub fn install<U, C>(state: &AppState, placement: Placement<U, C>) -> Result<String, AppError>
where
    U: FnOnce(&Path, &Path) -> Result<(), AppError>,
    C: FnOnce(&Path, &Path) -> Result<(), AppError>,
{
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err(failed("an update is already being installed"));
    }
    let result = install_once(state, placement);
    if let Err(e) = &result {
        log::error!("update failed: {}", e.message);
        INSTALLING.store(false, Ordering::SeqCst);
    }
    result
}

fn install_once<U, C>(state: &AppState, placement: Placement<U, C>) -> Result<String, AppError>
where
    U: FnOnce(&Path, &Path) -> Result<(), AppError>,
    C: FnOnce(&Path, &Path) -> Result<(), AppError>,
{
    ensure_idle(state)?;
    let manifest = fetch_manifest()?;
    if !is_newer(&manifest.display_version, current_version()) {
        return Err(failed("Portal is already on the latest build"));
    }
    let asset = manifest
        .assets
        .iter()
        .find(|a| a.name == placement.asset)
        .ok_or_else(|| failed(format!("the release has no {}", placement.asset)))?;
    let staging = placement
        .target
        .parent()
        .map(|dir| dir.join(format!("{STAGING_PREFIX}{}", std::process::id())))
        .ok_or_else(|| failed("no folder to stage the update in"))?;
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir(&staging)
        .map_err(|e| failed(format!("couldn't create {}: {e}", staging.display())))?;
    let result = (|| -> Result<(), AppError> {
        let download = staging.join(&asset.name);
        fetch_verified(asset, &download, |progress| {
            state.events.publish(AppEvent::UpdateProgress(progress))
        })?;
        (placement.unpack)(&download, &staging)?;
        // Again after the download, waiting out a sync if need be: a swap or router can start
        // during it. Before the guard, so a slow wallet read holds up nobody else.
        ensure_idle(state)?;
        let guard = SensitiveOperationGuard::acquire(
            &state.sensitive_operation_active,
            SensitiveOperation::InstallUpdate,
        )
        .map_err(|_| busy())?;
        if !state.close_for_restart()? {
            return Err(busy());
        }
        // With the guard held and opens closed, nothing may wait here: a wallet mid-sync is busy.
        let committed = if state.idle_now() {
            (placement.commit)(&download, &staging)
        } else {
            Err(busy())
        };
        if committed.is_err() {
            state.reopen_after_restart_refused();
        }
        committed?;
        std::mem::forget(guard);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result?;
    log::info!(
        "update {} installed at {}; restarting",
        manifest.display_version,
        placement.target.display()
    );
    Ok(manifest.display_version)
}

fn ensure_idle(state: &AppState) -> Result<(), AppError> {
    let blockers = state.running_work();
    if blockers.swap_running || blockers.recovery_running || !blockers.running_routers.is_empty() {
        return Err(busy());
    }
    Ok(())
}

fn fetch_verified(
    asset: &Asset,
    to: &Path,
    progress: impl FnMut(UpdateProgress),
) -> Result<(), AppError> {
    let response = get(&asset.url, Duration::from_secs(60))?;
    let total = response
        .header("content-length")
        .and_then(|v| v.parse().ok());
    save_verified(response.into_reader(), total, to, &asset.sha256, progress)
}

/// Streams `source` into `to`, then fails unless its SHA-256 is `expected`.
fn save_verified(
    mut source: impl Read,
    total: Option<u64>,
    to: &Path,
    expected: &str,
    mut progress: impl FnMut(UpdateProgress),
) -> Result<(), AppError> {
    let save = io_failed("couldn't save the update");
    let mut file = fs::File::create(to).map_err(&save)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 1 << 16];
    let mut downloaded = 0u64;
    let mut reported = 0u64;
    loop {
        let n = source
            .read(&mut buf)
            .map_err(io_failed("the download broke off"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(&save)?;
        downloaded += n as u64;
        if downloaded - reported >= 1 << 20 {
            reported = downloaded;
            progress(UpdateProgress { downloaded, total });
        }
    }
    file.sync_all().map_err(&save)?;
    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if !digest.eq_ignore_ascii_case(expected) {
        return Err(failed("the download doesn't match the release's checksum"));
    }
    Ok(())
}

/// Unpacks a release `.tar.gz` into `into` with the system `tar`, which keeps a bundle's
/// symlinks and modes intact.
pub fn untar(archive: &Path, into: &Path) -> Result<(), AppError> {
    let status = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .status()
        .map_err(io_failed("couldn't unpack the update"))?;
    if !status.success() {
        return Err(failed("couldn't unpack the update"));
    }
    Ok(())
}

/// Removes what earlier updates left beside `target`: the replaced copy, or a download that
/// never finished.
pub fn sweep_staging(target: &Path) {
    let Some(entries) = target.parent().and_then(|dir| fs::read_dir(dir).ok()) else {
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
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("portal-update-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        dir
    }

    /// SHA-256 of "portal".
    const PORTAL_SHA256: &str = "d0960501f8971be812f2e5494426e08cdbb2cbc3b3190ba60075f14b8da7178a";

    #[test]
    fn a_download_matching_its_checksum_is_kept() {
        let dir = scratch();
        let to = dir.join("asset");
        let mut reports = 0;
        save_verified(&b"portal"[..], Some(6), &to, PORTAL_SHA256, |_| {
            reports += 1
        })
        .unwrap();
        assert_eq!(fs::read(&to).unwrap(), b"portal");
        // Upper case too: the manifest's hex is compared case-insensitively.
        save_verified(
            &b"portal"[..],
            None,
            &to,
            &PORTAL_SHA256.to_uppercase(),
            |_| {},
        )
        .unwrap();
        assert_eq!(
            reports, 0,
            "progress is reported per MiB, not per small read"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_download_not_matching_its_checksum_fails() {
        let dir = scratch();
        let error = save_verified(
            &b"tampered"[..],
            None,
            &dir.join("asset"),
            PORTAL_SHA256,
            |_| {},
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpdateFailed);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sweeping_removes_only_staging_folders() {
        let dir = scratch();
        let target = dir.join("portal-server");
        fs::write(&target, b"binary").unwrap();
        fs::create_dir(dir.join(format!("{STAGING_PREFIX}123"))).unwrap();
        fs::write(dir.join(format!("{STAGING_PREFIX}123/replaced")), b"old").unwrap();
        fs::create_dir(dir.join("portal-update-notes")).unwrap();
        fs::write(dir.join("notes.txt"), b"keep").unwrap();
        sweep_staging(&target);
        let mut left: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, ["notes.txt", "portal-server", "portal-update-notes"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn changes_cover_every_build_after_this_one() {
        let build = |n: u32, since: u32, portal: &[u64], openswap: &[u64]| Build {
            version: format!("0.1.0-beta.{n}+20261125"),
            since: format!("0.1.0-beta.{since}+20261124"),
            portal: portal
                .iter()
                .map(|&number| Merged { number, title: format!("pr {number}") })
                .collect(),
            openswap: openswap
                .iter()
                .map(|&number| Merged { number, title: format!("pr {number}") })
                .collect(),
        };
        let numbers = |changes: &[Change]| changes.iter().map(|c| c.number).collect::<Vec<_>>();
        let log = || {
            vec![
                build(71, 70, &[95], &[]),
                build(73, 72, &[97], &[1242]),
                build(72, 71, &[96], &[1234, 1242]),
            ]
        };

        let changes = changes_since(log(), "0.1.0-beta.71+20261124");
        assert_eq!(numbers(&changes.portal), [97, 96]);
        assert_eq!(numbers(&changes.openswap), [1242, 1234]);
        assert_eq!(
            changes.openswap[1].url,
            "https://github.com/citadel-foss/openswap/pull/1234"
        );
        assert!(changes.complete);

        let changes = changes_since(log(), "0.1.0-beta.60+20261101");
        assert_eq!(numbers(&changes.portal), [97, 96, 95]);
        assert!(!changes.complete, "history starts at 70, after 60");
    }

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
