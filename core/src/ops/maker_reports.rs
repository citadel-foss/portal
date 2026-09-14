//! Maker-side swap reports & deniability — the maker's counterpart to
//! `commands::taker_reports`. Same on-disk file
//! (`<wallet_name>_swap_report.json`), different section: openswap's
//! `SwapReportFile.maker` is a `HashMap<String, Vec<MakerReport>>` keyed by
//! maker node name (see `openswap::wallet::report::wallet_name_for_report`,
//! not itself re-exported). Each command resolves one registered maker's
//! report file and flattens the node-name buckets stored within that file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use openswap::maker::MakerServer;
use openswap::wallet::MakerReport;

use crate::ops::maker_settings;
use crate::ops::taker_reports::{self, status_label, to_report_utxos};
use crate::error::{AppError, ErrorCode};
use crate::state::AppState;
use crate::types::{MakerSwapReportDetail, MakerSwapReportSummary, Outpoint};

fn get_maker_server(state: &Arc<AppState>, router_id: &str) -> Result<Arc<MakerServer>, AppError> {
    let makers = state.makers.lock()?;
    let entry = makers
        .get(router_id)
        .ok_or_else(|| AppError::maker_not_found(router_id))?;
    entry
        .runtime
        .as_ref()
        .map(|runtime| runtime.server.clone())
        .ok_or_else(AppError::maker_not_initialized)
}

/// Mirrors `taker_reports.rs`'s own local `SwapReportFile` — the wrapper type isn't re-exported
/// from `openswap::wallet` (see that file's doc comment), so both read the on-disk JSON shape
/// directly rather than waiting on the crate to fix the re-export. Only the field this file
/// needs is declared; `taker`/`recovery`/`deniability_proofs` are ignored here.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct SwapReportFile {
    #[serde(default)]
    maker: HashMap<String, Vec<MakerReport>>,
}

fn resolve_report_path(state: &Arc<AppState>, router_id: &str) -> Result<PathBuf, AppError> {
    let makers = state.makers.lock()?;
    let in_memory = makers.get(router_id).map(|entry| entry.settings.clone());
    drop(makers);
    let settings = match in_memory {
        Some(settings) => settings,
        None => {
            maker_settings::load(router_id)?.ok_or_else(|| AppError::maker_not_found(router_id))?
        }
    };
    let data_dir = settings
        .data_dir
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(AppError::maker_not_initialized)?;
    Ok(super::taker_reports::report_path(
        &data_dir,
        &settings.wallet_name,
    ))
}

fn load_report_file(path: &Path) -> Result<SwapReportFile, AppError> {
    serde_json::from_value(taker_reports::read_report_json(path)?)
        .map_err(|e| AppError::internal(format!("failed to read {}: {e}", path.display())))
}

pub async fn list_maker_swap_reports(
    state: &Arc<AppState>,
    router_id: String,
) -> Result<Vec<MakerSwapReportSummary>, AppError> {
    let path = resolve_report_path(state, &router_id)?;
    let file = tokio::task::spawn_blocking(move || load_report_file(&path))
        .await
        .map_err(AppError::internal)??;

    let mut reports: Vec<_> = file
        .maker
        .into_values()
        .flatten()
        .map(|r| MakerSwapReportSummary {
            swap_id: r.swap_id,
            status: status_label(&r.status).to_string(),
            start_timestamp: r.start_timestamp,
            end_timestamp: r.end_timestamp,
            incoming_amount_sats: r.incoming_amount,
            outgoing_amount_sats: r.outgoing_amount,
            fee_earned_sats: r.fee_earned,
        })
        .collect();
    reports.sort_by(|a, b| {
        b.start_timestamp
            .cmp(&a.start_timestamp)
            .then_with(|| a.swap_id.cmp(&b.swap_id))
    });
    Ok(reports)
}

pub async fn get_maker_swap_report(
    state: &Arc<AppState>,
    router_id: String,
    swap_id: String,
) -> Result<MakerSwapReportDetail, AppError> {
    let path = resolve_report_path(state, &router_id)?;
    let file = tokio::task::spawn_blocking(move || load_report_file(&path))
        .await
        .map_err(AppError::internal)??;

    let r = file
        .maker
        .into_values()
        .flatten()
        .find(|r| r.swap_id == swap_id)
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::ReportNotFound,
                format!("no router report found for swap_id {swap_id}"),
            )
        })?;

    // Since upstream PR #1006 the proof is the only record of either contract's location, and
    // it names them as outpoints rather than bare txids.
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
    let deniability_proof = r
        .deniability_proof
        .map(|p| serde_json::to_value(p).unwrap_or(serde_json::Value::Null));

    Ok(MakerSwapReportDetail {
        swap_id: r.swap_id,
        status: status_label(&r.status).to_string(),
        network: r.network,
        swap_duration_seconds: r.swap_duration_seconds,
        start_timestamp: r.start_timestamp,
        end_timestamp: r.end_timestamp,
        incoming_amount_sats: r.incoming_amount,
        outgoing_amount_sats: r.outgoing_amount,
        fee_earned_sats: r.fee_earned,
        incoming_contract_outpoint,
        outgoing_contract_outpoint,
        incoming_utxos: to_report_utxos(r.incoming_utxos),
        outgoing_utxos: to_report_utxos(r.outgoing_utxos),
        timelock: r.timelock,
        deniability_proof,
    })
}

pub async fn verify_maker_deniability(
    state: &Arc<AppState>,
    router_id: String,
    swap_id: String,
) -> Result<bool, AppError> {
    let server = get_maker_server(state, &router_id)?;
    tokio::task::spawn_blocking(move || -> Result<bool, AppError> {
        Ok(server.verify_deniability(&swap_id)?)
    })
    .await
    .map_err(AppError::internal)?
}
