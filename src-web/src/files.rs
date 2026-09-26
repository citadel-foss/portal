//! Encrypted wallet backup in and out of the browser.
//!
//! There is no native picker here and no server path ever crosses the wire. An upload is
//! written to a server-chosen private file and handed back as an opaque, single-use,
//! session-bound ID; a download is an authenticated attachment, never a bearer URL.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use portal_core::error::{AppError, ErrorCode};
use portal_core::security::operation::{SensitiveOperation, SensitiveOperationGuard};
use serde_json::json;

use crate::routes::{authenticate, check_csrf, check_origin, ApiError};
use crate::state::WebState;

/// A wallet backup is a small JSON document. The cap is a starting bound, not a measured
/// maximum: a representative fixture must fit, or this moves before release.
pub const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;

pub async fn upload(
    State(state): State<WebState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    check_origin(&state, &headers)?;
    let caller = authenticate(&state, &headers)?;
    check_csrf(&caller, &headers)?;

    if body.len() > MAX_UPLOAD_BYTES {
        return Err(ApiError(
            StatusCode::PAYLOAD_TOO_LARGE,
            AppError::new(ErrorCode::InvalidInput, "backup upload is too large"),
        ));
    }
    // Parsed before anything is registered: an unparseable body is not a wallet backup, and
    // finding that out at restore time would waste the user's password attempt.
    if serde_json::from_slice::<serde_json::Value>(&body).is_err() {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            AppError::new(ErrorCode::InvalidFileSelection, "upload is not valid JSON"),
        ));
    }

    // Expired leftovers go now rather than accumulating in a private directory forever.
    portal_core::storage::sweep_stale_transfers(&state.data_root, std::time::Duration::from_secs(300));

    let guard = SensitiveOperationGuard::acquire(
        &state.runtime.sensitive_operation_active,
        SensitiveOperation::RestorePrivateKey,
    )?;
    let path = portal_core::storage::stage_private_file(&state.data_root, &body)?;
    let view = portal_core::ops::taker_wallet::register_restore_selection(
        &state.runtime,
        guard,
        path,
    )?;
    Ok((StatusCode::OK, Json(serde_json::to_value(view).unwrap_or(json!({})))).into_response())
}

/// Produces the password-encrypted backup into private staging and returns its ID. The ID is
/// an identifier, not authorization: downloading it still needs the session and a CSRF token.
pub async fn create_backup(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(body): Json<BackupBody>,
) -> Result<Response, ApiError> {
    check_origin(&state, &headers)?;
    let caller = authenticate(&state, &headers)?;
    check_csrf(&caller, &headers)?;

    // The desktop wrapper enforces this before opening its save dialog; a backup reachable
    // over HTTP must clear the same floor rather than a weaker one.
    portal_core::security::input::validate_password(&body.password, "backup password")?;
    let taker = state.runtime.taker_for(&caller.session)?;
    let guard = SensitiveOperationGuard::acquire(
        &state.runtime.sensitive_operation_active,
        SensitiveOperation::BackupPrivateKey,
    )?;
    let id = uuid::Uuid::new_v4().to_string();
    let destination = state
        .data_root
        .join("portal")
        .join("transfers")
        .join(format!("{id}.backup.json"));
    portal_core::security::fs::ensure_private_dir(
        destination.parent().expect("transfers has a parent"),
    )?;
    portal_core::ops::taker_wallet::write_backup(
        &taker,
        guard,
        destination,
        body.password,
    )
    .await?;
    Ok((StatusCode::OK, Json(json!({ "artifactId": id }))).into_response())
}

#[derive(serde::Deserialize)]
pub struct BackupBody {
    pub password: String,
}

/// Streams the artifact once. Marked consumed on the delivery attempt and deleted afterwards,
/// so a stale link cannot be replayed; a failed download is regenerated deliberately.
pub async fn download_backup(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    check_origin(&state, &headers)?;
    let caller = authenticate(&state, &headers)?;
    check_csrf(&caller, &headers)?;

    // The id names a file we created, so it must be a UUID and nothing else — never a path.
    if uuid::Uuid::parse_str(&id).is_err() {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            AppError::new(ErrorCode::InvalidInput, "no such artifact"),
        ));
    }
    let path = state
        .data_root
        .join("portal")
        .join("transfers")
        .join(format!("{id}.backup.json"));
    let bytes = tokio::fs::read(&path).await.map_err(|_| {
        ApiError(
            StatusCode::NOT_FOUND,
            AppError::new(ErrorCode::InvalidInput, "no such artifact"),
        )
    })?;
    let _ = tokio::fs::remove_file(&path).await;

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"portal-wallet-backup.json\"",
            ),
            (header::CACHE_CONTROL, "no-store"),
        ],
        bytes,
    )
        .into_response())
}
