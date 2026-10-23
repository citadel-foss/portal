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
/// offerbook and config per data dir, and Portal keeps the wallet's swap journal there.
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

/// A wallet's chain, kept beside it because the wallet file that knows it is encrypted. Written
/// on the first successful open: the crate refuses a wallet on any other chain, so that chain is
/// the wallet's. Routers don't need it: openswap records theirs in `config.toml`.
const NETWORK_FILE: &str = "network";

pub fn record_network(dir: &Path, chain: &str) -> Result<(), AppError> {
    crate::security::fs::write_private(&dir.join(NETWORK_FILE), chain.as_bytes())
}

pub fn recorded_network(dir: &Path) -> Option<String> {
    fs::read_to_string(dir.join(NETWORK_FILE)).ok().map(|chain| chain.trim().to_string())
}

/// Deletes a swap tracker openswap stopped reading at #1137. Logged when it fails, because the
/// file holds swap secrets in the clear.
pub fn remove_legacy_tracker(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => log::info!("removed {}", path.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("could not remove {}: {e}", path.display()),
    }
}

/// The chain of a wallet or router, readable while it is locked: the one openswap records in a
/// started router's `config.toml`, else the one Portal recorded on a wallet's first open. One
/// never opened (a Maker Dashboard import, a `makerd` dir) falls back to an address Portal issued
/// from it: the prefix tells mainnet from the test networks (`"test"`), though not one test
/// network from another.
pub fn wallet_network(dir: &Path) -> Option<String> {
    if let Some(chain) = configured_network(dir).or_else(|| recorded_network(dir)) {
        return Some(chain);
    }
    let issued = fs::read_to_string(dir.join("last_address.json")).ok()?;
    let issued: serde_json::Value = serde_json::from_str(&issued).ok()?;
    let address = ["p2tr", "p2wpkh"].iter().find_map(|key| issued.get(key)?.as_str())?;
    let network = if address.starts_with("bcrt1") {
        "regtest"
    } else if address.starts_with("bc1") {
        "bitcoin"
    } else if address.starts_with("tb1") {
        "test"
    } else {
        return None;
    };
    Some(network.to_string())
}

/// `network = "signet"`, which `MakerServer::init` writes from the wallet itself.
fn configured_network(dir: &Path) -> Option<String> {
    fs::read_to_string(dir.join("config.toml"))
        .ok()?
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            let value = value.split('#').next().unwrap_or_default();
            (key.trim() == "network").then(|| value.trim().trim_matches('"').to_string())
        })
        .filter(|chain| !chain.is_empty())
}

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

/// Present from a new wallet's creation until its phrase is confirmed saved. The phrase lives in
/// memory only, so past a restart or the end of its session this file is all that says it was
/// never saved: nothing has been allowed to fund the wallet while it is here. Named for the one
/// wallet it covers, since a router's data dir can hold wallets of other names.
pub fn unsaved_phrase_marker(data_dir: &Path, wallet_name: &str) -> PathBuf {
    data_dir.join(format!(".phrase-unsaved-{wallet_name}"))
}

pub fn clear_unsaved_phrase_marker(marker: &Path) -> Result<(), AppError> {
    match fs::remove_file(marker) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Removes a wallet whose phrase was never saved and is no longer held, so it is created again
/// with a phrase the user sees. It cannot hold funds: no address is issued and no router starts
/// while its marker stands. Callers hold the dir lock and have checked no phrase is held for it.
pub(crate) fn discard_unsaved_wallet(
    data_dir: &Path,
    wallet_name: &str,
) -> Result<bool, AppError> {
    let (wallet, marker) = (
        wallet_path(data_dir, wallet_name),
        unsaved_phrase_marker(data_dir, wallet_name),
    );
    if !(wallet.exists() && marker.exists()) {
        return Ok(false);
    }
    fs::remove_file(&wallet)?;
    clear_unsaved_phrase_marker(&marker)?;
    log::info!("removed {wallet:?}: created, but its recovery phrase was never saved");
    Ok(true)
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

    /// openswap's own record wins over a stale one Portal kept for a router.
    #[test]
    fn a_router_is_placed_by_its_config() {
        let dir = scratch("configured");
        fs::create_dir_all(dir.join("wallets")).unwrap();
        assert_eq!(wallet_network(&dir), None);
        record_network(&dir, "bitcoin").unwrap();
        assert_eq!(wallet_network(&dir).as_deref(), Some("bitcoin"));
        fs::write(
            dir.join("config.toml"),
            "network_port = 6102\nnetwork = \"signet\" # set by hand\nrpc_port = 6103\n",
        )
        .unwrap();
        assert_eq!(wallet_network(&dir).as_deref(), Some("signet"));
        fs::remove_dir_all(&dir).unwrap();
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

    /// The only automatic wallet removal there is: it must never reach a wallet without its own
    /// marker, including another wallet's in the same data dir.
    #[test]
    fn only_a_marked_wallet_is_discarded() {
        let dir = scratch("unsaved");
        fs::create_dir_all(dir.join("wallets")).unwrap();
        let (wallet, other) = (wallet_path(&dir, "w"), wallet_path(&dir, "other"));
        fs::write(&wallet, b"x").unwrap();
        fs::write(&other, b"x").unwrap();

        assert!(!discard_unsaved_wallet(&dir, "w").unwrap());
        assert!(wallet.exists(), "a wallet with no marker is never touched");

        fs::write(unsaved_phrase_marker(&dir, "other"), b"").unwrap();
        assert!(!discard_unsaved_wallet(&dir, "w").unwrap());
        assert!(wallet.exists(), "another wallet's marker never removes this one");

        fs::write(unsaved_phrase_marker(&dir, "w"), b"").unwrap();
        assert!(discard_unsaved_wallet(&dir, "w").unwrap());
        assert!(!wallet.exists() && !unsaved_phrase_marker(&dir, "w").exists());
        assert!(other.exists() && unsaved_phrase_marker(&dir, "other").exists());

        fs::write(unsaved_phrase_marker(&dir, "w"), b"").unwrap();
        assert!(
            !discard_unsaved_wallet(&dir, "w").unwrap(),
            "a stale marker alone removes nothing"
        );
        clear_unsaved_phrase_marker(&unsaved_phrase_marker(&dir, "w")).unwrap();
        clear_unsaved_phrase_marker(&unsaved_phrase_marker(&dir, "w")).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }
}
