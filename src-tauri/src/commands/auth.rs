//! Signing in to the desktop app with the owner password — the same credential, in the same
//! file, as the web server's, so a machine running both has one password.
//!
//! Signed in is a flag in this process: it survives a webview reload and ends when the app
//! quits, as a web session ends with the server.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use portal_core::error::{AppError, ErrorCode};
use portal_core::security::owner::{owner_file, OwnerCredential};
use portal_core::state::{AppState, DESKTOP_SESSION};

pub struct DesktopAuth {
    owner: OwnerCredential,
    signed_in: AtomicBool,
}

impl DesktopAuth {
    pub fn load() -> Self {
        let store = openswap::utill::get_taker_dir().ok().map(|root| owner_file(&root));
        DesktopAuth {
            owner: OwnerCredential::load(None, store),
            signed_in: AtomicBool::new(false),
        }
    }

    pub fn signed_in(&self) -> bool {
        self.signed_in.load(Ordering::SeqCst)
    }
}

/// The shape the web host's `/session` answers with, so the frontend reads both the same way.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSessionDto {
    authenticated: bool,
    has_owner: bool,
}

fn denied(message: &str) -> AppError {
    AppError::new(ErrorCode::AuthorizationDenied, message)
}

#[tauri::command]
pub fn auth_session(auth: tauri::State<'_, Arc<DesktopAuth>>) -> AuthSessionDto {
    AuthSessionDto {
        authenticated: auth.signed_in(),
        has_owner: auth.owner.has_owner(),
    }
}

/// Argon2 is deliberately slow, so both of these run off the main thread the window draws on.
#[tauri::command]
pub async fn auth_claim(
    auth: tauri::State<'_, Arc<DesktopAuth>>,
    password: String,
) -> Result<(), AppError> {
    let auth = Arc::clone(&auth);
    tokio::task::spawn_blocking(move || auth.owner.claim(&password).map_err(denied))
        .await
        .map_err(AppError::internal)?
}

#[tauri::command]
pub async fn auth_login(
    auth: tauri::State<'_, Arc<DesktopAuth>>,
    password: String,
) -> Result<(), AppError> {
    let auth = Arc::clone(&auth);
    tokio::task::spawn_blocking(move || {
        auth.owner.verify(&password).map_err(denied)?;
        auth.signed_in.store(true, Ordering::SeqCst);
        Ok(())
    })
    .await
    .map_err(AppError::internal)?
}

/// Signing out takes the window off its wallet too, exactly as a web session ending does.
#[tauri::command]
pub fn auth_logout(auth: tauri::State<'_, Arc<DesktopAuth>>, state: tauri::State<'_, Arc<AppState>>) {
    auth.signed_in.store(false, Ordering::SeqCst);
    portal_core::ops::taker_wallet::end_session(&state, DESKTOP_SESSION);
}
