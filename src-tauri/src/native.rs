//! Native host authorization. Window identity is a Tauri concept, so it cannot live in core:
//! the web host authorizes the equivalent request through its session instead.

use portal_core::error::{AppError, ErrorCode};

/// Restricts sensitive commands to the application's primary webview.
pub fn ensure_main_window(window: &tauri::WebviewWindow) -> Result<(), AppError> {
    if window.label() != "main" {
        return Err(AppError::new(
            ErrorCode::AuthorizationDenied,
            "sensitive operations are available only from the main Portal window",
        ));
    }
    Ok(())
}
