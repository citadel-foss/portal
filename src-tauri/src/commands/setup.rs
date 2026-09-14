//! Setup and connectivity commands.
//!
//! Bodies live in `portal_core::ops::setup`; these wrappers register the operation with
//! Tauri and hand over the managed state.


use portal_core::error::AppError;
use portal_core::ops::setup;
use portal_core::types::*;

#[tauri::command]
pub async fn check_tor() -> Result<TorStatus, AppError> {
    setup::check_tor().await
}

#[tauri::command]
pub async fn restart_tor_bootstrap() -> Result<(), AppError> {
    setup::restart_tor_bootstrap().await
}

