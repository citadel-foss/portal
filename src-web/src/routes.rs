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
    pub(crate) session: String,
}

/// Every request carries a session earned by logging in. Sessions are not exclusive: any
/// number of browsers, and the desktop app, may hold one at the same time.
pub(crate) fn authenticate(state: &WebState, headers: &HeaderMap) -> Result<Caller, ApiError> {
    let token = cookie(headers, SESSION_COOKIE).ok_or_else(|| unauthorized(state))?;
    let session = state
        .auth
        .validate(&token)
        .ok_or_else(|| unauthorized(state))?;
    Ok(Caller {
        csrf: session.csrf,
        session: session.id,
    })
}

/// Same-origin enforcement. A browser always sends `Origin` on a cross-origin request, so a
/// mismatch is rejected; CORS stays off entirely rather than being widened for convenience.
/// True for an origin served from this machine, whatever port it uses.
///
/// The port cannot be pinned down: in development the page is on Vite's port and the API is
/// proxied, so the browser sends `http://localhost:1430` to a server bound to 3000. The host
/// is the part that matters — a page from somewhere else on the internet can never present
/// one of these. An opaque `null` origin (a sandboxed frame) is not one of them.
fn is_loopback_origin(origin: &str) -> bool {
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let host = match rest.strip_prefix('[') {
        // IPv6 literal: everything up to the closing bracket, port or not.
        Some(after) => match after.split_once(']') {
            Some((inside, tail)) if tail.is_empty() || tail.starts_with(':') => inside,
            _ => return false,
        },
        None => rest.split(':').next().unwrap_or(""),
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

pub(crate) fn check_origin(state: &WebState, headers: &HeaderMap) -> Result<(), ApiError> {
    let refused = || {
        ApiError(
            StatusCode::FORBIDDEN,
            AppError::new(ErrorCode::AuthorizationDenied, "cross-origin request refused"),
        )
    };
    // No configured origin means a local run, where the port the page is served from is not
    // knowable here. Browsers attach `Origin` to every POST, same-origin ones included, so
    // its mere presence proves nothing — but a hostile page on the public internet cannot
    // forge a loopback one, which is the attack this closes.
    let Some(expected) = state.config.public_origin.as_deref() else {
        return match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
            None => Ok(()),
            Some(origin) if is_loopback_origin(origin) => Ok(()),
            Some(_) => Err(refused()),
        };
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
        .route(&c.route("/api/v1/auth/claim"), post(claim))
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
        .route(
            &c.route("/api/v1/operations/{id}/reconcile"),
            post(reconcile_operation),
        )
        .route(
            &c.route("/api/v1/operations/{id}/acknowledge"),
            post(acknowledge_operation),
        )
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
struct ClaimBody {
    password: String,
}

async fn claim(
    State(state): State<WebState>,
    headers: HeaderMap,
    Json(body): Json<ClaimBody>,
) -> Result<Response, ApiError> {
    check_origin(&state, &headers)?;
    state.auth.claim(&body.password).map_err(|message| {
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
    // Accepted work keeps running: logging out takes this browser off its wallet, and the
    // wallet closes only if no other browser is on it and no swap is running.
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

/// Where a returning browser picks its session back up. Without one this is a 401, which is
/// what sends the page to the login screen.
async fn session(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let caller = authenticate(&state, &headers)?;
    let body = Json(json!({
        "installationId": state.installation_id,
        "runtimeId": state.runtime_id,
        "csrfToken": caller.csrf,
        "capabilities": {
            "nativeFilePicker": false,
            "canQuit": false,
        },
        "storageLabel": state.storage_label(),
        "hasOwner": state.auth.has_owner(),
    }));
    Ok(body.into_response())
}

async fn events(
    State(state): State<WebState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let token = cookie(&headers, SESSION_COOKIE).ok_or_else(|| unauthorized(&state))?;
    // Held for the life of the stream: it is how the session knows this tab is still open.
    let guard = state.auth.open_stream(&token).ok_or_else(|| unauthorized(&state))?;
    Ok(crate::sse::stream(state.runtime.clone(), guard).into_response())
}

async fn command(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    body: Option<Json<Value>>,
) -> Result<Response, ApiError> {
    check_origin(&state, &headers)?;
    let caller = authenticate(&state, &headers)?;
    let args = body.map(|Json(v)| v).unwrap_or(Value::Null);

    let outcome = if let Some(operation) = commands::lookup_durable(&name) {
        durable(&state, &caller, &headers, &name, operation, args).await
    } else if let Some(operation) = commands::lookup(&name) {
        if operation.mutates {
            check_csrf(&caller, &headers)?;
        }
        (operation.run)(state.ctx(&caller), args)
            .await
            .map(|result| Json(result).into_response())
            .map_err(ApiError::from)
    } else {
        // Unknown or desktop-only: a 404 rather than a fall-through into arbitrary dispatch.
        Err(ApiError(
            StatusCode::NOT_FOUND,
            AppError::new(ErrorCode::InvalidInput, "no such operation on this host"),
        ))
    };

    outcome
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

    // A spend whose effect could not be proven may already have moved coins, so another
    // spend could double it. Scoped deliberately: only fund-moving work is held, and only by
    // other fund-moving work. Recovery is never held — it is the remedy for a stuck swap, and
    // blocking it would strand the funds it exists to reclaim. Reading an existing key is
    // still allowed, since that is how one gets settled.
    // Per wallet: a payment stuck on one wallet cannot double-spend another wallet's coins.
    let wallet_id = state
        .runtime
        .wallet_of(&caller.session)
        .map(|dir| dir.display().to_string());
    if portal_core::operations::moves_funds(name) && state.journal.get(&key).is_none() {
        let blocking = state.journal.blocking_conflicts(wallet_id.as_deref());
        if !blocking.is_empty() {
            let mut error = AppError::new(
                ErrorCode::SwapInProgress,
                "An earlier payment's outcome could not be confirmed. Check it before \
                 spending again.",
            );
            error.details = Some(json!({
                "blocking": blocking.iter().map(|r| &r.operation_id).collect::<Vec<_>>(),
            }));
            return Err(ApiError(StatusCode::CONFLICT, error));
        }
    }

    match state.journal.admit(&key, name, wallet_id, 0, &args)? {
        portal_core::operations::Admission::Replayed(record) => {
            // Not an error: this is the answer the client came back for.
            Ok((StatusCode::OK, Json(serde_json::to_value(record).unwrap_or(Value::Null)))
                .into_response())
        }
        portal_core::operations::Admission::Accepted(record) => {
            let ctx = state.ctx(caller);
            let journal = state.journal.clone();
            let run = operation.run;
            let id = record.operation_id.clone();
            // Owned by the server from here: closing the tab or losing the connection does
            // not cancel work that has already been accepted.
            tokio::spawn(async move {
                // Must be the first thing here, and nothing side-effecting may precede it:
                // `Journal::open` treats a record still in `Accepted` as proof the operation
                // never ran, which is what keeps a crash from blocking future spends. So if
                // this write fails, the work must not start either — running it anyway would
                // spend against a record that recovery is entitled to read as unexecuted, and
                // a later retry would move the funds a second time.
                if let Err(e) = journal.mark_running(&id) {
                    log::error!("refusing to start {id}: could not record it as running: {e:?}");
                    return;
                }
                match run(ctx, args).await {
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
                                | ErrorCode::WalletNetworkMismatch
                                | ErrorCode::WalletOpenElsewhere
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
    let caller = authenticate(&state, &headers)?;
    let wallet_id = state
        .runtime
        .wallet_of(&caller.session)
        .map(|dir| dir.display().to_string());
    let recent = state.journal.recent(100);
    Ok(Json(json!({
        "operations": recent,
        "blockingConflicts": state.journal.blocking_conflicts(wallet_id.as_deref()),
    })))
}

/// Re-reads chain evidence for one unresolved operation. Can only ever settle it as
/// succeeded — nothing here can prove a broadcast did not happen.
async fn reconcile_operation(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    check_origin(&state, &headers)?;
    let caller = authenticate(&state, &headers)?;
    check_csrf(&caller, &headers)?;

    // The same evidence the wallet page reconciles pending sends against.
    let known: Vec<String> = match state.runtime.taker_for(&caller.session) {
        Ok(taker) => portal_core::ops::taker_wallet::get_transactions(&taker, Some(200), None)
            .await
            .map(|txs| txs.into_iter().map(|tx| tx.txid).collect())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    let record = state.journal.reconcile(&id, &known)?;
    Ok(Json(serde_json::to_value(record).map_err(AppError::internal)?))
}

/// Records that the owner accepts an outcome that cannot be proven, so it stops holding up
/// new spending. The operation's state is unchanged: it is still unknown.
async fn acknowledge_operation(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    check_origin(&state, &headers)?;
    let caller = authenticate(&state, &headers)?;
    check_csrf(&caller, &headers)?;
    let record = state.journal.acknowledge(&id)?;
    Ok(Json(serde_json::to_value(record).map_err(AppError::internal)?))
}

async fn operation(
    State(state): State<WebState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    authenticate(&state, &headers)?;
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

#[cfg(test)]
mod tests {
    use super::is_loopback_origin;

    /// Every one of these is a real browser `Origin` this app receives. Browsers attach the
    /// header to same-origin POSTs too, so treating its presence as proof of a cross-site
    /// request breaks the whole app — which is exactly what happened once.
    #[test]
    fn the_app_talking_to_itself_is_not_cross_origin() {
        for origin in [
            "http://127.0.0.1:3000",     // web:start, opened directly
            "http://localhost:1430",     // web:dev, page on Vite proxying to 3000
            "http://localhost:3000",
            "http://[::1]:3000",
            "https://localhost:8443",
            "http://127.0.0.1",          // default port
        ] {
            assert!(is_loopback_origin(origin), "must be allowed: {origin}");
        }
    }

    #[test]
    fn a_page_from_anywhere_else_is_refused() {
        for origin in [
            "http://evil.example",
            "https://evil.example:3000",
            "null",                          // sandboxed frame
            "http://127.0.0.1.evil.example", // suffix trick
            "http://localhost.evil.example",
            "http://[::1].evil.example",
            "file://",
            "",
        ] {
            assert!(!is_loopback_origin(origin), "must be refused: {origin}");
        }
    }
}
