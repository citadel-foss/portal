//! Wallet swap-report commands.
//!
//! Bodies live in `portal_core::ops::taker_reports`; these wrappers register the operation with
//! Tauri and hand over the managed state.

use std::sync::Arc;

use portal_core::error::AppError;
use portal_core::ops::taker_reports;
use portal_core::state::AppState;
use portal_core::types::*;

#[tauri::command]
pub async fn list_swap_reports(state: tauri::State<'_, Arc<AppState>>) -> Result<Vec<SwapReportSummary>, AppError> {
    taker_reports::list_swap_reports(&state).await
}

#[tauri::command]
pub async fn get_swap_report(state: tauri::State<'_, Arc<AppState>>, swap_id: String) -> Result<SwapReportDetail, AppError> {
    taker_reports::get_swap_report(&state, swap_id).await
}

#[tauri::command]
pub async fn verify_deniability(state: tauri::State<'_, Arc<AppState>>, swap_id: String) -> Result<bool, AppError> {
    taker_reports::verify_deniability(&state, swap_id).await
}

#[tauri::command]
pub async fn get_incoming_swap_utxo(state: tauri::State<'_, Arc<AppState>>, swap_id: String) -> Result<Option<SwapUtxoDto>, AppError> {
    taker_reports::get_incoming_swap_utxo(&state, swap_id).await
}

