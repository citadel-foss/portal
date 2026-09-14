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
        let auth = Auth::load(
            config.owner_credential_file.as_deref(),
            config.bootstrap_file.as_deref(),
            Some(data_root.join("portal").join("auth").join("owner")),
        );
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

    /// Whether this request path authenticates. False only for a plain local run that has
    /// never been claimed: loopback, development profile, nothing provisioned, no owner on
    /// disk. Once an owner exists the password is honoured for the rest of the install's
    /// life — a restart without `--bootstrap-file` must not quietly drop protection from a
    /// server the user deliberately secured.
    pub fn open_local(&self) -> bool {
        self.config.open_local() && !self.auth.has_owner()
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
