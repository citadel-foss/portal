//! Swap reports & deniability. The crate keeps its reports inside the encrypted wallet
//! (`Wallet::swap_reports`), so they are read through the open wallet's handle.

use openswap::bitcoin::{Address, Txid};
use openswap::wallet::{AnyBlockchain, Blockchain, SwapStatus, TakerReport, UTXOSpendInfo};

use crate::ops::chain_backend;
use crate::ops::taker_journal::{self, SwapEntry, SwapOutcome};
use crate::error::{AppError, ErrorCode};
use crate::state::{try_lock_taker, TakerInstance};
use crate::types::{
    ReportRouterFee, ReportUtxo, SwapReportDetail, SwapReportSummary, SwapUtxoDto,
};

/// Shared with `commands::maker_reports`: both sides report the same `SwapStatus`.
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

/// One swap's report, read off the open wallet. Blocking: a swap can hold the wallet a while.
async fn find_report(taker: &TakerInstance, swap_id: String) -> Result<TakerReport, AppError> {
    let wallet = taker.wallet.clone();
    tokio::task::spawn_blocking(move || {
        wallet
            .read()?
            .swap_reports()
            .taker
            .into_iter()
            .find(|r| r.swap_id == swap_id)
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::ReportNotFound,
                    format!("no report found for swap_id {swap_id}"),
                )
            })
    })
    .await
    .map_err(AppError::internal)?
}

pub async fn list_swap_reports(
    taker: &TakerInstance,
) -> Result<Vec<SwapReportSummary>, AppError> {
    let wallet = taker.wallet.clone();
    let (data_dir, key) = (taker.data_dir.clone(), taker.journal_key.clone());
    let live_swap = taker.running_swap_id();
    tokio::task::spawn_blocking(move || -> Result<Vec<SwapReportSummary>, AppError> {
        let mut rows: Vec<SwapReportSummary> = wallet
            .read()?
            .swap_reports()
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

        let journal = taker_journal::load_for_view(&data_dir, &key);
        let unreported = unreported_rows(journal.into_values(), &rows, live_swap.as_deref());
        rows.extend(unreported);
        rows.sort_by_key(|r| r.start_timestamp);
        Ok(rows)
    })
    .await
    .map_err(AppError::internal)?
}

/// History rows for journaled swaps the crate wrote no report for.
///
/// The crate writes a report only from inside `start_swap`. A swap whose process died, or that
/// failed past a point the crate reports, gets none — but it really happened and may still be
/// holding funds, so leaving it out would report "0 failed" while money sits in a contract.
fn unreported_rows(
    journal: impl IntoIterator<Item = SwapEntry>,
    reported: &[SwapReportSummary],
    live_swap: Option<&str>,
) -> Vec<SwapReportSummary> {
    journal
        .into_iter()
        .filter(|entry| !reported.iter().any(|r| r.swap_id == entry.swap_id))
        .filter_map(|entry| {
            let status = match entry.effective_outcome(live_swap) {
                SwapOutcome::Recovering => "interrupted",
                SwapOutcome::Recovered => "recovered",
                _ => return None,
            };
            Some(SwapReportSummary {
                status: status.to_string(),
                reported: false,
                start_timestamp: entry.created_at,
                end_timestamp: None,
                outgoing_amount_sats: entry.send_amount_sats,
                // The journal keeps no fee figures, so nothing is invented for them.
                received_amount_sats: 0,
                fee_paid_sats: 0,
                routers_count: entry.routers.len(),
                swap_id: entry.swap_id,
            })
        })
        .collect()
}

pub async fn get_swap_report(
    taker: &TakerInstance,
    swap_id: String,
) -> Result<SwapReportDetail, AppError> {
    let r = find_report(taker, swap_id).await?;
    let raw = serde_json::to_string_pretty(&r).map_err(AppError::internal)?;

    let received_amount_sats = received_sats(&r);
    let pay_swap = r.payment.is_some();
    let router_fee_info = r
        .maker_fee_info
        .into_iter()
        .map(|m| ReportRouterFee {
            router_index: m.maker_index,
            base_fee_sats: m.base_fee,
            amount_relative_fee_sats: m.amount_relative_fee,
            time_relative_fee_sats: m.time_relative_fee,
            total_fee_sats: m.total_fee,
        })
        .collect();

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
        pay_swap,
        deniability_proof,
        raw,
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
/// The report can't answer this: its `output_swap_utxos` is the wallet's whole swept-coin
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
    let report = find_report(taker, swap_id).await?;
    let wallet = taker.wallet.clone();
    // The session config can be re-pointed mid-session; this is the route the wallet is
    // actually built against.
    let active_backend = taker.chain_backend.clone();
    let socks_port = Some(taker.socks_port);

    tokio::task::spawn_blocking(move || -> Result<Option<SwapUtxoDto>, AppError> {

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

        let backend = AnyBlockchain::from_config(&chain_backend::resolve_bounded(
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
    use crate::types::SwapSummaryDto;

    fn entry(id: &str) -> SwapEntry {
        SwapEntry::start(&SwapSummaryDto {
            swap_id: id.into(),
            protocol: "taproot".into(),
            send_amount_sats: 50_000,
            routers: Vec::new(),
            total_estimated_fee_sats: 0,
            estimated_receive_amount_sats: 0,
            router_fee_sats: 0,
            mining_fee_sats: 0,
            payment: None,
        })
    }

    fn with_outcome(id: &str, outcome: SwapOutcome) -> SwapEntry {
        SwapEntry {
            outcome,
            ..entry(id)
        }
    }

    fn reported(id: &str) -> SwapReportSummary {
        SwapReportSummary {
            swap_id: id.into(),
            status: "failed".into(),
            reported: true,
            start_timestamp: 0,
            end_timestamp: Some(0),
            outgoing_amount_sats: 0,
            received_amount_sats: 0,
            fee_paid_sats: 0,
            routers_count: 0,
        }
    }

    #[test]
    fn recovering_and_recovered_swaps_without_a_report_are_listed() {
        let rows = unreported_rows(
            [
                with_outcome("a", SwapOutcome::Recovering),
                with_outcome("b", SwapOutcome::Recovered),
            ],
            &[],
            None,
        );
        let statuses: Vec<_> = rows.iter().map(|r| (r.swap_id.as_str(), r.status.as_str())).collect();
        assert_eq!(statuses, [("a", "interrupted"), ("b", "recovered")]);
        assert!(rows.iter().all(|r| !r.reported && r.end_timestamp.is_none()));
        assert_eq!(rows[0].outgoing_amount_sats, 50_000);
    }

    #[test]
    fn a_swap_the_crate_reported_is_not_listed_twice() {
        let rows = unreported_rows(
            [with_outcome("a", SwapOutcome::Recovering)],
            &[reported("a")],
            None,
        );
        assert!(rows.is_empty());
    }

    #[test]
    fn finished_aborted_and_running_swaps_are_left_to_their_own_views() {
        let mut funded = entry("live");
        funded.observe("Broadcast contract tx: aa");
        let rows = unreported_rows(
            [
                with_outcome("done", SwapOutcome::Completed),
                with_outcome("aborted", SwapOutcome::Aborted),
                funded,
            ],
            &[],
            Some("live"),
        );
        assert!(rows.is_empty());
    }
}
