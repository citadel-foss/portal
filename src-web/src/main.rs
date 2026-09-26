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

use std::time::Duration;

use clap::Parser;
use portal_core::state::AppState;

use crate::config::Config;
use crate::state::WebState;

/// Deliberately not `#[tokio::main]`: the signal disposition is claimed before the runtime
/// exists, so nothing that starts afterwards can be running while the process is unarmed.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::parse();

    // Before validate(), and before the mask: a probe talks to an instance that is already
    // running, needs no assets directory or access profile of its own, and exits in seconds.
    if config.healthcheck {
        let url = format!("http://{}{}", config.bind, config.route("/health/live"));
        let ok = minreq::get(&url)
            .with_timeout(3)
            .send()
            .is_ok_and(|r| r.status_code == 200);
        std::process::exit(if ok { 0 } else { 1 });
    }

    portal_core::shutdown_signal::arm();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(serve(config))
}

async fn serve(config: Config) -> Result<(), Box<dyn std::error::Error>> {
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
    portal_core::logging::set_taker_dir(data_root.clone());
    portal_core::tor::sweep_stale_tor_dirs();

    // The protocol crate dumps a whole swap report to stdout when a swap ends, which buries
    // the URL below and shows nothing the app does not already render from the saved report.
    // Silenced only once the log file is confirmed to be taking the root logger's output, so
    // this can never be the reason there are no diagnostics anywhere. Deliberate lines go
    // through `console`, which still holds the real stdout.
    let mut console = if portal_core::logging::logs_to_file() {
        portal_core::console::detach()
    } else {
        portal_core::console::Console::Inherited
    };

    if let Some(dir) = &config.assets_dir {
        if !dir.is_dir() {
            return Err(format!("--assets-dir {} is not a directory", dir.display()).into());
        }
    }

    let runtime = Arc::new(AppState::default());
    // Opened before the listener binds: a corrupt or newer-schema journal must stop startup,
    // not surface once someone is already trying to spend.
    let journal = portal_core::operations::Journal::open(&data_root).map_err(|e| e.message)?;
    if journal.has_unsettled() {
        log::warn!("some operations from a previous run have unknown outcomes; only conflicting spends are held");
    }
    let state = WebState::new(config, runtime, journal, data_root.clone());
    let bind = state.config.bind;
    let shutdown_timeout = state.config.shutdown_timeout_secs;

    let app = routes::router(state.clone()).merge(assets::router(&state).with_state(state.clone()));

    // Expiry has a side effect — the session's wallet may close — so it cannot wait for the
    // session's own next request, which for a closed tab never comes.
    let reaper = state.auth.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            reaper.reap();
        }
    });

    // Tor comes up first and goes down last: everything that carries swap traffic binds to
    // its ports, and the teardown below halts it only after the routers and wallet are done
    // with it. Started here rather than on the first page view so a server left running has
    // a bootstrapped Tor waiting, instead of charging whoever opens the page ~60s for it.
    // Detached: a slow or failed bootstrap must not hold up the listener, and every caller
    // of `ensure_tor` already waits on the same single instance.
    tokio::task::spawn_blocking(|| {
        if let Err(e) = portal_core::tor::ensure_tor() {
            log::error!("Portal's Tor failed to start: {e}");
        }
    });

    let listener = tokio::net::TcpListener::bind(bind).await?;
    log::info!("portal-web listening on {bind}");
    if state.auth.has_owner() {
        console.line("sign in with the owner password for this installation");
    } else {
        console.line("no owner yet — open the page and choose the owner password");
    }
    // Deliberately last, and nothing may print after it: closing the tab does not stop the
    // server, so hours later this is the line someone scrolls back to in order to return.
    // Everything else the process has to say goes to the log file, not here.
    //
    // Only claimed when this process serves the UI. In development Vite serves it on its own
    // port and proxies the API here, so pointing anyone at this address lands them on a 404.
    if state.config.assets_dir.is_some() {
        console.line(&format!(
            "\nPortal is running. Open: {}",
            state.config.browsable_url()
        ));
    } else {
        console.line(&format!(
            "\nportal-web API on {} — the UI is served separately in development",
            state.config.browsable_url()
        ));
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(await_signal())
        .await?;

    // Reached once in-flight requests have drained. Everything below outlives the HTTP
    // server: routers, the wallet and Tor are the process, not the transport.
    teardown(&state, Duration::from_secs(shutdown_timeout)).await;
    Ok(())
}

/// Stops accepting new work on SIGTERM or SIGINT, then lets in-flight requests drain.
///
/// Deliberately not `tokio::signal`: embedded Tor replaces whatever handler is installed when
/// it starts, and tokio installs its own only once, so it cannot be put back afterwards. The
/// flag in `shutdown_signal` is re-armed after Tor is up and is the only thing that survives.
async fn await_signal() {
    portal_core::shutdown_signal::wait().await;
    log::info!("shutdown signal received; draining in-flight requests");
}

/// Ordered teardown, matching the desktop host.
///
/// Routers first: each finishes its in-flight connections and a closing wallet sync, and all
/// of that traffic is still riding on Tor. Then the wallets, whose `Drop` flushes each swap
/// tracker and stops each recovery loop. Tor last, so nothing that still needs it is cut off.
///
/// Every phase is logged, and bounded: the whole sequence must finish inside the supervisor's
/// stop grace, or the platform kills the process mid-write instead of letting it record what
/// it was doing.
async fn teardown(state: &WebState, budget: Duration) {
    let deadline = std::time::Instant::now() + budget;
    let remaining = |deadline: std::time::Instant| deadline.saturating_duration_since(std::time::Instant::now());

    let phase = |name: &'static str, work: Box<dyn FnOnce() + Send>, left: Duration| async move {
        log::info!("shutdown phase start: {name}");
        // Blocking work off the async runtime, bounded so one wedged phase cannot eat the
        // budget the later ones need.
        match tokio::time::timeout(left, tokio::task::spawn_blocking(work)).await {
            Ok(Ok(())) => log::info!("shutdown phase done: {name}"),
            Ok(Err(e)) => log::warn!("shutdown phase failed: {name}: {e}"),
            Err(_) => log::warn!("shutdown phase timed out: {name}"),
        }
    };

    let routers = state.runtime.clone();
    phase(
        "routers",
        Box::new(move || portal_core::ops::maker::shutdown_all(&routers)),
        remaining(deadline),
    )
    .await;

    let wallets = state.runtime.clone();
    phase(
        "wallets",
        Box::new(move || portal_core::ops::taker_wallet::shutdown_all(&wallets)),
        remaining(deadline),
    )
    .await;

    phase(
        "tor",
        Box::new(portal_core::tor::shutdown),
        remaining(deadline),
    )
    .await;

    log::info!("shutdown complete");
}
