//! Log tail commands.
//!
//! Bodies live in `portal_core::ops::logs`; these wrappers register the operation with
//! Tauri and hand over the managed state.

use std::sync::Arc;

use portal_core::error::AppError;
use portal_core::ops::logs;
use portal_core::state::AppState;

use super::desktop_taker;
use portal_core::types::*;

#[tauri::command]
pub async fn get_logs(state: tauri::State<'_, Arc<AppState>>, lines: Option<usize>) -> Result<Vec<LogLine>, AppError> {
    logs::get_logs(&*desktop_taker(&state)?, lines).await
}

#[tauri::command]
pub async fn get_maker_logs(
    state: tauri::State<'_, Arc<AppState>>, router_id: String, lines: Option<usize>,
) -> Result<Vec<LogLine>, AppError> {
    logs::get_maker_logs(&state, router_id, lines).await
}

