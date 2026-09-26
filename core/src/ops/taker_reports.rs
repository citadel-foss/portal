//! Swap reports & deniability. One consolidated file per wallet
//! (`<wallet_name>_swap_report.json`, not per swap id) written by the crate
//! itself — we only read it.


use std::path::{Path, PathBuf};

use openswap::bitcoin::{Address, Txid};
use openswap::taker::swap_tracker::{RecoveryPhase, SwapPhase, SwapRecord};
use openswap::wallet::{AnyBlockchain, Blockchain, SwapStatus, TakerReport, UTXOSpendInfo};

use crate::ops::chain_backend;
use crate::error::{AppError, ErrorCode};
use crate::state::{try_lock_taker, TakerInstance};
use crate::types::{
    Outpoint, ReportRouterFee, ReportUtxo, SwapReportDetail, SwapReportSummary, SwapUtxoDto,
};

/// Mirrors the `taker` field of the crate's `wallet::report::SwapReportFile` — that wrapper type
/// isn't re-exported from `openswap::wallet`, so this reads the same on-disk JSON shape directly
/// rather than waiting on the crate to fix the re-export.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct SwapReportFile {
    #[serde(default)]
    taker: Vec<TakerReport>,
}

/// Shared with `commands::maker_reports` — both resolve the same per-wallet report file.
///
/// The crate writes `<stem>_swap_report.json`, taking the *stem* of the wallet file name
/// (`wallet::report::wallet_name_for_report`), so a wallet named `wallet.dat` reports to
/// `wallet_swap_report.json`. Formatting the full file name here instead would silently
/// read a path that never exists and report zero swaps.
pub(crate) fn report_path(data_dir: &Path, wallet_name: &str) -> PathBuf {
    let stem = Path::new(wallet_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(wallet_name);
    data_dir
        .join("wallets")
        .join(format!("{stem}_swap_report.json"))
}

/// Shared with `commands::maker_reports` — both read the same `SwapStatus` enum off the same
/// on-disk report file, just different sections of it (`taker` vs `maker`).
pub(crate) fn status_label(s: &SwapStatus) -> &'static str {
    match s {
        SwapStatus::Success => "success",
        SwapStatus::RecoveryHashlock => "recovery_hashlock",
        SwapStatus::RecoveryTimelock => "recovery_timelock",
        SwapStatus::Failed => "failed",
    }
}

pub(crate) fn to_report_utxos(utxos: Vec<openswap::wallet::ReportUtxo>) -> Vec<ReportUtxo> {
    utxos
        .into_iter()
        .map(|u| ReportUtxo {
            address: u.address,
            value_sats: u.value,
        })
        .collect()
}

/// What the wallet actually got back, in sats.
///
/// `incoming_amount` is the swept total the crate measured on-chain, so it is the real figure
/// wherever it exists. It is absent on a PaySwap (the receiver was paid, nothing came back) and
/// on reports written before upstream #1015 fixed the fee accounting, hence the fallback.
fn received_sats(r: &TakerReport) -> u64 {
    if r.incoming_amount > 0 {
        r.incoming_amount
    } else {
        r.outgoing_amount.saturating_sub(r.fee_paid)
    }
}

fn resolve_report_path(taker: &TakerInstance) -> PathBuf {
    report_path(&taker.data_dir, &taker.wallet_name)
}

fn load_report_file(path: &Path) -> Result<SwapReportFile, AppError> {
    serde_json::from_value(read_report_json(path)?)
        .map_err(|e| AppError::internal(format!("failed to read {}: {e}", path.display())))
}

/// Reads a report file as raw JSON, with the compatibility fix-ups applied.
///
/// Shared with `commands::maker_reports`: both sections of the file need the same treatment, and
/// a maker's file carries taker entries too.
pub(crate) fn read_report_json(path: &Path) -> Result<serde_json::Value, AppError> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    let contents = std::fs::read_to_string(path)?;
    let mut value: serde_json::Value = serde_json::from_str(&contents)
        .map_err(|e| AppError::internal(format!("failed to parse {}: {e}", path.display())))?;
    backfill_report_utxos(&mut value);
    Ok(value)
}

/// Gives every report entry the `*_utxos` arrays upstream PR #1006 added.
///
/// It made them required rather than defaulted on both `TakerReport` and `MakerReport`, so a file
/// written before that update fails to deserialise and takes the wallet's *entire* swap history
/// with it — one missing field and every past swap reads as unopenable. An empty list is what the
/// field means for those swaps: nothing of the kind was recorded.
fn backfill_report_utxos(value: &mut serde_json::Value) {
    fn patch(entry: &mut serde_json::Value) {
        let Some(object) = entry.as_object_mut() else {
            return;
        };
        for field in ["outgoing_utxos", "incoming_utxos"] {
            object
                .entry(field)
                .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        }
    }
    if let Some(entries) = value.get_mut("taker").and_then(|t| t.as_array_mut()) {
        entries.iter_mut().for_each(patch);
    }
    // The maker section is keyed by maker node name, one bucket of reports each.
    if let Some(buckets) = value.get_mut("maker").and_then(|m| m.as_object_mut()) {
        for (_, bucket) in buckets.iter_mut() {
            if let Some(entries) = bucket.as_array_mut() {
                entries.iter_mut().for_each(patch);
            }
        }
    }
}

/// Every record in `swap_tracker.cbor`, deserialised here rather than through `SwapTracker`.
///
/// Its own `incomplete_swaps()` is the only enumeration the crate exposes, and it deliberately
/// excludes a swap that failed and was then recovered — which is exactly the case that has no
/// report file entry either, so those swaps would be invisible in both places. The container is
/// one field, and `SwapRecord` is public and `Deserialize`, so this reads the same bytes the
/// crate wrote. A shape change upstream degrades to "report file only" rather than an error.
fn tracker_records(data_dir: &Path) -> Vec<SwapRecord> {
    #[derive(serde::Deserialize)]
    struct TrackerFile {
        swaps: std::collections::HashMap<String, SwapRecord>,
    }
    let path = data_dir.join("swap_tracker.cbor");
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    match serde_cbor::from_slice::<TrackerFile>(&bytes) {
        Ok(file) => file.swaps.into_values().collect(),
        Err(error) => {
            log::warn!("could not read {}: {error:?}", path.display());
            Vec::new()
        }
    }
}

pub async fn list_swap_reports(
    taker: &TakerInstance,
) -> Result<Vec<SwapReportSummary>, AppError> {
    let path = resolve_report_path(taker);
    let data_dir = taker.data_dir.clone();
    let file = tokio::task::spawn_blocking(move || load_report_file(&path))
        .await
        .map_err(AppError::internal)??;

    let mut rows: Vec<SwapReportSummary> = file
        .taker
        .iter()
        .map(|r| SwapReportSummary {
            swap_id: r.swap_id.clone(),
            status: status_label(&r.status).to_string(),
            reported: true,
            start_timestamp: r.start_timestamp,
            end_timestamp: Some(r.end_timestamp),
            outgoing_amount_sats: r.outgoing_amount,
            received_amount_sats: received_sats(r),
            fee_paid_sats: r.fee_paid,
            routers_count: r.makers_count,
        })
        .collect();

    // A report only gets written from inside `start_swap`. A swap the process never returned
    // from — the app was closed, or it crashed — is marked Failed by `cleanup_incomplete` at the
    // next launch, which writes no report at all. Those swaps really happened and may still be
    // holding funds, so listing only the report file understates what the wallet has done and
    // reports "0 failed" while money sits in a contract.
    let reported: std::collections::HashSet<String> =
        rows.iter().map(|r| r.swap_id.clone()).collect();
    for record in tracker_records(&data_dir) {
        if reported.contains(&record.swap_id) {
            continue;
        }
        let status = if record.phase == SwapPhase::Completed {
            "success"
        } else if record.phase == SwapPhase::Failed {
            if record.recovery.phase >= RecoveryPhase::CleanedUp {
                "recovered"
            } else {
                "interrupted"
            }
        } else {
            "unfinished"
        };
        rows.push(SwapReportSummary {
            swap_id: record.swap_id.clone(),
            status: status.to_string(),
            reported: false,
            start_timestamp: record.created_at,
            end_timestamp: None,
            outgoing_amount_sats: record.send_amount_sat,
            // The tracker keeps no fee figures, so nothing is invented for them.
            received_amount_sats: 0,
            fee_paid_sats: 0,
            routers_count: record.maker_count,
        });
    }
    rows.sort_by_key(|r| r.start_timestamp);
    Ok(rows)
}

pub async fn get_swap_report(
    taker: &TakerInstance,
    swap_id: String,
) -> Result<SwapReportDetail, AppError> {
    let path = resolve_report_path(taker);
    let file = tokio::task::spawn_blocking(move || load_report_file(&path))
        .await
        .map_err(AppError::internal)??;

    let r = file
        .taker
        .into_iter()
        .find(|r| r.swap_id == swap_id)
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::WalletNotFound,
                format!("no report found for swap_id {swap_id}"),
            )
        })?;

    let received_amount_sats = received_sats(&r);
    let router_fee_info = r
        .maker_fee_info
        .into_iter()
        .map(|m| ReportRouterFee {
            router_index: m.maker_index,
            router_address: m.maker_address,
            base_fee_sats: m.base_fee,
            amount_relative_fee_sats: m.amount_relative_fee,
            time_relative_fee_sats: m.time_relative_fee,
            total_fee_sats: m.total_fee,
        })
        .collect();

    // Both sides of the route as outpoints. `proven_outpoint` is the incoming contract — the one
    // `verify_deniability` checks on-chain — and `outgoing_swapcoin` is what this wallet paid in.
    let to_outpoint = |op: openswap::bitcoin::OutPoint| Outpoint {
        txid: op.txid.to_string(),
        vout: op.vout,
    };
    let incoming_contract_outpoint = r
        .deniability_proof
        .as_ref()
        .map(|p| to_outpoint(p.proven_outpoint()));
    let outgoing_contract_outpoint = r
        .deniability_proof
        .as_ref()
        .and_then(|p| p.outgoing_swapcoin)
        .map(to_outpoint);
    // Raw pass-through — see the field's doc comment in types.rs for why this isn't hand-mirrored.
    let deniability_proof = r
        .deniability_proof
        .map(|p| serde_json::to_value(p).unwrap_or(serde_json::Value::Null));

    Ok(SwapReportDetail {
        swap_id: r.swap_id,
        status: status_label(&r.status).to_string(),
        network: r.network,
        swap_duration_seconds: r.swap_duration_seconds,
        start_timestamp: r.start_timestamp,
        end_timestamp: r.end_timestamp,
        error_message: r.error_message,
        outgoing_amount_sats: r.outgoing_amount,
        received_amount_sats,
        fee_paid_sats: r.fee_paid,
        mining_fee_sats: r.mining_fee,
        fee_percentage: r.fee_percentage,
        total_router_fees_sats: r.total_maker_fees,
        outgoing_utxos: to_report_utxos(r.outgoing_utxos),
        incoming_utxos: to_report_utxos(r.incoming_utxos),
        funding_txids: r.funding_txids,
        routers_count: r.makers_count,
        router_addresses: r.maker_addresses,
        router_fee_info,
        outgoing_contract_outpoint,
        incoming_contract_outpoint,
        deniability_proof,
    })
}

pub async fn verify_deniability(
    instance: &TakerInstance,
    swap_id: String,
) -> Result<bool, AppError> {
    let taker = instance.taker.clone();
    tokio::task::spawn_blocking(move || -> Result<bool, AppError> {
        let guard = try_lock_taker(&taker)?;
        let taker = guard.as_ref().ok_or_else(AppError::not_initialized)?;
        taker
            .verify_deniability(&swap_id)
            .map_err(|e| AppError::internal(e.to_string()))
    })
    .await
    .map_err(AppError::internal)?
}

/// Recovers the coin a swap actually paid the user.
///
/// The report file can't answer this: its `output_swap_utxos` is the wallet's whole swept-coin
/// set at write time, not this swap's, and it stores bare amounts with no outpoint. So this
/// walks the chain instead — the sweep is the transaction spending the incoming contract
/// outpoint, and its output is the coin. Costs a chain round-trip per candidate, so it is a
/// separate on-demand command rather than part of `get_swap_report`.
/// Chain round-trips this lookup is willing to spend; the closest-valued candidate is almost
/// always the coin, so this only bounds a wallet that has accumulated many unspent sweeps.
const MAX_SWEPT_CANDIDATES: usize = 8;

pub async fn get_incoming_swap_utxo(
    taker: &TakerInstance,
    swap_id: String,
) -> Result<Option<SwapUtxoDto>, AppError> {
    let path = resolve_report_path(taker);
    let wallet = taker.wallet.clone();
    // The session config can be re-pointed mid-session; this is the route the wallet is
    // actually built against.
    let active_backend = taker.chain_backend.clone();
    let socks_port = Some(taker.socks_port);

    tokio::task::spawn_blocking(move || -> Result<Option<SwapUtxoDto>, AppError> {
        let report = load_report_file(&path)?
            .taker
            .into_iter()
            .find(|r| r.swap_id == swap_id)
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::ReportNotFound,
                    format!("no router report found for swap_id {swap_id}"),
                )
            })?;

        // The proof is the only record of the incoming contract outpoint — upstream PR #1006
        // dropped the bare contract txids from the report — so a swap that produced no proof
        // cannot be matched to its sweep at all.
        let Some(contract) = report
            .deniability_proof
            .as_ref()
            .map(|p| p.proven_outpoint())
        else {
            return Ok(None);
        };

        // Deliberately not `get_transactions`: the incoming contract's script is watched
        // (`Wallet::watch_script`), so Electrum values the sweep's input as spent-by-us, marks
        // the whole transaction a send, and then skips its output back to us — the sweep is
        // absent from wallet history entirely. The crate already tags the resulting coin
        // `SweptCoin` in the local UTXO cache, which costs no round-trip at all.
        let (wallet_name, candidates) = {
            let w = wallet.read()?;
            // `fee_paid` includes the funding transaction's mining fee, which left the input
            // rather than the contract, so the report's figure lands near the coin without
            // equalling it. It only orders the candidates — the contract-outpoint check below
            // is what identifies the coin.
            let target = report.outgoing_amount.saturating_sub(report.fee_paid);
            let mut swept: Vec<(u64, Txid, u32)> = w
                .list_all_utxo_spend_info()
                .into_iter()
                .filter(|(_, info)| matches!(info, UTXOSpendInfo::SweptCoin { .. }))
                .map(|(utxo, _)| (utxo.amount.to_sat().abs_diff(target), utxo.txid, utxo.vout))
                .collect();
            swept.sort_unstable();
            swept.truncate(MAX_SWEPT_CANDIDATES);
            let ordered: Vec<(Txid, u32)> =
                swept.into_iter().map(|(_, txid, vout)| (txid, vout)).collect();
            (w.get_name().to_string(), ordered)
        };

        let backend = AnyBlockchain::from_config(&chain_backend::resolve_from(
            &active_backend,
            &wallet_name,
            socks_port,
        )?)
        .map_err(|e| AppError::internal(format!("{e:?}")))?;
        // `Wallet` keeps its network private, so the backend is the only source for the
        // address encoding.
        let network = backend
            .get_blockchain_info()
            .map(|info| info.chain)
            .map_err(|e| AppError::internal(format!("{e:?}")))?;

        for (txid, vout) in candidates {
            let Ok(tx) = backend.get_raw_transaction(&txid, None) else {
                continue;
            };
            if !tx.input.iter().any(|i| i.previous_output == contract) {
                continue;
            }
            let Some(out) = tx.output.get(vout as usize) else {
                continue;
            };
            return Ok(Some(SwapUtxoDto {
                txid: txid.to_string(),
                vout,
                amount_sats: out.value.to_sat(),
                // Decoding the script here is what gives a real address on Electrum, whose
                // `list_unspent` leaves the address blank.
                address: Address::from_script(&out.script_pubkey, network)
                    .ok()
                    .map(|a| a.to_string()),
            }));
        }
        Ok(None)
    })
    .await
    .map_err(AppError::internal)?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The crate writes reports under the wallet file's *stem*, so a dotted wallet name must
    /// not produce `wallet.dat_swap_report.json` — that path never exists and reads as zero swaps.
    #[test]
    fn report_path_uses_the_wallet_file_stem() {
        let dir = Path::new("/data");
        assert_eq!(
            report_path(dir, "wallet.dat"),
            dir.join("wallets").join("wallet_swap_report.json")
        );
        assert_eq!(
            report_path(dir, "taker-wallet"),
            dir.join("wallets").join("taker-wallet_swap_report.json")
        );
    }

    /// Reports written before upstream PR #1006 have no `*_utxos` arrays, which the crate's
    /// structs made required — without the backfill a single missing field makes the whole
    /// file unreadable and the wallet's entire swap history disappears from the UI.
    #[test]
    fn a_report_written_before_the_utxo_fields_existed_still_loads() {
        // Field-for-field the shape a pre-update taker report has on disk, including the two
        // contract txids the same upstream change removed.
        let old_schema = serde_json::json!({
            "taker": [{
                "swap_id": "040f15b3deb9c6ab",
                "status": "Success",
                "network": "signet",
                "swap_duration_seconds": 258.9433155,
                "start_timestamp": 1787141340u64,
                "end_timestamp": 1787141598u64,
                "error_message": null,
                "outgoing_amount": 155190,
                "incoming_amount": 151695,
                "fee_paid": 3825,
                "mining_fee": 1653,
                "fee_percentage": 2.464720664991301,
                "total_maker_fees": 2172,
                "outgoing_contract_txid": "47".repeat(32),
                "incoming_contract_txid": "5d".repeat(32),
                "funding_txids": [[]],
                "makers_count": 2,
                "maker_addresses": ["a.onion", "b.onion"],
                "maker_fee_info": [],
                "input_utxos": [10000000],
                "output_change_amounts": [9844480],
                "output_swap_amounts": [151695],
                "output_change_utxos": [[9844480, "Unknown"]],
                "output_swap_utxos": [[151695, "Unknown"]],
                "deniability_proof": null
            }]
        });

        let mut value = old_schema;
        super::backfill_report_utxos(&mut value);
        let file: SwapReportFile =
            serde_json::from_value(value).expect("a pre-#1006 report must still deserialise");

        assert_eq!(file.taker.len(), 1);
        assert_eq!(file.taker[0].swap_id, "040f15b3deb9c6ab");
        assert_eq!(file.taker[0].outgoing_amount, 155190);
        // Absent on disk, so the only honest reading is that none were recorded.
        assert!(file.taker[0].outgoing_utxos.is_empty());
    }

    /// A dotfile name is all extension and no stem; falling back to the raw name keeps the
    /// path in the wallets directory instead of collapsing to `_swap_report.json`.
    #[test]
    fn report_path_falls_back_when_there_is_no_stem() {
        assert_eq!(
            report_path(Path::new("/data"), ".wallet"),
            Path::new("/data")
                .join("wallets")
                .join(".wallet_swap_report.json")
        );
    }
}
