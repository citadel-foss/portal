//! The funding-address blocklist: swaps funded from a listed address are refused. The crate does
//! the screening; this imports, lists and removes entries.
//!
//! The crate keeps the list in the parent of a wallet's or router's data dir, so every wallet
//! shares one file and every router shares another.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use openswap::bitcoin::{Address, Network};
use openswap::blocklist::{blocklist_path, AddressBlocklist, BlocklistEntry};

use crate::error::{AppError, ErrorCode};
use crate::ops::{chain_backend, maker_settings};
use crate::types::{BlocklistEntryDto, BlocklistImportDto, BlocklistRejectDto};

/// Far above any real list; it only bounds what one upload can make the server parse.
const MAX_CSV_BYTES: usize = 5 * 1024 * 1024;

pub fn router_dir(router_id: &str) -> Result<PathBuf, AppError> {
    let settings =
        maker_settings::load(router_id)?.ok_or_else(|| AppError::maker_not_found(router_id))?;
    maker_settings::maker_data_dir(&settings)
}

/// The chain a wallet or router is on. Both are only shown on the chain the session reached, so
/// the session's chain stands in for one Portal never recorded.
async fn network(session: &str, dir: &Path) -> Result<Network, AppError> {
    let chain = match crate::storage::recorded_network(dir) {
        Some(chain) => chain,
        None => {
            let socks_port = crate::tor::ensure_tor()
                .map_err(|e| AppError::new(ErrorCode::TorUnreachable, e))?
                .socks_port;
            chain_backend::check_backend(session, None, Some(socks_port))
                .await?
                .chain
                .ok_or_else(|| {
                    AppError::new(ErrorCode::RpcUnreachable, "Could not confirm the network.")
                })?
        }
    };
    Network::from_str(&chain).map_err(AppError::internal)
}

/// The address in its canonical form: bech32 is case-insensitive, so `TB1Q…` and `tb1q…` are one
/// entry.
fn on_network(address: &str, network: Network) -> Result<String, String> {
    Address::from_str(address)
        .map_err(|_| "not a Bitcoin address".to_string())?
        .require_network(network)
        .map(|address| address.to_string())
        .map_err(|_| format!("not a {} address", display_network(network)))
}

fn display_network(network: Network) -> String {
    match network {
        Network::Bitcoin => "mainnet".to_string(),
        other => other.to_string(),
    }
}

fn blocklist_error(e: openswap::blocklist::BlocklistError) -> AppError {
    AppError::new(ErrorCode::Io, e.to_string())
}

/// Entries for this chain only: the file is shared by every chain's wallets, and the crate
/// ignores the others when it screens.
pub async fn list(session: &str, dir: &Path) -> Result<Vec<BlocklistEntryDto>, AppError> {
    #[derive(serde::Deserialize)]
    struct File {
        #[serde(default)]
        entries: Vec<BlocklistEntry>,
    }
    let network = network(session, dir).await?;
    // The crate keeps its entries private, so the list is read from the file it writes.
    let bytes = match std::fs::read(blocklist_path(dir)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let file: File = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::new(ErrorCode::Io, format!("The blocklist file is unreadable: {e}")))?;
    Ok(file
        .entries
        .into_iter()
        .filter(|entry| on_network(&entry.address, network).is_ok())
        .map(|entry| BlocklistEntryDto {
            address: entry.address,
            label: entry.label,
        })
        .collect())
}

/// One address per line, an optional label after a comma. Blank lines, `#` comments and a
/// leading `address` header are skipped.
fn parse_csv(csv: &str, network: Network) -> (Vec<BlocklistEntry>, Vec<BlocklistRejectDto>) {
    let mut entries = Vec::new();
    let mut rejected = Vec::new();
    let mut seen = HashSet::new();
    let unquote = |field: &str| field.trim().trim_matches('"').trim().to_string();
    for (index, line) in csv.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.splitn(2, ',');
        let address = unquote(fields.next().unwrap_or_default());
        if entries.is_empty() && rejected.is_empty() && address.eq_ignore_ascii_case("address") {
            continue;
        }
        let address = match on_network(&address, network) {
            Ok(canonical) => canonical,
            Err(reason) => {
                rejected.push(BlocklistRejectDto {
                    line: index + 1,
                    address,
                    reason,
                });
                continue;
            }
        };
        if !seen.insert(address.clone()) {
            continue;
        }
        let label = fields.next().map(unquote).filter(|label| !label.is_empty());
        entries.push(BlocklistEntry::new(address, label));
    }
    (entries, rejected)
}

/// Rows the crate would refuse are reported rather than imported: it rejects a whole batch over
/// one bad address.
pub async fn import(session: &str, dir: PathBuf, csv: String) -> Result<BlocklistImportDto, AppError> {
    if csv.len() > MAX_CSV_BYTES {
        return Err(AppError::new(ErrorCode::InvalidInput, "That file is too large."));
    }
    let network = network(session, &dir).await?;
    let (entries, rejected) = parse_csv(&csv, network);
    if entries.is_empty() {
        return Ok(BlocklistImportDto {
            added: 0,
            updated: 0,
            rejected,
        });
    }
    let outcome = tokio::task::spawn_blocking(move || {
        AddressBlocklist::load(&dir, network)
            .and_then(|mut blocklist| blocklist.add(entries))
            .map_err(blocklist_error)
    })
    .await
    .map_err(AppError::internal)??;
    Ok(BlocklistImportDto {
        added: outcome.added,
        updated: outcome.updated,
        rejected,
    })
}

pub async fn remove(session: &str, dir: PathBuf, addresses: Vec<String>) -> Result<usize, AppError> {
    let network = network(session, &dir).await?;
    tokio::task::spawn_blocking(move || {
        AddressBlocklist::load(&dir, network)
            .and_then(|mut blocklist| blocklist.remove(addresses))
            .map_err(blocklist_error)
    })
    .await
    .map_err(AppError::internal)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_csv_imports_its_valid_rows_and_reports_the_rest() {
        let csv = "address,label\n\
                   tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7,exchange\n\
                   \n\
                   # a comment\n\
                   \"tb1pqqqqp399et2xygdj5xreqhjjvcmzhxw4aywxecjdzew6hylgvsesf3hn0c\"\n\
                   TB1QRP33G0Q5C5TXSP9ARYSRX4K6ZDKFS4NCE4XJ0GDCCCEFVPYSXF3Q0SL5K7,same address upper-cased\n\
                   bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq\n\
                   not-an-address\n";
        let (entries, rejected) = parse_csv(csv, Network::Signet);
        let addresses: Vec<_> = entries.iter().map(|e| e.address.as_str()).collect();
        assert_eq!(
            addresses,
            [
                "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7",
                "tb1pqqqqp399et2xygdj5xreqhjjvcmzhxw4aywxecjdzew6hylgvsesf3hn0c",
            ]
        );
        assert_eq!(entries[0].label.as_deref(), Some("exchange"));
        assert_eq!(entries[1].label, None);
        let lines: Vec<_> = rejected.iter().map(|r| (r.line, r.reason.as_str())).collect();
        assert_eq!(lines, [(7, "not a signet address"), (8, "not a Bitcoin address")]);
    }

    /// Through the crate's own file: siblings share it, and other chains' entries stay hidden.
    #[tokio::test]
    async fn an_imported_list_is_shared_by_siblings_and_removable() {
        let root = std::env::temp_dir().join(format!("portal-blocklist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (alice, bob) = (root.join("alice"), root.join("bob"));
        for dir in [&alice, &bob] {
            std::fs::create_dir_all(dir).unwrap();
            crate::storage::record_network(dir, "signet").unwrap();
        }
        let signet = "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7";

        let imported = import("test", alice.clone(), format!("{signet},exchange\n")).await.unwrap();
        assert_eq!((imported.added, imported.updated), (1, 0));
        let listed = list("test", &bob).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].label.as_deref(), Some("exchange"));

        crate::storage::record_network(&bob, "bitcoin").unwrap();
        assert!(list("test", &bob).await.unwrap().is_empty());

        assert_eq!(remove("test", alice.clone(), vec![signet.to_string()]).await.unwrap(), 1);
        assert!(list("test", &alice).await.unwrap().is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
