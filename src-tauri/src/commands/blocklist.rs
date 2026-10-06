//! Funding-address blocklist commands, one set per side: every wallet shares one list and every
//! router shares another.
//!
//! Bodies live in `portal_core::ops::blocklist`; these wrappers resolve whose list it is.

use std::sync::Arc;

use portal_core::error::AppError;
use portal_core::ops::blocklist;
use portal_core::state::{AppState, DESKTOP_SESSION};
use portal_core::types::*;

use super::desktop_taker;

#[tauri::command]
pub async fn list_taker_blocklist(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<BlocklistEntryDto>, AppError> {
    blocklist::list(DESKTOP_SESSION, &desktop_taker(&state)?.data_dir).await
}

#[tauri::command]
pub async fn import_taker_blocklist(
    state: tauri::State<'_, Arc<AppState>>,
    csv: String,
) -> Result<BlocklistImportDto, AppError> {
    let dir = desktop_taker(&state)?.data_dir.clone();
    blocklist::import(DESKTOP_SESSION, dir, csv).await
}

#[tauri::command]
pub async fn remove_taker_blocklist(
    state: tauri::State<'_, Arc<AppState>>,
    addresses: Vec<String>,
) -> Result<usize, AppError> {
    let dir = desktop_taker(&state)?.data_dir.clone();
    blocklist::remove(DESKTOP_SESSION, dir, addresses).await
}

#[tauri::command]
pub async fn list_maker_blocklist(router_id: String) -> Result<Vec<BlocklistEntryDto>, AppError> {
    blocklist::list(DESKTOP_SESSION, &blocklist::router_dir(&router_id)?).await
}

#[tauri::command]
pub async fn import_maker_blocklist(
    router_id: String,
    csv: String,
) -> Result<BlocklistImportDto, AppError> {
    blocklist::import(DESKTOP_SESSION, blocklist::router_dir(&router_id)?, csv).await
}

#[tauri::command]
pub async fn remove_maker_blocklist(
    router_id: String,
    addresses: Vec<String>,
) -> Result<usize, AppError> {
    blocklist::remove(
        DESKTOP_SESSION,
        blocklist::router_dir(&router_id)?,
        addresses,
    )
    .await
}
