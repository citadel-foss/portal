//! Self-hosted Portal: the second host over the same core the desktop app uses.

mod assets;
mod auth;
mod commands;
mod config;
mod files;
mod routes;
mod sse;
mod state;

use std::sync::Arc;

use clap::Parser;
use portal_core::state::AppState;

use crate::config::Config;
use crate::state::WebState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::parse();

    // Before validate(): a probe talks to an instance that is already running and needs no
    // assets directory or access profile of its own.
    if config.healthcheck {
        let url = format!("http://{}{}", config.bind, config.route("/health/live"));
        let ok = minreq::get(&url)
            .with_timeout(3)
            .send()
            .is_ok_and(|r| r.status_code == 200);
        std::process::exit(if ok { 0 } else { 1 });
    }

    config.validate()?;

    // Resolved once, here: core is handed a path, never a client-supplied one. A configured
    // override that disagrees with the crate's own default is a startup error rather than a
    // silently split data tree.
    let default_root = portal_core::storage::resolve_data_dir(&None)
        .map_err(|e| e.message)?;
    let data_root = match &config.data_root {
        Some(configured) if configured != &default_root => {
            return Err(format!(
                "--data-root {} does not match the path the protocol crate resolves ({}); \
                 set the process home instead of splitting the tree",
                configured.display(),
                default_root.display()
            )
            .into())
        }
        Some(configured) => configured.clone(),
        None => default_root,
    };
    std::fs::create_dir_all(&data_root)?;
    // Held for the life of the process. Two Portals on one root share wallets, the journal
    // and the Tor identity directory; the second to start would otherwise report a confusing
    // Tor bootstrap failure instead of the conflict that actually caused it.
    let _root_lock = portal_core::storage::lock_data_root(&data_root).map_err(|e| e.message)?;
    portal_core::logging::set_taker_dir(data_root.clone());

    if let Some(dir) = &config.assets_dir {
        if !dir.is_dir() {
            return Err(format!("--assets-dir {} is not a directory", dir.display()).into());
        }
    }

    let runtime = Arc::new(AppState::default());
    // Opened before the listener binds: a corrupt or newer-schema journal must stop startup,
    // not surface once someone is already trying to spend.
    let journal = portal_core::operations::Journal::open(&data_root).map_err(|e| e.message)?;
    if journal.has_unresolved() {
        log::warn!("operations from a previous run are unresolved; new spending is blocked until reconciled");
    }
    let state = WebState::new(config, runtime, journal, data_root.clone());
    let bind = state.config.bind;
    let shutdown_timeout = state.config.shutdown_timeout_secs;

    let app = routes::router(state.clone()).merge(assets::router(&state).with_state(state.clone()));

    let listener = tokio::net::TcpListener::bind(bind).await?;
    log::info!("portal-web listening on {bind}");
    println!("portal-web listening on http://{bind}");
    if state.open_local() {
        println!(
            "local mode: loopback only, no login. Pass --bootstrap-file to require one."
        );
    } else if state.auth.has_owner() {
        println!("sign in with the owner password for this installation");
    } else {
        println!("no owner yet — open the page and claim it with the --bootstrap-file secret");
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown(shutdown_timeout))
        .await?;
    Ok(())
}

/// Stops accepting new work on SIGTERM, then gives in-flight requests a bounded window. The
/// deadline must stay under the supervisor's stop grace, or the platform kills the process
/// mid-write instead of letting it record what it was doing.
async fn shutdown(timeout_secs: u64) {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler installs on unix");
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = ctrl_c.await;
    log::info!("draining for up to {timeout_secs}s");
}
