//! Static frontend, served from a directory beside the binary, or else from the copy built into
//! the binary itself (see `build.rs`).
//!
//! Only the allowlisted directory is served, and an API path never falls back to `index.html`
//! — a mistyped route must 404 rather than hand back an HTML page a client will try to parse
//! as JSON.

use axum::http::{header, HeaderValue, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::state::WebState;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_web.rs"));
}

/// Whether this binary carries its own frontend.
pub fn has_embedded() -> bool {
    !embedded::FILES.is_empty()
}

pub(crate) fn no_store() -> SetResponseHeaderLayer<HeaderValue> {
    // No private API response or backup byte may be cached; the shell is versioned with the
    // build, so it must not be reused across incompatible releases either.
    SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))
}

pub fn router(state: &WebState) -> Router<WebState> {
    let Some(dir) = state.config.assets_dir.as_ref() else {
        // Nothing to serve in development: Vite owns the UI and only proxies the API here.
        if !has_embedded() {
            return Router::new();
        }
        return Router::new().fallback(serve_embedded).layer(no_store());
    };
    let index = dir.join("index.html");
    Router::new().fallback_service(
        ServeDir::new(dir)
            // A single-page app serves its shell for unknown *page* paths; API routes are
            // matched earlier, so they never reach this.
            .not_found_service(ServeFile::new(index)),
    )
    .layer(no_store())
}

/// The embedded counterpart of the directory service above, with the same single-page fallback:
/// an unknown path gets the shell, and API routes were matched before this is reached.
async fn serve_embedded(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file = |name: &str| embedded::FILES.iter().find(|(route, _)| *route == name);
    let Some((route, bytes)) = file(path).or_else(|| file("index.html")) else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let content_type = match route.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("ttf") => "font/ttf",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    };
    ([(header::CONTENT_TYPE, content_type)], *bytes).into_response()
}
