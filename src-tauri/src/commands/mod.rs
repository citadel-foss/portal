pub mod auth;
pub mod chain_backend;
pub mod logs;
pub mod maker;
pub mod maker_reports;
pub mod maker_settings;
pub mod maker_wallet;
pub mod market;
pub mod setup;
pub mod shutdown;
pub mod taker_reports;
pub mod taker_swap;
pub mod taker_wallet;

use std::sync::Arc;

use portal_core::error::AppError;
use portal_core::state::{AppState, TakerInstance, DESKTOP_SESSION};

/// The desktop has one window and so one session; every wallet command acts on its wallet.
pub(crate) fn desktop_taker(state: &AppState) -> Result<Arc<TakerInstance>, AppError> {
    state.taker_for(DESKTOP_SESSION)
}
