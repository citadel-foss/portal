//! Swap and recovery commands.
//!
//! Bodies live in `portal_core::ops::taker_swap`. The main-window check stays here: window
//! identity is a Tauri concept the web host authorizes through its session instead.

use std::sync::Arc;

use crate::native::ensure_main_window;
use portal_core::error::AppError;
use portal_core::ops::taker_swap;
use portal_core::state::AppState;

use super::desktop_taker;
use portal_core::types::*;

#[tauri::command]
pub async fn estimate_swap_funding(
    state: tauri::State<'_, Arc<AppState>>, amount_sats: u64, protocol: ProtocolVersionDto, outpoints: Option<Vec<Outpoint>>, tx_count: Option<u32>,
) -> Result<SwapFundingEstimateDto, AppError> {
    taker_swap::estimate_swap_funding(&*desktop_taker(&state)?, amount_sats, protocol, outpoints, tx_count).await
}

#[tauri::command]
pub async fn prepare_swap(state: tauri::State<'_, Arc<AppState>>, request: SwapRequest) -> Result<SwapSummaryDto, AppError> {
    taker_swap::prepare_swap(&*desktop_taker(&state)?, request).await
}

#[tauri::command]
pub async fn get_swap_preparation(state: tauri::State<'_, Arc<AppState>>, since: u64) -> Result<Option<SwapPreparationDto>, AppError> {
    taker_swap::get_swap_preparation(&*desktop_taker(&state)?, since).await
}

#[tauri::command]
pub async fn start_swap(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<AppState>>,
    swap_id: String,
) -> Result<(), AppError> {
    ensure_main_window(&window)?;
    taker_swap::start_swap(&state, &desktop_taker(&state)?, swap_id).await
}

#[tauri::command]
pub fn get_swap_progress(state: tauri::State<'_, Arc<AppState>>) -> Result<Option<SwapProgressDto>, AppError> {
    taker_swap::get_swap_progress(&*desktop_taker(&state)?)
}

#[tauri::command]
pub async fn get_swap_tracker(
    state: tauri::State<'_, Arc<AppState>>, swap_id: Option<String>,
) -> Result<Option<SwapTrackerDto>, AppError> {
    taker_swap::get_swap_tracker(&*desktop_taker(&state)?, swap_id).await
}

#[tauri::command]
pub async fn recover_swap(state: tauri::State<'_, Arc<AppState>>) -> Result<(), AppError> {
    taker_swap::recover_swap(&*desktop_taker(&state)?).await
}

#[tauri::command]
pub async fn list_recoveries(state: tauri::State<'_, Arc<AppState>>) -> Result<Vec<RecoverySummary>, AppError> {
    taker_swap::list_recoveries(&*desktop_taker(&state)?).await
}

#[tauri::command]
pub async fn get_recovery_status(
    state: tauri::State<'_, Arc<AppState>>, swap_id: Option<String>,
) -> Result<RecoveryStatus, AppError> {
    taker_swap::get_recovery_status(&*desktop_taker(&state)?, swap_id).await
}

