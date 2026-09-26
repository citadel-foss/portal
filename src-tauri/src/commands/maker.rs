//! Router lifecycle commands.
//!
//! Bodies live in `portal_core::ops::maker`. The managed runtime replaces the `AppHandle`
//! these once took: core reaches the registry and publishes phase events through it, without
//! knowing a webview exists.

use std::sync::Arc;

use portal_core::error::AppError;
use portal_core::ops::maker;
use portal_core::state::{AppState, DESKTOP_SESSION};
use portal_core::types::{MakerInitConfig, MakerSettingsDto, MakerStatusDto, RouterDefaultsDto, WalletInfo};

#[tauri::command]
pub fn get_router_defaults() -> RouterDefaultsDto {
    maker::router_defaults()
}

#[tauri::command]
pub async fn init_maker(
    state: tauri::State<'_, Arc<AppState>>,
    config: MakerInitConfig,
) -> Result<MakerStatusDto, AppError> {
    maker::init_maker(&state, DESKTOP_SESSION, config).await
}

#[tauri::command]
pub fn update_maker_settings(
    state: tauri::State<'_, Arc<AppState>>,
    router_id: String,
    settings: MakerSettingsDto,
) -> Result<MakerSettingsDto, AppError> {
    maker::update_maker_settings(&state, router_id, settings)
}

#[tauri::command]
pub async fn start_maker(
    state: tauri::State<'_, Arc<AppState>>,
    router_id: String,
    wallet_password: Option<String>,
) -> Result<(), AppError> {
    maker::start_maker(&state, DESKTOP_SESSION, router_id, wallet_password).await
}

#[tauri::command]
pub async fn stop_maker(
    state: tauri::State<'_, Arc<AppState>>,
    router_id: String,
) -> Result<(), AppError> {
    maker::stop_maker(&state, router_id).await
}

#[tauri::command]
pub fn get_maker_status(
    state: tauri::State<'_, Arc<AppState>>,
    router_id: String,
) -> Result<MakerStatusDto, AppError> {
    maker::get_maker_status(&state, router_id)
}

#[tauri::command]
pub fn get_maker_info(
    state: tauri::State<'_, Arc<AppState>>,
    router_id: String,
) -> Result<WalletInfo, AppError> {
    maker::get_maker_info(&state, router_id)
}
