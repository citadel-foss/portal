//! Host state: what the web host owns on top of the shared runtime.
//!
//! Core owns the wallet, routers and events. Sessions, CSRF and request policy live here,
//! because they are the web host's concerns and must not leak into code the desktop shares.

use std::path::PathBuf;
use std::sync::Arc;

use portal_core::operations::Journal;
use portal_core::state::AppState;
use uuid::Uuid;

use crate::auth::Auth;
use crate::config::Config;

#[derive(Clone)]
pub struct WebState {
    pub config: Arc<Config>,
    pub auth: Arc<Auth>,
    pub runtime: Arc<AppState>,
    pub journal: Arc<Journal>,
    pub data_root: PathBuf,
    /// Stable across restarts is not required for v1; it identifies this process's runtime so
    /// a reconnecting browser can tell it is talking to the same one it had a snapshot from.
    pub runtime_id: String,
    pub installation_id: String,
}

impl WebState {
    pub fn new(
        config: Config,
        runtime: Arc<AppState>,
        journal: Journal,
        data_root: PathBuf,
    ) -> Self {
        let ended = runtime.clone();
        let auth = Auth::load(
            config.owner_credential_file.as_deref(),
            Some(portal_core::security::owner::owner_file(&data_root)),
        )
        .on_session_end(move |session| portal_core::ops::taker_wallet::end_session(&ended, session));
        WebState {
            config: Arc::new(config),
            auth: Arc::new(auth),
            runtime,
            journal: Arc::new(journal),
            data_root,
            runtime_id: Uuid::new_v4().to_string(),
            installation_id: Uuid::new_v4().to_string(),
        }
    }

    pub fn ctx(&self, caller: &crate::routes::Caller) -> crate::commands::Ctx {
        crate::commands::Ctx {
            rt: self.runtime.clone(),
            session: caller.session.clone(),
        }
    }

    /// Cached enough to answer a readiness probe without touching a wallet lock or the chain.
    pub fn storage_ready(&self) -> bool {
        self.data_root.is_dir()
    }

    /// A label, never a path: a browser has no business learning the server's filesystem
    /// layout, and nothing in the UI needs it.
    pub fn storage_label(&self) -> String {
        self.data_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("wallet storage")
            .to_string()
    }
}
