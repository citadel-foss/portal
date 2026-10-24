//! Wallet lifecycle and operation commands.
//!
//! Bodies live in `portal_core::ops::taker_wallet`, except the three that open a native
//! dialog: the picker and the main-window check are Tauri concerns, so those wrappers do that
//! part themselves and hand core the resulting path together with the guard they already hold.

use openswap::wallet::Wallet;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;
use uuid::Uuid;

use portal_core::error::{AppError, ErrorCode};
use portal_core::ops::taker_wallet;
use portal_core::security::input::validate_password;
use portal_core::security::operation::{SensitiveOperation, SensitiveOperationGuard};
use portal_core::state::{AppState, DESKTOP_SESSION};

use super::desktop_taker;
use portal_core::types::*;
use tauri_plugin_dialog::DialogExt;

use crate::native::ensure_main_window;

#[tauri::command]
pub fn list_wallets(
    data_dir: Option<String>,
) -> Result<Vec<portal_core::types::WalletListing>, AppError> {
    taker_wallet::list_wallets(data_dir)
}

#[tauri::command]
pub async fn init_taker(
    state: tauri::State<'_, Arc<AppState>>,
    config: InitConfig,
) -> Result<InitResult, AppError> {
    taker_wallet::init_taker(&state, DESKTOP_SESSION, config).await
}

/// Closes the wallet so another can be unlocked, without stopping the process. Returns at once:
/// the Taker finishes dropping in the background. Routers keep running — they are a separate
/// role and do not belong to this wallet session.
#[tauri::command]
pub fn shutdown_taker(state: tauri::State<'_, Arc<AppState>>) {
    taker_wallet::release_session(&state, DESKTOP_SESSION);
}

#[tauri::command]
pub fn get_paths(state: tauri::State<'_, Arc<AppState>>) -> Result<PathsDto, AppError> {
    taker_wallet::get_paths(&state, DESKTOP_SESSION)
}

#[tauri::command]
pub fn get_session_state(state: tauri::State<'_, Arc<AppState>>) -> SessionStateDto {
    taker_wallet::get_session_state(&state, DESKTOP_SESSION)
}

#[tauri::command]
pub fn get_wallet_info(state: tauri::State<'_, Arc<AppState>>) -> Result<WalletInfo, AppError> {
    taker_wallet::get_wallet_info(&*desktop_taker(&state)?)
}

#[tauri::command]
pub async fn choose_restore_backup(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<RestoreSelectionView, AppError> {
    ensure_main_window(&window)?;
    // Acquired before the picker opens and handed to core afterwards, so one guard covers the
    // dialog and the restore it authorizes.
    let operation = SensitiveOperationGuard::acquire(
        &state.sensitive_operation_active,
        SensitiveOperation::RestorePrivateKey,
    )?;
    let dialog_window = window.clone();
    let selected = tauri::async_runtime::spawn_blocking(move || {
        dialog_window
            .dialog()
            .file()
            .add_filter("Portal wallet backup", &["json"])
            .blocking_pick_file()
    })
    .await
    .map_err(AppError::internal)?
    .ok_or_else(|| AppError::user_cancelled("restore file selection was cancelled"))?;
    let path = selected.into_path().map_err(|_| {
        AppError::new(
            ErrorCode::InvalidFileSelection,
            "restore selection is not a local filesystem path",
        )
    })?;
    taker_wallet::register_restore_selection(&state, operation, path, false)
}

#[tauri::command]
pub async fn restore_wallet(
    state: tauri::State<'_, Arc<AppState>>,
    data_dir: Option<String>,
    wallet_name: String,
    socks_port: Option<u16>,
    selection_id: Uuid,
    password: String,
) -> Result<(), AppError> {
    taker_wallet::restore_wallet(
        &state,
        DESKTOP_SESSION,
        data_dir,
        wallet_name,
        socks_port,
        selection_id,
        password,
    )
    .await
}

#[tauri::command]
pub async fn restore_wallet_from_mnemonic(
    state: tauri::State<'_, Arc<AppState>>,
    data_dir: Option<String>,
    wallet_name: String,
    socks_port: Option<u16>,
    mnemonic: String,
    password: String,
) -> Result<(), AppError> {
    taker_wallet::restore_wallet_from_mnemonic(
        &state,
        DESKTOP_SESSION,
        data_dir,
        wallet_name,
        socks_port,
        mnemonic,
        password,
    )
    .await
}

#[tauri::command]
pub fn recovery_phrase_saved(state: tauri::State<'_, Arc<AppState>>) -> Result<bool, AppError> {
    Ok(taker_wallet::recovery_phrase_saved(&*desktop_taker(
        &state,
    )?))
}

#[tauri::command]
pub async fn reveal_recovery_phrase(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<AppState>>,
    wallet_password: String,
) -> Result<RecoveryPhraseDto, AppError> {
    ensure_main_window(&window)?;
    taker_wallet::reveal_recovery_phrase(&*desktop_taker(&state)?, wallet_password).await
}

#[tauri::command]
pub async fn confirm_recovery_phrase_saved(
    state: tauri::State<'_, Arc<AppState>>,
    wallet_password: String,
) -> Result<(), AppError> {
    taker_wallet::confirm_recovery_phrase_saved(&*desktop_taker(&state)?, wallet_password).await
}

#[tauri::command]
pub async fn backup_wallet(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<AppState>>,
    password: String,
    wallet_password: String,
) -> Result<String, AppError> {
    ensure_main_window(&window)?;
    validate_password(&password, "backup password")?;
    let wallet = desktop_taker(&state)?.wallet.clone();
    save_backup(
        window,
        &state,
        wallet,
        "portal-wallet-backup",
        password,
        wallet_password,
    )
    .await
}

/// Asks where to save, then writes the encrypted backup of `wallet` there. Shared by the wallet
/// and router backups.
pub(crate) async fn save_backup(
    window: tauri::WebviewWindow,
    state: &AppState,
    wallet: impl AsRef<RwLock<Wallet>> + Send + 'static,
    file_stem: &str,
    password: String,
    wallet_password: String,
) -> Result<String, AppError> {
    let operation = SensitiveOperationGuard::acquire(
        &state.sensitive_operation_active,
        SensitiveOperation::BackupPrivateKey,
    )?;
    let date = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let dialog_window = window.clone();
    let file_name = format!("{file_stem}-{date}.json");
    let selected = tauri::async_runtime::spawn_blocking(move || {
        dialog_window
            .dialog()
            .file()
            .set_file_name(file_name)
            .add_filter("JSON files", &["json"])
            .blocking_save_file()
    })
    .await
    .map_err(AppError::internal)?
    .ok_or_else(|| AppError::user_cancelled("backup destination selection was cancelled"))?;
    let destination = selected.into_path().map_err(|_| {
        AppError::new(
            ErrorCode::InvalidFileSelection,
            "backup destination is not a local filesystem path",
        )
    })?;
    taker_wallet::write_backup(wallet, operation, destination, password, wallet_password).await
}

#[tauri::command]
pub fn validate_address(address: String) -> AddressValidation {
    taker_wallet::validate_address(address)
}

#[tauri::command]
pub async fn get_balances(state: tauri::State<'_, Arc<AppState>>) -> Result<BalancesDto, AppError> {
    taker_wallet::get_balances(&*desktop_taker(&state)?).await
}

#[tauri::command]
pub async fn get_new_address(
    state: tauri::State<'_, Arc<AppState>>,
    address_type: AddressTypeDto,
) -> Result<NewAddress, AppError> {
    taker_wallet::get_new_address(&*desktop_taker(&state)?, address_type).await
}

#[tauri::command]
pub async fn get_transactions(
    state: tauri::State<'_, Arc<AppState>>,
    count: Option<usize>,
    skip: Option<usize>,
) -> Result<Vec<TxSummary>, AppError> {
    taker_wallet::get_transactions(&*desktop_taker(&state)?, count, skip).await
}

#[tauri::command]
pub async fn list_addresses(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<WalletAddressDto>, AppError> {
    taker_wallet::list_addresses(&*desktop_taker(&state)?).await
}

#[tauri::command]
pub async fn list_utxos(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<UtxoEntry>, AppError> {
    taker_wallet::list_utxos(&*desktop_taker(&state)?).await
}

#[tauri::command]
pub async fn send_to_address(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<AppState>>,
    address: String,
    amount_sats: u64,
    fee_rate: Option<f64>,
    outpoints: Option<Vec<Outpoint>>,
) -> Result<SendResult, AppError> {
    ensure_main_window(&window)?;
    taker_wallet::send_to_address(
        &state,
        &*desktop_taker(&state)?,
        address,
        amount_sats,
        fee_rate,
        outpoints,
    )
    .await
}

#[tauri::command]
pub async fn sync_wallet(state: tauri::State<'_, Arc<AppState>>) -> Result<(), AppError> {
    taker_wallet::sync_wallet(&*desktop_taker(&state)?).await
}

#[tauri::command]
pub async fn estimate_send_fee(
    state: tauri::State<'_, Arc<AppState>>,
    address: String,
    amount_sats: u64,
    fee_rate: Option<f64>,
    outpoints: Option<Vec<Outpoint>>,
) -> Result<portal_core::types::SendFeeEstimate, AppError> {
    taker_wallet::estimate_send_fee(
        &*desktop_taker(&state)?,
        address,
        amount_sats,
        fee_rate,
        outpoints,
    )
    .await
}

#[tauri::command]
pub async fn get_btc_price() -> Result<PriceEstimate, AppError> {
    taker_wallet::get_btc_price().await
}
