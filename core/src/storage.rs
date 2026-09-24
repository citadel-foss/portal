//! Data-root resolution and the wallet directory listing.
//!
//! Paths are resolved here rather than by a host: the web host must never accept a
//! client-supplied filesystem path, and the desktop host's selected directory has to end up
//! under the same rules as the default one.

use std::fs;
use std::path::{Path, PathBuf};

use openswap::utill::get_taker_dir;

use crate::error::AppError;

/// Non-wallet files the crate and this app write into the wallets directory (crate
/// report/lock/temp, plus our own last-issued-address sidecar).
const NON_WALLET_SUFFIXES: &[&str] = &[
    "_swap_report.json",
    "_last_address.json",
    ".lock",
    ".partial",
    ".tmp",
];

/// Resolves an explicitly chosen root, falling back to the crate's own taker directory.
pub fn resolve_data_dir(data_dir: &Option<String>) -> Result<PathBuf, AppError> {
    match data_dir {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Ok(get_taker_dir()?),
    }
}

pub fn wallet_path(data_dir: &Path, wallet_name: &str) -> PathBuf {
    data_dir.join("wallets").join(wallet_name)
}

/// Wallet files in `<data_dir>/wallets`, sorted, with the crate's sidecar files filtered out.
pub fn list_wallets(data_dir: &Option<String>) -> Result<Vec<String>, AppError> {
    let dir = resolve_data_dir(data_dir)?.join("wallets");
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            if let Some(name) = entry.file_name().to_str() {
                if !NON_WALLET_SUFFIXES.iter().any(|suf| name.ends_with(suf)) {
                    names.push(name.to_string());
                }
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

/// Upstream renamed its data dir from `~/.coinswap` to `~/.openswap` (PR #988) without shipping
/// a migration, so an updated build starts against an empty directory and every existing wallet,
/// swap tracker and Tor identity looks lost. Copied rather than moved, so an older build still
/// finds its own data.
///
/// The new root existing is not proof the move already happened — Tor writes `tor-manager/` under
/// it the moment the app launches, and a build run before this migration existed leaves one
/// behind — so the marker records that instead, and only entries with nothing in their way are
/// filled in.
pub fn migrate_legacy_data_dir() {
    let Ok(taker_dir) = openswap::utill::get_taker_dir() else {
        return;
    };
    let Some(root) = taker_dir.parent() else {
        return;
    };
    let Some(legacy_root) = root.parent().map(|home| home.join(".coinswap")) else {
        return;
    };
    migrate_from_legacy_root(&legacy_root, root);
}

/// Split from the path derivation above so it can be driven against temporary roots — the real
/// ones resolve through the crate to the user's home directory.
fn migrate_from_legacy_root(legacy_root: &Path, root: &Path) {
    // Unlike the copy, this is not marker-gated: a router pointed into the old root is wrong on
    // every launch, not just the first, and rewriting a path already under the new root is a
    // no-op. It also repairs installs migrated before this existed.
    repoint_maker_data_dirs(legacy_root, root);

    let marker = root.join(".migrated-from-coinswap");
    if marker.exists() || !legacy_root.is_dir() {
        return;
    }
    if let Err(e) = merge_dir(legacy_root, root) {
        // Deliberately no marker on failure, so the next launch tries the rest again rather
        // than leaving the wallets stranded in a directory nothing reads any more.
        log::error!(
            "migrating {} to {}: {e}",
            legacy_root.display(),
            root.display()
        );
        return;
    }
    let _ = fs::write(&marker, "");
    repoint_maker_data_dirs(legacy_root, root);
}

/// Rewrites the absolute `dataDir` each router is registered under, from the old root to the new
/// one.
///
/// `makers.json` stores absolute paths, so copying it verbatim leaves every router reading and
/// writing the old tree while the taker uses the new one — two divergent copies of the same
/// wallet, and nothing at all once the old tree is deleted. Paths outside the legacy root are
/// left alone: those are locations the user chose, and they are still valid.
fn repoint_maker_data_dirs(legacy_root: &Path, root: &Path) {
    let path = root.join("maker").join("makers.json");
    let Ok(contents) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(mut parsed) = serde_json::from_str::<serde_json::Value>(&contents) else {
        log::warn!("{} is not valid JSON; leaving it alone", path.display());
        return;
    };
    let Some(makers) = parsed.get_mut("makers").and_then(|m| m.as_object_mut()) else {
        return;
    };

    let mut changed = false;
    for (_, maker) in makers.iter_mut() {
        let Some(dir) = maker.get("dataDir").and_then(|d| d.as_str()) else {
            continue;
        };
        let Ok(relative) = Path::new(dir).strip_prefix(legacy_root) else {
            continue;
        };
        let moved = root.join(relative);
        maker["dataDir"] = serde_json::Value::String(moved.to_string_lossy().into_owned());
        changed = true;
    }
    if !changed {
        return;
    }
    match serde_json::to_string_pretty(&parsed) {
        Ok(rewritten) => {
            if let Err(e) = fs::write(&path, rewritten) {
                log::error!("rewriting {}: {e}", path.display());
            }
        }
        Err(e) => log::error!("re-encoding {}: {e}", path.display()),
    }
}

/// Copies everything under `from` that `to` does not already have, recursing into directories
/// present in both. Never overwrites: anything already in the new tree is the newer copy.
///
/// Directory modes are carried over rather than left to `create_dir_all`: Tor refuses to start
/// when its data directory is group- or world-readable, and the process umask would widen it.
fn merge_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    if !to.exists() {
        fs::create_dir_all(to)?;
        fs::set_permissions(to, from.metadata()?.permissions())?;
    }
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            merge_dir(&entry.path(), &target)?;
        } else if !target.exists() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// The real shape this has to survive: Tor creates `taker/tor-manager/` under the new root
    /// on launch, so the destination already exists before any wallet has been moved into it.
    #[test]
    fn merge_fills_in_what_the_new_tree_is_missing_without_overwriting() {
        let tmp = std::env::temp_dir().join(format!("portal-merge-{}", std::process::id()));
        let (old, new) = (tmp.join("old"), tmp.join("new"));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(old.join("taker/wallets")).unwrap();
        std::fs::create_dir_all(old.join("taker/tor-manager")).unwrap();
        std::fs::create_dir_all(new.join("taker/tor-manager")).unwrap();
        std::fs::write(old.join("taker/wallets/Potterverse"), "wallet").unwrap();
        std::fs::write(old.join("taker/swap_tracker.cbor"), "tracker").unwrap();
        std::fs::write(old.join("taker/tor-manager/tor.log"), "stale").unwrap();
        std::fs::write(new.join("taker/tor-manager/tor.log"), "current").unwrap();

        super::merge_dir(&old, &new).unwrap();

        assert_eq!(read(&new.join("taker/wallets/Potterverse")), "wallet");
        assert_eq!(read(&new.join("taker/swap_tracker.cbor")), "tracker");
        // This session's own Tor log must not be replaced by the old tree's stale one.
        assert_eq!(read(&new.join("taker/tor-manager/tor.log")), "current");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// The migration runs once and then never again, whatever is left behind in the old tree:
    /// repeating it would resurrect wallets the user has since deleted from the new one.
    #[test]
    fn migration_marks_itself_done_and_does_not_run_twice() {
        let tmp = std::env::temp_dir().join(format!("portal-migrate-{}", std::process::id()));
        let (old, new) = (tmp.join("old"), tmp.join("new"));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(old.join("taker/wallets")).unwrap();
        std::fs::write(old.join("taker/wallets/Potterverse"), "wallet").unwrap();

        super::migrate_from_legacy_root(&old, &new);
        assert_eq!(read(&new.join("taker/wallets/Potterverse")), "wallet");
        assert!(new.join(".migrated-from-coinswap").exists(), "marker written");

        // A second launch: the old tree gained a file and the new tree lost one. Neither moves.
        std::fs::write(old.join("taker/wallets/Later"), "later").unwrap();
        std::fs::remove_file(new.join("taker/wallets/Potterverse")).unwrap();
        super::migrate_from_legacy_root(&old, &new);
        assert!(!new.join("taker/wallets/Later").exists(), "no second migration");
        assert!(!new.join("taker/wallets/Potterverse").exists(), "deletion stays deleted");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// `makers.json` stores absolute paths, so a verbatim copy leaves every router reading the
    /// old tree — stale reports while it still exists, and nothing at all once it is deleted.
    #[test]
    fn migration_repoints_router_data_dirs_at_the_new_root() {
        let tmp = std::env::temp_dir().join(format!("portal-repoint-{}", std::process::id()));
        let (old, new) = (tmp.join("old"), tmp.join("new"));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(old.join("maker")).unwrap();
        let elsewhere = tmp.join("custom/Franky");
        std::fs::write(
            old.join("maker/makers.json"),
            format!(
                r#"{{"makers":{{
                     "Zoro":{{"routerId":"Zoro","walletName":"Zoro","dataDir":{:?}}},
                     "Franky":{{"routerId":"Franky","walletName":"Franky","dataDir":{:?}}}
                   }}}}"#,
                old.join("Zoro").to_string_lossy(),
                elsewhere.to_string_lossy(),
            ),
        )
        .unwrap();

        super::migrate_from_legacy_root(&old, &new);

        let rewritten: serde_json::Value =
            serde_json::from_str(&read(&new.join("maker/makers.json"))).unwrap();
        assert_eq!(
            rewritten["makers"]["Zoro"]["dataDir"].as_str().unwrap(),
            new.join("Zoro").to_string_lossy(),
            "a router under the old root is repointed"
        );
        assert_eq!(
            rewritten["makers"]["Franky"]["dataDir"].as_str().unwrap(),
            elsewhere.to_string_lossy(),
            "a router the user put elsewhere is left alone"
        );
        // Other fields survive the rewrite.
        assert_eq!(rewritten["makers"]["Zoro"]["walletName"], "Zoro");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Without the marker the new root already existing proves nothing — Tor writes into it on
    /// the very first launch, before any wallet has been moved across.
    #[test]
    fn migration_still_runs_when_the_new_root_already_exists() {
        let tmp = std::env::temp_dir().join(format!("portal-premade-{}", std::process::id()));
        let (old, new) = (tmp.join("old"), tmp.join("new"));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(old.join("taker/wallets")).unwrap();
        std::fs::write(old.join("taker/wallets/Potterverse"), "wallet").unwrap();
        std::fs::create_dir_all(new.join("taker/tor-manager")).unwrap();

        super::migrate_from_legacy_root(&old, &new);

        assert_eq!(read(&new.join("taker/wallets/Potterverse")), "wallet");
        let _ = std::fs::remove_dir_all(&tmp);
    }


    #[test]
    fn listing_skips_the_crate_s_own_sidecar_files() {
        let root = std::env::temp_dir().join(format!("portal-core-wallets-{}", std::process::id()));
        let wallets = root.join("wallets");
        std::fs::create_dir_all(&wallets).unwrap();
        for name in [
            "alice",
            "bob",
            "alice_swap_report.json",
            "alice_last_address.json",
            "alice.lock",
            "bob.partial",
            "bob.tmp",
        ] {
            std::fs::write(wallets.join(name), b"x").unwrap();
        }

        let found = list_wallets(&Some(root.display().to_string())).unwrap();
        assert_eq!(found, vec!["alice".to_string(), "bob".to_string()]);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_missing_wallets_directory_is_empty_rather_than_an_error() {
        let root = std::env::temp_dir().join(format!("portal-core-absent-{}", std::process::id()));
        assert!(list_wallets(&Some(root.display().to_string()))
            .unwrap()
            .is_empty());
    }
}
