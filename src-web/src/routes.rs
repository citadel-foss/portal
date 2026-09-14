//! HTTP surface. Every route here is explicit: there is no reflective dispatcher, so a name
//! the inventory does not carry cannot reach core no matter what a body contains.

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use tower_http::limit::RequestBodyLimitLayer;
use portal_core::error::{AppError, ErrorCode};
use serde_json::{json, Value};

use crate::auth::SESSION_COOKIE;
use crate::commands;
use crate::state::WebState;

/// Serializes an `AppError` into the envelope the frontend already parses, so a web failure
/// reaches `isAppError` in exactly the shape a desktop one does.
pub struct ApiError(pub StatusCode, pub AppError);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let ApiError(status, error) = self;
        let body = json!({
            "error": {
                "code": error.code,
                "message": error.message,
                "details": error.details,
            }
        });
        (status, Json(body)).into_response()
    }
}

impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        let status = match error.code {
            ErrorCode::NotInitialized | ErrorCode::InvalidInput => StatusCode::BAD_REQUEST,
            ErrorCode::AuthorizationDenied => StatusCode::FORBIDDEN,
            ErrorCode::SwapInProgress
            | ErrorCode::MakerBusy
            | ErrorCode::SensitiveOperationInProgress => StatusCode::CONFLICT,
            ErrorCode::RpcUnreachable | ErrorCode::TorUnreachable => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(status, error)
    }
}

/// Carries whether an owner exists so an unauthenticated page can tell "claim this
/// installation" from "sign in" without probing the login endpoint and burning its throttle.
/// Not sensitive: the setup page is public, and it says the same thing.
fn unauthorized(state: &WebState) -> ApiError {
    let mut error = AppError::new(ErrorCode::AuthorizationDenied, "authentication required");
    error.details = Some(serde_json::json!({ "hasOwner": state.auth.has_owner() }));
    ApiError(StatusCode::UNAUTHORIZED, error)
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_string())
}

/// A session plus the CSRF token bound to it.
pub(crate) struct Caller {
    pub(crate) csrf: String,
}

/// `refresh` distinguishes operator activity from background polling: only the former should
/// push the idle deadline out.
pub(crate) fn authenticate(state: &WebState, headers: &HeaderMap, refresh: bool) -> Result<Caller, ApiError> {
    if state.open_local() {
        return Ok(Caller { csrf: String::new() });
    }
    let token = cookie(headers, SESSION_COOKIE).ok_or_else(|| unauthorized(state))?;
    let csrf = state
        .auth
        .validate(&token, refresh)
        .ok_or_else(|| unauthorized(state))?;
    Ok(Caller { csrf })
}

/// Same-origin enforcement. A browser always sends `Origin` on a cross-origin request, so a
/// mismatch is rejected; CORS stays off entirely rather than being widened for convenience.
pub(crate) fn check_origin(state: &WebState, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(expected) = state.config.public_origin.as_deref() else {
        return Ok(());
    };
    match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        Some(origin) if origin == expected => Ok(()),
        None => Ok(()),
        Some(_) => Err(ApiError(
            StatusCode::FORBIDDEN,
            AppError::new(ErrorCode::AuthorizationDenied, "cross-origin request refused"),
        )),
    }
}

pub(crate) fn check_csrf(caller: &Caller, headers: &HeaderMap) -> Result<(), ApiError> {
    // Nothing to forge a request against when there is no session to ride on.
    if caller.csrf.is_empty() {
        return Ok(());
    }
    let presented = headers.get("x-csrf-token").and_then(|v| v.to_str().ok());
    if presented == Some(caller.csrf.as_str()) {
        return Ok(());
    }
    Err(ApiError(
        StatusCode::FORBIDDEN,
        AppError::new(ErrorCode::AuthorizationDenied, "missing or stale CSRF token"),
    ))
}

fn session_cookie(state: &WebState, token: &str) -> String {
    let base = state.config.base_path.trim_end_matches('/');
    let path = if base.is_empty() { "/" } else { base };
    let secure = if state.config.secure_cookies() { "; Secure" } else { "" };
    // Host-only (no Domain), so a sibling host on the same registrable domain cannot read it.
    format!("{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path={path}{secure}")
}

pub fn router(state: WebState) -> Router {
    let c = state.config.clone();
    Router::new()
        .route(&c.route("/api/v1/auth/bootstrap"), post(bootstrap))
        .route(&c.route("/api/v1/auth/login"), post(login))
        .route(&c.route("/api/v1/auth/logout"), post(logout))
        .route(&c.route("/api/v1/session"), get(session))
        .route(&c.route("/api/v1/events"), get(events))
        .route(&c.route("/api/v1/commands/{name}"), post(command))
        .route(&c.route("/api/v1/uploads"), post(crate::files::upload))
        .route(&c.route("/api/v1/backups"), post(crate::files::create_backup))
        .route(
            &c.route("/api/v1/backups/{id}/download"),
            post(crate::files::download_backup),
        )
        .route(&c.route("/api/v1/operations"), get(operations))
        .route(&c.route("/api/v1/operations/{id}"), get(operation))
        .route(&c.route("/health/live"), get(live))
        .route(&c.route("/health/ready"), get(ready))
        .layer(RequestBodyLimitLayer::new(crate::files::MAX_UPLOAD_BYTES))
        .with_state(state)
}

/// Public and side-effect free by design: a supervisor probe must never start Tor, touch a
/// wallet lock or reach a configured node.
async fn live() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "status": "live" })))
}

/// Ready means this service is initialized and its storage is usable. A locked wallet or an
/// unreachable chain backend is a normal operating state, not an unhealthy container.
async fn ready(State(state): State<WebState>) -> impl IntoResponse {
    let ready = state.storage_ready();
    let status = if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (status, Json(json!({ "status": if ready { "ready" } else { "unavailable" } })))
}

#[derive(serde::Deserialize)]
struct BootstrapBody {
    secret: String,
    password: String,
}

async fn bootstrap(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(body): Json<BootstrapBody>,
) -> Result<Response, ApiError> {
    check_origin(&state, &headers)?;
    state.auth.bootstrap(&body.secret, &body.password).map_err(|message| {
        ApiError(
            StatusCode::FORBIDDEN,
            AppError::new(ErrorCode::AuthorizationDenied, message),
        )
    })?;
    Ok((StatusCode::OK, Json(json!({ "status": "claimed" }))).into_response())
}

#[derive(serde::Deserialize)]
struct LoginBody {
    password: String,
}

async fn login(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> Result<Response, ApiError> {
    check_origin(&state, &headers)?;
    let issued = state.auth.login(&body.password).map_err(|message| {
        ApiError(
            StatusCode::UNAUTHORIZED,
            AppError::new(ErrorCode::AuthorizationDenied, message),
        )
    })?;
    let mut response = Json(json!({ "csrfToken": issued.csrf })).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie(&state, &issued.token).parse().expect("cookie is ascii"),
    );
    Ok(response)
}

async fn logout(State(state): State<WebState>, headers: HeaderMap) -> Result<Response, ApiError> {
    // Accepted work keeps running: logging out ends a viewing session, not a swap.
    if let Some(token) = cookie(&headers, SESSION_COOKIE) {
        state.auth.logout(&token);
    }
    let mut response = Json(json!({ "status": "ended" })).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        // Same Path as it was set with: a cookie cleared at "/" leaves the one scoped to a
        // deployment's base path in place.
        {
            let base = state.config.base_path.trim_end_matches('/');
            let path = if base.is_empty() { "/" } else { base };
            format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path={path}; Max-Age=0")
                .parse()
                .expect("cookie is ascii")
        },
    );
    Ok(response)
}

async fn session(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let caller = authenticate(&state, &headers, true)?;
    Ok(Json(json!({
        "installationId": state.installation_id,
        "runtimeId": state.runtime_id,
        "csrfToken": caller.csrf,
        "capabilities": {
            "nativeFilePicker": false,
            "canQuit": false,
        },
        // Tells the UI whether to show a login gate at all, so a local run looks exactly
        // like the desktop app.
        "requiresLogin": !state.open_local(),
        "storageLabel": state.storage_label(),
        "hasOwner": state.auth.has_owner(),
    })))
}

async fn events(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    // A stream must not keep a session alive on heartbeats alone, so this read does not
    // refresh the idle deadline.
    authenticate(&state, &headers, false)?;
    Ok(crate::sse::stream(state.runtime.events.subscribe()).into_response())
}

async fn command(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    body: Option<Json<Value>>,
) -> Result<Response, ApiError> {
    check_origin(&state, &headers)?;
    let caller = authenticate(&state, &headers, true)?;
    let args = body.map(|Json(v)| v).unwrap_or(Value::Null);

    if let Some(operation) = commands::lookup_durable(&name) {
        return durable(&state, &caller, &headers, &name, operation, args).await;
    }
    let Some(operation) = commands::lookup(&name) else {
        // Unknown or desktop-only: a 404 rather than a fall-through into arbitrary dispatch.
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            AppError::new(ErrorCode::InvalidInput, "no such operation on this host"),
        ));
    };
    if operation.mutates {
        check_csrf(&caller, &headers)?;
    }
    let result = (operation.run)(state.runtime.clone(), args).await?;
    Ok(Json(result).into_response())
}

/// Durable submission. Acceptance is persisted before the worker starts and before this
/// returns, so a response lost in transit can be reconciled with the key the client already
/// has rather than resubmitted blind.
async fn durable(
    state: &WebState,
    caller: &Caller,
    headers: &HeaderMap,
    name: &str,
    operation: &'static crate::commands::Operation,
    args: Value,
) -> Result<Response, ApiError> {
    check_csrf(caller, headers)?;
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            ApiError(
                StatusCode::BAD_REQUEST,
                AppError::new(
                    ErrorCode::InvalidInput,
                    "durable operations require a client-generated Idempotency-Key",
                ),
            )
        })?
        .to_string();

    // An operation whose effect could not be proven may already have moved funds. Starting
    // another that could conflict is exactly how the same coins get spent twice, so new
    // financial work waits until the earlier one is settled. Reading an existing key is
    // still allowed below — that is how it gets settled.
    if state.journal.has_unresolved() && state.journal.get(&key).is_none() {
        return Err(ApiError(
            StatusCode::CONFLICT,
            AppError::new(
                ErrorCode::SwapInProgress,
                "An earlier operation's outcome is still unresolved. Check it before starting \
                 new work — see the recovery page.",
            ),
        ));
    }

    match state.journal.admit(&key, name, None, 0, &args)? {
        portal_core::operations::Admission::Replayed(record) => {
            // Not an error: this is the answer the client came back for.
            Ok((StatusCode::OK, Json(serde_json::to_value(record).unwrap_or(Value::Null)))
                .into_response())
        }
        portal_core::operations::Admission::Accepted(record) => {
            let runtime = state.runtime.clone();
            let journal = state.journal.clone();
            let run = operation.run;
            let id = record.operation_id.clone();
            // Owned by the server from here: closing the tab or losing the connection does
            // not cancel work that has already been accepted.
            tokio::spawn(async move {
                let _ = journal.mark_running(&id);
                match run(runtime, args).await {
                    Ok(value) => {
                        let _ = journal.mark_succeeded(&id, value);
                    }
                    // Only a definite failure is recorded as failed. Anything that could have
                    // taken effect before the error stays indeterminate and blocks conflicting
                    // spends until it is settled.
                    Err(error) => {
                        let settled = matches!(
                            error.code,
                            ErrorCode::InvalidInput
                                | ErrorCode::InsufficientFunds
                                | ErrorCode::WalletWrongPassword
                                | ErrorCode::NotInitialized
                        );
                        let _ = if settled {
                            journal.mark_failed(&id, error)
                        } else {
                            journal.mark_indeterminate(&id, error)
                        };
                    }
                }
            });
            Ok((
                StatusCode::ACCEPTED,
                Json(json!({ "operationId": record.operation_id, "state": "accepted" })),
            )
                .into_response())
        }
    }
}

async fn operations(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    authenticate(&state, &headers, true)?;
    let recent = state.journal.recent(100);
    Ok(Json(json!({
        "operations": recent,
        "hasUnresolved": state.journal.has_unresolved(),
    })))
}

async fn operation(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    authenticate(&state, &headers, true)?;
    let record = state.journal.get(&id).ok_or_else(|| {
        // No durable acceptance record exists under this key. The client should reconcile its
        // view before deliberately resubmitting, keeping the same key.
        ApiError(
            StatusCode::NOT_FOUND,
            AppError::new(ErrorCode::InvalidInput, "no operation with that id"),
        )
    })?;
    Ok(Json(serde_json::to_value(record).map_err(AppError::internal)?))
}
