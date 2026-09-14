//! Chain backend selection and probing.
//!
//! Bodies live in `portal_core::ops::chain_backend`; these wrappers register the operation with
//! Tauri and hand over the managed state.


use portal_core::error::AppError;
use portal_core::ops::chain_backend;
use portal_core::types::*;

#[tauri::command]
pub fn get_chain_backend() -> ChainBackendView {
    chain_backend::get_chain_backend()
}

#[tauri::command]
pub fn set_chain_backend(config: ChainBackendConfig) -> Result<(), AppError> {
    chain_backend::set_chain_backend(config)
}

#[tauri::command]
pub async fn check_backend(config: Option<ChainBackendConfig>, socks_port: Option<u16>) -> Result<BackendStatus, AppError> {
    chain_backend::check_backend(config, socks_port).await
}

