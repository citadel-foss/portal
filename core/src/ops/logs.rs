//! Tail of each role's `debug.log`, written by our own dual-role logger
//! (see `logging.rs`, wired up in `commands::taker_wallet::init_taker` /
//! `commands::maker::init_maker`). One function per role, not worth two
//! files for — both are a three-line tail against a different `AppState`
//! field.

use std::sync::Arc;

use crate::error::AppError;
use crate::state::AppState;
use crate::types::LogLine;

pub async fn get_logs(
    state: &Arc<AppState>,
    lines: Option<usize>,
) -> Result<Vec<LogLine>, AppError> {
    let data_dir = state
        .data_dir
        .read()?
        .clone()
        .ok_or_else(AppError::not_initialized)?;
    let path = data_dir.join("debug.log");
    let want = lines.unwrap_or(100).min(1000);

    tokio::task::spawn_blocking(move || -> Result<Vec<LogLine>, AppError> {
        Ok(crate::logging::tail_lines(&path, want)?
            .into_iter()
            .map(|line| LogLine { line })
            .collect())
    })
    .await
    .map_err(AppError::internal)?
}

pub async fn get_maker_logs(
    state: &Arc<AppState>,
    router_id: String,
    lines: Option<usize>,
) -> Result<Vec<LogLine>, AppError> {
    let data_dir = {
        let makers = state.makers.lock()?;
        let in_memory = makers.get(&router_id).map(|entry| entry.settings.clone());
        drop(makers);
        let settings = match in_memory {
            Some(settings) => settings,
            None => crate::ops::maker_settings::load(&router_id)?
                .ok_or_else(|| AppError::maker_not_found(&router_id))?,
        };
        settings
            .data_dir
            .as_deref()
            .map(std::path::PathBuf::from)
            .ok_or_else(AppError::maker_not_initialized)?
    };
    let path = data_dir.join("debug.log");
    let want = lines.unwrap_or(100).min(1000);

    tokio::task::spawn_blocking(move || -> Result<Vec<LogLine>, AppError> {
        Ok(crate::logging::tail_lines(&path, want)?
            .into_iter()
            .map(|line| LogLine { line })
            .collect())
    })
    .await
    .map_err(AppError::internal)?
}
