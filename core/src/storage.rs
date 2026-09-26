//! Data-root resolution and the wallet directory listing.
//!
//! Paths are resolved here rather than by a host: the web host must never accept a
//! client-supplied filesystem path, and the desktop host's selected directory has to end up
//! under the same rules as the default one.

use std::fs;
use std::path::{Path, PathBuf};

use openswap::utill::get_taker_dir;

use crate::error::{AppError, ErrorCode};

/// `~/.openswap`: `takers/` and `makers/` hold one folder per wallet and per router, and
/// everything that belongs to the app rather than to one of them lives here directly.
pub fn openswap_root() -> Result<PathBuf, AppError> {
    let taker = get_taker_dir()?;
    Ok(taker.parent().map(Path::to_path_buf).unwrap_or(taker))
}

/// Resolves an explicitly chosen root, falling back to `~/.openswap`.
pub fn resolve_data_dir(data_dir: &Option<String>) -> Result<PathBuf, AppError> {
    match data_dir {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => openswap_root(),
    }
}

/// The crate `data_dir` for one wallet. One per wallet, never shared: the crate keeps a single
/// swap tracker and offerbook per data dir, and every `Taker::init` fails the unfinished swaps
/// it finds in that tracker, whichever wallet they belong to.
pub fn wallet_data_dir(root: &Path, wallet_name: &str) -> PathBuf {
    root.join(WALLET_DATA_DIR).join(wallet_name)
}

/// A router's default data dir, `~/.openswap/makers/<id>`.
pub fn maker_data_dir(router_id: &str) -> Result<PathBuf, AppError> {
    Ok(openswap_root()?.join("makers").join(router_id))
}

/// The router registry.
pub fn makers_registry() -> Result<PathBuf, AppError> {
    Ok(openswap_root()?.join("makers.json"))
}

pub fn wallet_path(data_dir: &Path, wallet_name: &str) -> PathBuf {
    data_dir.join("wallets").join(wallet_name)
}

/// Where the per-wallet data dirs live under a root.
pub const WALLET_DATA_DIR: &str = "takers";

/// Wallets under `<root>/takers`, sorted: each is a directory holding its own
/// `wallets/<same name>`.
pub fn list_wallets(data_dir: &Option<String>) -> Result<Vec<String>, AppError> {
    let dir = resolve_data_dir(data_dir)?.join(WALLET_DATA_DIR);
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            if wallet_path(&entry.path(), name).is_file() {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Writes bytes a host received into a private file under the managed root, returning the
/// path. The name is chosen here, never by the client: a filename that crossed the network is
/// attacker-controlled, and honouring it is how uploads become path traversal.
pub fn stage_private_file(root: &Path, bytes: &[u8]) -> Result<PathBuf, AppError> {
    let dir = root.join("portal").join("transfers");
    crate::security::fs::ensure_private_dir(&dir)?;
    let path = dir.join(format!("{}.upload", uuid::Uuid::new_v4()));
    crate::security::fs::write_private(&path, bytes)?;
    Ok(path)
}

/// Drops staging files older than `max_age`. Covers the cases a success path never sees:
/// an abandoned upload, an expired capability, a restart between transfer and restore.
pub fn sweep_stale_transfers(root: &Path, max_age: std::time::Duration) {
    let dir = root.join("portal").join("transfers");
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().map(|age| age > max_age).unwrap_or(false))
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Exclusive hold on one wallet's data dir across processes, released when dropped.
///
/// Inside one process the taker registry already refuses to open a wallet twice. This covers
/// the desktop app and a web server run from the same home, which are separate processes over
/// the same files and would otherwise each load the wallet and overwrite the other's saves.
#[derive(Debug)]
pub struct WalletDirLock {
    _file: fs::File,
}

#[cfg(unix)]
pub fn lock_wallet_dir(dir: &Path) -> Result<WalletDirLock, AppError> {
    use std::os::unix::io::AsRawFd;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(".portal-open.lock"))?;
    // Non-blocking: the other process may hold it for hours, and waiting would just hang.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(AppError::new(
            ErrorCode::WalletOpenElsewhere,
            "This wallet is already open in another Portal on this machine. Close it there first.",
        ));
    }
    Ok(WalletDirLock { _file: file })
}

/// Windows has no `flock`; the equivalent is opening the file without share permissions,
/// which needs its own testing before it guards a wallet. Until then the lock is held in name
/// only there, rather than pretending to a guarantee it does not give.
#[cfg(not(unix))]
pub fn lock_wallet_dir(dir: &Path) -> Result<WalletDirLock, AppError> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(".portal-open.lock"))?;
    Ok(WalletDirLock { _file: file })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("portal-storage-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn make_wallet(root: &Path, name: &str) {
        let dir = wallet_data_dir(root, name).join("wallets");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(name), b"x").unwrap();
    }

    #[test]
    fn lists_one_wallet_per_data_dir() {
        let root = scratch("list");
        make_wallet(&root, "bob");
        make_wallet(&root, "alice");
        // A dir without its wallet file is not a wallet.
        fs::create_dir_all(wallet_data_dir(&root, "empty").join("wallets")).unwrap();

        let found = list_wallets(&Some(root.display().to_string())).unwrap();
        assert_eq!(found, vec!["alice".to_string(), "bob".to_string()]);
        fs::remove_dir_all(&root).unwrap();
    }

    /// The old shared layout is not read at all: its tracker holds several wallets' swaps, which
    /// is exactly what a per-wallet Taker must never load.
    #[test]
    fn the_old_shared_layout_is_ignored() {
        let root = scratch("legacy");
        fs::create_dir_all(root.join("wallets")).unwrap();
        fs::write(root.join("wallets").join("old"), b"x").unwrap();
        assert!(list_wallets(&Some(root.display().to_string())).unwrap().is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_missing_root_is_empty_rather_than_an_error() {
        let root = scratch("absent");
        assert!(list_wallets(&Some(root.display().to_string())).unwrap().is_empty());
    }

    /// flock is per open file description, so a second open in the same process contends
    /// exactly as another process would.
    #[cfg(unix)]
    #[test]
    fn a_wallet_dir_cannot_be_locked_twice() {
        let root = scratch("lock");
        fs::create_dir_all(&root).unwrap();
        let held = lock_wallet_dir(&root).unwrap();
        let second = lock_wallet_dir(&root).unwrap_err();
        assert_eq!(second.code, ErrorCode::WalletOpenElsewhere);
        drop(held);
        assert!(lock_wallet_dir(&root).is_ok(), "released on drop");
        fs::remove_dir_all(&root).unwrap();
    }
}
