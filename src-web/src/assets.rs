//! Static frontend, served from a directory beside the binary.
//!
//! Only the allowlisted directory is served, and an API path never falls back to `index.html`
//! — a mistyped route must 404 rather than hand back an HTML page a client will try to parse
//! as JSON.

use axum::http::{header, HeaderValue};
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::state::WebState;

pub fn router(state: &WebState) -> Router<WebState> {
    // Nothing to serve in development: Vite owns the UI and only proxies the API here.
    let Some(dir) = state.config.assets_dir.as_ref() else {
        return Router::new();
    };
    let index = dir.join("index.html");
    Router::new().fallback_service(
        ServeDir::new(dir)
            // A single-page app serves its shell for unknown *page* paths; API routes are
            // matched earlier, so they never reach this.
            .not_found_service(ServeFile::new(index)),
    )
    // No private API response or backup byte may be cached; the shell is versioned with the
    // build, so it must not be reused across incompatible releases either.
    .layer(SetResponseHeaderLayer::overriding(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    ))
}
