//! Router swap-report commands.
//!
//! Bodies live in `portal_core::ops::maker_reports`; these wrappers register the operation with
//! Tauri and hand over the managed state.

use std::sync::Arc;

use portal_core::error::AppError;
use portal_core::ops::maker_reports;
use portal_core::state::AppState;
use portal_core::types::*;

#[tauri::command]
pub async fn list_maker_swap_reports(state: tauri::State<'_, Arc<AppState>>, router_id: String) -> Result<Vec<MakerSwapReportSummary>, AppError> {
    maker_reports::list_maker_swap_reports(&state, router_id).await
}

#[tauri::command]
pub async fn get_maker_swap_report(
    state: tauri::State<'_, Arc<AppState>>, router_id: String, swap_id: String,
) -> Result<MakerSwapReportDetail, AppError> {
    maker_reports::get_maker_swap_report(&state, router_id, swap_id).await
}

#[tauri::command]
pub async fn verify_maker_deniability(
    state: tauri::State<'_, Arc<AppState>>, router_id: String, swap_id: String,
) -> Result<bool, AppError> {
    maker_reports::verify_maker_deniability(&state, router_id, swap_id).await
}

