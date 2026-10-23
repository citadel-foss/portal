//! Maker-side swap reports & deniability — the maker's counterpart to `taker_reports`. The crate
//! keeps a router's reports inside its encrypted wallet, so they are read through the running
//! server's wallet handle; a stopped router's reports are unreadable without its password.

use std::sync::Arc;

use openswap::maker::MakerServer;
use openswap::wallet::MakerReport;

use crate::ops::taker_reports::{status_label, to_report_utxos};
use crate::error::{AppError, ErrorCode};
use crate::state::AppState;
use crate::types::{MakerSwapReportDetail, MakerSwapReportSummary};

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

/// Every report the router's wallet holds. The crate keys them by wallet name, one bucket.
async fn load_reports(state: &Arc<AppState>, router_id: &str) -> Result<Vec<MakerReport>, AppError> {
    let server = get_maker_server(state, router_id).map_err(|e| match e.code {
        ErrorCode::RouterNotInitialized => AppError::new(
            ErrorCode::RouterNotRunning,
            "Start the router to see its reports. They are kept inside its wallet.",
        ),
        _ => e,
    })?;
    tokio::task::spawn_blocking(move || -> Result<Vec<MakerReport>, AppError> {
        Ok(server.wallet.read()?.swap_reports().maker.into_values().flatten().collect())
    })
    .await
    .map_err(AppError::internal)?
}

pub async fn list_maker_swap_reports(
    state: &Arc<AppState>,
    router_id: String,
) -> Result<Vec<MakerSwapReportSummary>, AppError> {
    let mut reports: Vec<_> = load_reports(state, &router_id)
        .await?
        .into_iter()
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
    let r = load_reports(state, &router_id)
        .await?
        .into_iter()
        .find(|r| r.swap_id == swap_id)
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::ReportNotFound,
                format!("no router report found for swap_id {swap_id}"),
            )
        })?;

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
