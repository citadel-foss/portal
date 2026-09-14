//! Router settings and registration commands.
//!
//! Bodies live in `portal_core::ops::maker_settings`; these wrappers register the operation with
//! Tauri and hand over the managed state.

use std::sync::Arc;

use portal_core::error::AppError;
use portal_core::ops::maker_settings;
use portal_core::state::AppState;
use portal_core::types::*;

#[tauri::command]
pub fn list_makers() -> Result<Vec<MakerSettingsDto>, AppError> {
    maker_settings::list_makers()
}

#[tauri::command]
pub fn get_saved_maker_settings(router_id: String) -> Result<Option<MakerSettingsDto>, AppError> {
    maker_settings::get_saved_maker_settings(router_id)
}

#[tauri::command]
pub fn list_dashboard_imports() -> Result<Vec<MakerSettingsDto>, AppError> {
    maker_settings::list_dashboard_imports()
}

#[tauri::command]
pub fn import_dashboard_makers(router_ids: Vec<String>) -> Result<Vec<MakerSettingsDto>, AppError> {
    maker_settings::import_dashboard_makers(router_ids)
}

#[tauri::command]
pub fn clear_maker_settings(state: tauri::State<'_, Arc<AppState>>, router_id: String) -> Result<(), AppError> {
    maker_settings::clear_maker_settings(&state, router_id)
}

#[tauri::command]
pub fn get_suggested_maker_ports() -> Result<SuggestedMakerPortsDto, AppError> {
    maker_settings::get_suggested_maker_ports()
}

#[tauri::command]
pub fn check_maker_ports(network_port: u16, rpc_port: u16) -> Result<MakerPortCheckDto, AppError> {
    maker_settings::check_maker_ports(network_port, rpc_port)
}

