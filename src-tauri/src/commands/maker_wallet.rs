//! Router wallet commands.
//!
//! Bodies live in `portal_core::ops::maker_wallet`; these wrappers register the operation with
//! Tauri and hand over the managed state.

use std::sync::Arc;

use portal_core::error::AppError;
use portal_core::ops::maker_wallet;
use portal_core::state::AppState;
use portal_core::types::*;

#[tauri::command]
pub async fn get_maker_balances(state: tauri::State<'_, Arc<AppState>>, router_id: String) -> Result<BalancesDto, AppError> {
    maker_wallet::get_maker_balances(&state, router_id).await
}

#[tauri::command]
pub async fn list_maker_utxos(state: tauri::State<'_, Arc<AppState>>, router_id: String) -> Result<Vec<UtxoEntry>, AppError> {
    maker_wallet::list_maker_utxos(&state, router_id).await
}

#[tauri::command]
pub async fn get_maker_transactions(
    state: tauri::State<'_, Arc<AppState>>, router_id: String, count: Option<usize>, skip: Option<usize>,
) -> Result<Vec<TxSummary>, AppError> {
    maker_wallet::get_maker_transactions(&state, router_id, count, skip).await
}

#[tauri::command]
pub async fn get_maker_new_address(
    state: tauri::State<'_, Arc<AppState>>, router_id: String, address_type: AddressTypeDto,
) -> Result<NewAddress, AppError> {
    maker_wallet::get_maker_new_address(&state, router_id, address_type).await
}

#[tauri::command]
pub async fn send_maker_to_address(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<AppState>>,
    router_id: String,
    address: String,
    amount_sats: u64,
    fee_rate: Option<f64>,
    outpoints: Option<Vec<Outpoint>>,
) -> Result<SendResult, AppError> {
    crate::native::ensure_main_window(&window)?;
    maker_wallet::send_maker_to_address(&state, router_id, address, amount_sats, fee_rate, outpoints)
        .await
}

#[tauri::command]
pub async fn sync_maker_wallet(state: tauri::State<'_, Arc<AppState>>, router_id: String) -> Result<(), AppError> {
    maker_wallet::sync_maker_wallet(&state, router_id).await
}

#[tauri::command]
pub async fn list_maker_fidelity_bonds(state: tauri::State<'_, Arc<AppState>>, router_id: String) -> Result<Vec<FidelityBondDto>, AppError> {
    maker_wallet::list_maker_fidelity_bonds(&state, router_id).await
}

