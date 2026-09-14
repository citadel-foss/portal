//! Market and offerbook commands.
//!
//! Bodies live in `portal_core::ops::market`; these wrappers register the operation with
//! Tauri and hand over the managed state.

use std::sync::Arc;

use portal_core::error::AppError;
use portal_core::ops::market;
use portal_core::state::AppState;
use portal_core::types::*;

#[tauri::command]
pub fn get_offers(state: tauri::State<'_, Arc<AppState>>) -> Result<OfferBookView, AppError> {
    market::get_offers(&state)
}

#[tauri::command]
pub async fn sync_offerbook(state: tauri::State<'_, Arc<AppState>>) -> Result<(), AppError> {
    market::sync_offerbook(&state).await
}

#[tauri::command]
pub async fn poll_maker(state: tauri::State<'_, Arc<AppState>>, address: String) -> Result<MakerDto, AppError> {
    market::poll_maker(&state, address).await
}

#[tauri::command]
pub async fn remove_maker(state: tauri::State<'_, Arc<AppState>>, address: String) -> Result<bool, AppError> {
    market::remove_maker(&state, address).await
}

