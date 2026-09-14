//! Multi-maker lifecycle. Registrations and wallets persist on disk; live
//! `MakerServer` objects exist only in this app process and never auto-start.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use openswap::maker::api::MIN_SWAP_AMOUNT;
use openswap::maker::{start_server, MakerServer, MakerServerConfig};
use openswap::utill::get_maker_dir;
use openswap::wallet::Wallet;
use crate::events::AppEvent;

use crate::ops::chain_backend;
use crate::ops::maker_settings;
use crate::storage::wallet_path;
use crate::error::{from_wallet_join_error, AppError, ErrorCode};
use crate::security::input::{validate_leaf_name, validate_password};
use crate::state::{try_lock_makers, AppState, MakerHandle, MakerRuntime};
use crate::types::{
    MakerInitConfig, MakerPhase, MakerPhaseEvent, MakerSettingsDto, MakerStatusDto, WalletInfo,
};

const MIN_FIDELITY_TIMELOCK: u32 = 12_960;
const MAX_FIDELITY_TIMELOCK: u32 = 25_920;

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn default_maker_data_dir(router_id: &str) -> Result<PathBuf, AppError> {
    let legacy = get_maker_dir()?;
    Ok(legacy
        .parent()
        .map(|base| base.join(router_id))
        .unwrap_or_else(|| legacy.join(router_id)))
}

fn resolve_maker_data_dir(config: &MakerInitConfig) -> Result<PathBuf, AppError> {
    match &config.data_dir {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => default_maker_data_dir(&config.router_id),
    }
}

fn validate_maker_config(config: &MakerInitConfig) -> Result<(), AppError> {
    let invalid = |msg: String| AppError::new(ErrorCode::InvalidInput, msg);
    if !valid_id(config.router_id.trim()) {
        return Err(invalid(
            "routerId must contain only letters, numbers, '-' or '_'".to_string(),
        ));
    }
    validate_leaf_name(&config.wallet_name, "walletName")?;

    let ports = [
        ("networkPort", config.network_port),
        ("rpcPort", config.rpc_port),
    ];
    for (name, port) in ports {
        if port == 0 {
            return Err(invalid(format!("{name} must be between 1 and 65535")));
        }
    }
    for i in 0..ports.len() {
        for j in (i + 1)..ports.len() {
            if ports[i].1 == ports[j].1 {
                return Err(invalid(format!(
                    "{} and {} cannot use the same port ({})",
                    ports[i].0, ports[j].0, ports[i].1
                )));
            }
        }
    }
    if !(MIN_FIDELITY_TIMELOCK..=MAX_FIDELITY_TIMELOCK).contains(&config.fidelity_timelock) {
        return Err(invalid(format!(
            "fidelityTimelock must be between {MIN_FIDELITY_TIMELOCK} and {MAX_FIDELITY_TIMELOCK} blocks"
        )));
    }
    if config.min_swap_amount < MIN_SWAP_AMOUNT {
        return Err(invalid(format!(
            "minSwapAmount must be at least {MIN_SWAP_AMOUNT} sats"
        )));
    }
    if config.fidelity_amount == 0 {
        return Err(invalid("fidelityAmount must be greater than 0".to_string()));
    }
    if config.required_confirms == 0 {
        return Err(invalid("requiredConfirms must be at least 1".to_string()));
    }
    for (name, value) in [
        ("amountRelativeFeePct", config.amount_relative_fee_pct),
        ("timeRelativeFeePct", config.time_relative_fee_pct),
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(invalid(format!("{name} must be finite and non-negative")));
        }
    }
    Ok(())
}

fn build_config(config: MakerInitConfig, data_dir: PathBuf) -> Result<MakerServerConfig, AppError> {
    let tor = crate::tor::ensure_tor().map_err(|e| AppError::new(ErrorCode::TorUnreachable, e))?;
    let backend = chain_backend::resolve(&config.wallet_name, Some(tor.socks_port))?;
    Ok(MakerServerConfig {
        data_dir,
        network_port: config.network_port,
        rpc_port: config.rpc_port,
        base_fee: config.base_fee,
        amount_relative_fee_pct: config.amount_relative_fee_pct,
        time_relative_fee_pct: config.time_relative_fee_pct,
        min_swap_amount: config.min_swap_amount,
        required_confirms: config.required_confirms,
        fidelity_amount: config.fidelity_amount,
        fidelity_timelock: config.fidelity_timelock,
        backend,
        wallet_name: config.wallet_name,
        control_port: tor.control_port,
        socks_port: tor.socks_port,
        tor_auth_password: tor.control_password,
        password: config.wallet_password,
        ..MakerServerConfig::default()
    })
}

async fn construct_server(
    config: MakerInitConfig,
    data_dir: PathBuf,
) -> Result<Arc<MakerServer>, AppError> {
    let server_config = build_config(config, data_dir)?;
    let server = tokio::task::spawn_blocking(move || MakerServer::init(server_config))
        .await
        .map_err(from_wallet_join_error)?
        .map_err(AppError::from)?;
    Ok(Arc::new(server))
}

/// Unwinds a failed `init_maker` so the attempt can be retried.
///
/// `init_maker` proves the wallet file is absent before `MakerServer::init` runs, so anything
/// at that path afterwards was created by this attempt — `MakerServer::init` goes through
/// `Wallet::load_or_init` and can create the wallet before a later step (sync, watch service,
/// report load) fails. Leaving it behind would make the pre-existence check reject every
/// retry of the same wallet name, and no command can register an already-created wallet.
fn abort_failed_creation(
    state: &AppState,
    router_id: &str,
    wallet_file: &Path,
    error: &AppError,
) -> Result<(), AppError> {
    match std::fs::remove_file(wallet_file) {
        Ok(()) => log::info!(
            "removed wallet from failed maker creation: {}",
            wallet_file.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!(
            "could not remove wallet from failed maker creation {}: {e}",
            wallet_file.display()
        ),
    }
    state.makers.lock()?.remove(router_id);
    crate::logging::unregister_maker(router_id);
    emit_phase(
        state,
        router_id,
        MakerPhase::Failed {
            message: error.message.clone(),
        },
    );
    Ok(())
}

pub(crate) fn read_tor_hostname(data_dir: &Path) -> Option<String> {
    let bytes = std::fs::read(data_dir.join("tor/hostname")).ok()?;
    if let Ok(hostname) = serde_cbor::from_slice::<String>(&bytes) {
        return Some(hostname);
    }
    let [_, hostname]: [String; 2] = serde_cbor::from_slice(&bytes).ok()?;
    Some(hostname)
}

fn emit_phase(state: &AppState, router_id: &str, phase: MakerPhase) {
    state.events.publish(AppEvent::RouterPhaseChanged(MakerPhaseEvent {
        router_id: router_id.to_string(),
        phase,
    }));
}

fn ensure_unique_registration(
    router_id: &str,
    data_dir: &Path,
    network_port: u16,
    rpc_port: u16,
) -> Result<(), AppError> {
    for saved in maker_settings::load_all()?.values() {
        if saved.router_id == router_id {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "router ID is already registered",
            ));
        }
        if saved.data_dir.as_deref().map(Path::new) == Some(data_dir) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "another router already uses this data directory",
            ));
        }
        if [saved.network_port, saved.rpc_port].contains(&network_port)
            || [saved.network_port, saved.rpc_port].contains(&rpc_port)
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "router network/RPC port is already registered",
            ));
        }
    }
    Ok(())
}

fn ensure_unique_settings_update(settings: &MakerSettingsDto) -> Result<(), AppError> {
    for saved in maker_settings::load_all()?.values() {
        if saved.router_id == settings.router_id {
            continue;
        }
        if [saved.network_port, saved.rpc_port].contains(&settings.network_port)
            || [saved.network_port, saved.rpc_port].contains(&settings.rpc_port)
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "router network/RPC port is already registered",
            ));
        }
    }
    Ok(())
}

/// Creates and registers a new maker wallet. This does not start its server.
pub async fn init_maker(
    state: &Arc<AppState>,
    mut config: MakerInitConfig,
) -> Result<MakerStatusDto, AppError> {
    config.router_id = config.router_id.trim().to_string();
    config.wallet_name = config.wallet_name.trim().to_string();
    validate_maker_config(&config)?;
    let password = config.wallet_password.as_deref().ok_or_else(|| {
        AppError::new(
            ErrorCode::InvalidInput,
            "Portal-created routers require a wallet password",
        )
    })?;
    validate_password(password, "router wallet password")?;
    let router_id = config.router_id.clone();
    let data_dir = resolve_maker_data_dir(&config)?;
    if config.data_dir.is_some() && data_dir.exists() {
        crate::security::fs::require_private_dir(&data_dir)?;
    } else {
        crate::security::fs::ensure_private_dir(&data_dir)?;
    }
    crate::security::fs::ensure_private_dir(&data_dir.join("wallets"))?;
    ensure_unique_registration(&router_id, &data_dir, config.network_port, config.rpc_port)?;
    // `ensure_unique_registration` has already ruled out a registration for this ID or data
    // directory, so a wallet sitting here has none — and nothing can register an existing one.
    let wallet_file = wallet_path(&data_dir, &config.wallet_name);
    if wallet_file.exists() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!(
                "a wallet named '{}' already exists in this data directory — choose a different \
                 wallet name or data directory",
                config.wallet_name
            ),
        ));
    }

    let settings = MakerSettingsDto::from_init(&config, &data_dir);
    {
        let mut makers = try_lock_makers(&state.makers)?;
        if makers.contains_key(&router_id) {
            return Err(AppError::new(
                ErrorCode::MakerBusy,
                "router is already being created",
            ));
        }
        if makers.values().any(|entry| {
            entry.settings.data_dir.as_deref().map(Path::new) == Some(data_dir.as_path())
                || [entry.settings.network_port, entry.settings.rpc_port]
                    .contains(&config.network_port)
                || [entry.settings.network_port, entry.settings.rpc_port].contains(&config.rpc_port)
        }) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "another router creation is already reserving this data directory or port",
            ));
        }
        makers.insert(
            router_id.clone(),
            MakerHandle {
                settings: settings.clone(),
                runtime: None,
                phase: MakerPhase::Initializing,
                generation: 0,
            },
        );
    }
    emit_phase(state, &router_id, MakerPhase::Initializing);

    crate::logging::register_maker(router_id.clone(), data_dir.clone(), settings.network_port);
    let server = match construct_server(config, data_dir.clone()).await {
        Ok(server) => server,
        Err(error) => {
            abort_failed_creation(state, &router_id, &wallet_file, &error)?;
            return Err(error);
        }
    };
    if let Err(error) = maker_settings::write_runtime_config(&settings)
        .and_then(|_| maker_settings::save(&settings))
    {
        server.watch_service.shutdown();
        drop(server);
        abort_failed_creation(state, &router_id, &wallet_file, &error)?;
        return Err(error);
    }
    // `MakerServer::init` is the crate's only wallet create/load API and starts
    // a watch service as a side effect. Creation is registration-only here, so
    // stop that temporary service and reconstruct a fresh runtime on start.
    server.watch_service.shutdown();
    drop(server);

    let network_port = settings.network_port;
    let mut makers = state.makers.lock()?;
    let entry = makers
        .get_mut(&router_id)
        .ok_or_else(|| AppError::maker_not_found(&router_id))?;
    entry.runtime = None;
    entry.phase = MakerPhase::Stopped;
    drop(makers);
    emit_phase(state, &router_id, MakerPhase::Stopped);

    Ok(MakerStatusDto {
        router_id,
        phase: MakerPhase::Stopped,
        running: false,
        tor_address: None,
        network_port,
        wallet_encrypted: Some(true),
    })
}

/// Updates a stopped maker's persisted configuration. Wallet identity and
/// storage location are immutable; changing them would silently point the
/// registration at a different wallet. A fresh runtime reads these settings
/// the next time the maker starts.
pub fn update_maker_settings(
    state: &Arc<AppState>,
    router_id: String,
    settings: MakerSettingsDto,
) -> Result<MakerSettingsDto, AppError> {
    let existing =
        maker_settings::load(&router_id)?.ok_or_else(|| AppError::maker_not_found(&router_id))?;
    if settings.router_id != router_id {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "router ID cannot be changed",
        ));
    }
    if settings.wallet_name != existing.wallet_name || settings.data_dir != existing.data_dir {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "router wallet name and data directory cannot be changed",
        ));
    }

    validate_maker_config(&settings.clone().into_init(None))?;
    ensure_unique_settings_update(&settings)?;

    let mut makers = try_lock_makers(&state.makers)?;
    if let Some(entry) = makers.get(&router_id) {
        let runtime_thread_is_active = entry
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.thread.as_ref())
            .is_some_and(|thread| !thread.is_finished());
        if !matches!(entry.phase, MakerPhase::Stopped | MakerPhase::Failed { .. })
            || runtime_thread_is_active
        {
            return Err(AppError::new(
                ErrorCode::MakerBusy,
                "stop the router before changing its settings",
            ));
        }
    }

    maker_settings::write_runtime_config(&settings)?;
    maker_settings::save(&settings)?;
    if let Some(entry) = makers.get_mut(&router_id) {
        entry.settings = settings.clone();
        entry.runtime = None;
    }
    drop(makers);

    if let Some(data_dir) = settings.data_dir.as_deref() {
        crate::logging::register_maker(router_id, PathBuf::from(data_dir), settings.network_port);
    }
    Ok(settings)
}

fn insert_saved_registration(state: &Arc<AppState>, settings: MakerSettingsDto) -> Result<(), AppError> {
    let router_id = settings.router_id.clone();
    let mut makers = state.makers.lock()?;
    makers.entry(router_id).or_insert(MakerHandle {
        settings,
        runtime: None,
        phase: MakerPhase::Stopped,
        generation: 0,
    });
    Ok(())
}

/// Starts a registered maker. After a fresh app launch, the runtime is first
/// reconstructed from persisted settings and the supplied wallet password.
pub async fn start_maker(
    state: &Arc<AppState>,
    router_id: String,
    wallet_password: Option<String>,
) -> Result<(), AppError> {
    // Reload before every start so config.toml remains the source of truth even
    // when it was edited outside this process between maker runs.
    let persisted_settings =
        maker_settings::load(&router_id)?.ok_or_else(|| AppError::maker_not_found(&router_id))?;
    if !state.makers.lock()?.contains_key(&router_id) {
        insert_saved_registration(state, persisted_settings.clone())?;
    }

    {
        let mut makers = try_lock_makers(&state.makers)?;
        let entry = makers
            .get_mut(&router_id)
            .ok_or_else(|| AppError::maker_not_found(&router_id))?;
        match entry.phase {
            MakerPhase::Starting | MakerPhase::Initializing | MakerPhase::Stopping => {
                return Err(AppError::maker_busy())
            }
            MakerPhase::Running => {
                return Err(AppError::new(
                    ErrorCode::MakerAlreadyRunning,
                    "router is already running",
                ))
            }
            _ => {}
        }
        entry.settings = persisted_settings;
        entry.phase = MakerPhase::Initializing;
        // A MakerServer owns one-shot watcher/thread-pool services. Never reuse
        // it after a stop or failed run.
        entry.runtime = None;
    }
    emit_phase(state, &router_id, MakerPhase::Initializing);

    let settings = {
        let makers = state.makers.lock()?;
        let entry = makers
            .get(&router_id)
            .ok_or_else(|| AppError::maker_not_found(&router_id))?;
        entry.settings.clone()
    };

    // Fail before wallet/runtime construction when another application or a
    // leftover maker process owns either listener. Choosing another network
    // port automatically is unsafe because an existing fidelity bond may
    // commit to this maker address.
    for (label, port) in [
        ("network", settings.network_port),
        ("RPC", settings.rpc_port),
    ] {
        if TcpListener::bind(("127.0.0.1", port)).is_err() {
            let message = format!(
                "Router {label} port {port} is already in use. Stop the other router process using this port before starting. Do not change the network port of a fidelity-bonded router."
            );
            if let Some(entry) = state.makers.lock()?.get_mut(&router_id) {
                entry.phase = MakerPhase::Failed {
                    message: message.clone(),
                };
            }
            emit_phase(
                state,
                &router_id,
                MakerPhase::Failed {
                    message: message.clone(),
                },
            );
            return Err(AppError::new(ErrorCode::InvalidInput, message));
        }
    }
    // The crate creates the wallet when the file is absent, so a registration whose wallet
    // was moved or deleted is a *creation* path — and Portal never creates one unencrypted.
    // Imported dashboard makers keep starting passwordless while their wallet still exists.
    let config = settings.clone().into_init(wallet_password);
    if !wallet_path(&resolve_maker_data_dir(&config)?, &config.wallet_name).exists() {
        let password = config.wallet_password.as_deref().unwrap_or_default();
        if let Err(error) = validate_password(password, "router wallet password") {
            let message = format!(
                "{} — the wallet file for '{router_id}' is missing, so starting it would create a \
                 new one",
                error.message
            );
            if let Some(entry) = state.makers.lock()?.get_mut(&router_id) {
                entry.phase = MakerPhase::Failed {
                    message: message.clone(),
                };
            }
            emit_phase(
                state,
                &router_id,
                MakerPhase::Failed {
                    message: message.clone(),
                },
            );
            return Err(AppError::new(ErrorCode::InvalidInput, message));
        }
    }
    if let Err(error) = validate_maker_config(&config) {
        if let Some(entry) = state.makers.lock()?.get_mut(&router_id) {
            entry.phase = MakerPhase::Failed {
                message: error.message.clone(),
            };
        }
        emit_phase(
            state,
            &router_id,
            MakerPhase::Failed {
                message: error.message.clone(),
            },
        );
        return Err(error);
    }
    let data_dir = resolve_maker_data_dir(&config)?;
    crate::logging::register_maker(router_id.clone(), data_dir.clone(), settings.network_port);
    let server = match construct_server(config, data_dir.clone()).await {
        Ok(server) => server,
        Err(error) => {
            if let Some(entry) = state.makers.lock()?.get_mut(&router_id) {
                entry.phase = MakerPhase::Failed {
                    message: error.message.clone(),
                };
            }
            emit_phase(
                state,
                &router_id,
                MakerPhase::Failed {
                    message: error.message.clone(),
                },
            );
            return Err(error);
        }
    };
    let mut makers = state.makers.lock()?;
    let entry = makers
        .get_mut(&router_id)
        .ok_or_else(|| AppError::maker_not_found(&router_id))?;
    entry.runtime = Some(MakerRuntime {
        server,
        thread: None,
    });

    entry.generation = entry.generation.wrapping_add(1);
    let generation = entry.generation;
    let runtime = entry
        .runtime
        .as_mut()
        .ok_or_else(AppError::maker_not_initialized)?;
    runtime.server.shutdown.store(false, Ordering::Relaxed);
    runtime
        .server
        .is_setup_complete
        .store(false, Ordering::Relaxed);
    let server = runtime.server.clone();
    entry.phase = MakerPhase::Starting;

    let run_state = Arc::clone(state);
    let run_id = router_id.clone();
    let run_server = server.clone();
    let server_thread = thread::Builder::new()
        .name(format!("maker-{router_id}"))
        .spawn(move || {
            let cleanup_server = run_server.clone();
            let result = start_server(run_server);
            if result.is_err() {
                // `openswap::start_server` has early error paths after it may
                // have spawned background work. Ensure those services cannot
                // outlive a failed maker runtime.
                cleanup_server.shutdown.store(true, Ordering::Relaxed);
                cleanup_server.watch_service.shutdown();
                let _ = cleanup_server.thread_pool.join_all_threads();
            }
            let phase = match result {
                Ok(()) => MakerPhase::Stopped,
                Err(e) => MakerPhase::Failed {
                    message: format!("{e:?}"),
                },
            };
            let state = &run_state;
            if let Ok(mut makers) = state.makers.lock() {
                if let Some(entry) = makers.get_mut(&run_id) {
                    if entry.generation == generation
                        && !matches!(entry.phase, MakerPhase::Stopping)
                    {
                        entry.phase = phase.clone();
                        drop(makers);
                        emit_phase(state, &run_id, phase);
                    }
                }
            };
        });
    let server_thread = match server_thread {
        Ok(thread) => thread,
        Err(error) => {
            runtime.server.watch_service.shutdown();
            entry.phase = MakerPhase::Failed {
                message: error.to_string(),
            };
            entry.runtime = None;
            drop(makers);
            emit_phase(
                state,
                &router_id,
                MakerPhase::Failed {
                    message: error.to_string(),
                },
            );
            return Err(AppError::internal(error));
        }
    };
    runtime.thread = Some(server_thread);
    drop(makers);
    emit_phase(state, &router_id, MakerPhase::Starting);

    let watch_state = Arc::clone(state);
    thread::spawn(move || loop {
        if server.is_setup_complete.load(Ordering::Relaxed) {
            let state = &watch_state;
            if let Ok(mut makers) = state.makers.lock() {
                if let Some(entry) = makers.get_mut(&router_id) {
                    if entry.generation == generation && matches!(entry.phase, MakerPhase::Starting)
                    {
                        entry.phase = MakerPhase::Running;
                        drop(makers);
                        emit_phase(state, &router_id, MakerPhase::Running);
                    }
                }
            }
            break;
        }
        let keep_watching = watch_state
            .makers
            .lock()
            .ok()
            .and_then(|makers| {
                makers
                    .get(&router_id)
                    .map(|e| e.generation == generation && matches!(e.phase, MakerPhase::Starting))
            })
            .unwrap_or(false);
        if !keep_watching {
            break;
        }
        thread::sleep(Duration::from_millis(250));
    });
    Ok(())
}

pub async fn stop_maker(state: &Arc<AppState>, router_id: String) -> Result<(), AppError> {
    let thread = {
        let mut makers = try_lock_makers(&state.makers)?;
        let entry = makers
            .get_mut(&router_id)
            .ok_or_else(|| AppError::maker_not_found(&router_id))?;
        if matches!(entry.phase, MakerPhase::Initializing | MakerPhase::Stopping) {
            return Err(AppError::maker_busy());
        }
        if !matches!(entry.phase, MakerPhase::Starting | MakerPhase::Running) {
            return Err(AppError::new(
                ErrorCode::MakerNotRunning,
                "router is not running",
            ));
        }
        entry.phase = MakerPhase::Stopping;
        let runtime = entry
            .runtime
            .as_mut()
            .ok_or_else(AppError::maker_not_initialized)?;
        runtime.server.shutdown.store(true, Ordering::Relaxed);
        runtime.thread.take()
    };
    emit_phase(state, &router_id, MakerPhase::Stopping);
    let join_result = if let Some(thread) = thread {
        tokio::task::spawn_blocking(move || thread.join())
            .await
            .map_err(AppError::internal)?
            .map_err(|_| AppError::new(ErrorCode::Internal, "router server thread panicked"))
    } else {
        Ok(())
    };
    if let Some(entry) = state.makers.lock()?.get_mut(&router_id) {
        entry.runtime = None;
        entry.phase = if join_result.is_ok() {
            MakerPhase::Stopped
        } else {
            MakerPhase::Failed {
                message: "router server thread panicked".to_string(),
            }
        };
    }
    let phase = if join_result.is_ok() {
        MakerPhase::Stopped
    } else {
        MakerPhase::Failed {
            message: "router server thread panicked".to_string(),
        }
    };
    emit_phase(state, &router_id, phase);
    join_result
}

pub fn get_maker_status(
    state: &Arc<AppState>,
    router_id: String,
) -> Result<MakerStatusDto, AppError> {
    if !state.makers.lock()?.contains_key(&router_id) {
        if let Some(settings) = maker_settings::load(&router_id)? {
            insert_saved_registration(state, settings)?;
        }
    }
    let makers = state.makers.lock()?;
    let entry = makers
        .get(&router_id)
        .ok_or_else(|| AppError::maker_not_found(&router_id))?;
    let (running, runtime_tor_address) = entry
        .runtime
        .as_ref()
        .map(|runtime| {
            (
                runtime.thread.as_ref().is_some_and(|t| !t.is_finished()),
                read_tor_hostname(&runtime.server.data_dir),
            )
        })
        .unwrap_or((false, None));
    let tor_address = runtime_tor_address.or_else(|| {
        entry
            .settings
            .data_dir
            .as_deref()
            .and_then(|data_dir| read_tor_hostname(Path::new(data_dir)))
    });
    let wallet_encrypted = entry.settings.data_dir.as_deref().and_then(|data_dir| {
        Wallet::is_wallet_encrypted(&wallet_path(
            Path::new(data_dir),
            &entry.settings.wallet_name,
        ))
        .ok()
    });
    Ok(MakerStatusDto {
        router_id,
        phase: entry.phase.clone(),
        running,
        tor_address,
        network_port: entry.settings.network_port,
        wallet_encrypted,
    })
}

pub fn get_maker_info(
    state: &Arc<AppState>,
    router_id: String,
) -> Result<WalletInfo, AppError> {
    if !state.makers.lock()?.contains_key(&router_id) {
        if let Some(settings) = maker_settings::load(&router_id)? {
            insert_saved_registration(state, settings)?;
        }
    }
    let makers = state.makers.lock()?;
    let entry = makers
        .get(&router_id)
        .ok_or_else(|| AppError::maker_not_found(&router_id))?;
    let data_dir = entry
        .settings
        .data_dir
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(AppError::maker_not_initialized)?;
    Ok(WalletInfo {
        wallet_path: wallet_path(&data_dir, &entry.settings.wallet_name)
            .display()
            .to_string(),
        wallet_name: entry.settings.wallet_name.clone(),
        data_dir: data_dir.display().to_string(),
    })
}

/// Signals and joins every maker. Used only during process shutdown.
pub fn shutdown_all(state: &Arc<AppState>) {
    let threads = state
        .makers
        .lock()
        .map(|mut makers| {
            makers
                .values_mut()
                .filter_map(|entry| {
                    let runtime = entry.runtime.as_mut()?;
                    runtime.server.shutdown.store(true, Ordering::Relaxed);
                    runtime.thread.take()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for thread in threads {
        let _ = thread.join();
    }
    if let Ok(mut makers) = state.makers.lock() {
        for entry in makers.values_mut() {
            entry.runtime = None;
            entry.phase = MakerPhase::Stopped;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maker_ids_are_path_safe() {
        assert!(valid_id("maker_01-test"));
        assert!(!valid_id("../maker"));
        assert!(!valid_id("maker one"));
    }

    #[test]
    fn wallet_names_are_single_components() {
        assert!(validate_leaf_name("wallet.dat", "walletName").is_ok());
        assert!(validate_leaf_name("../wallet.dat", "walletName").is_err());
        assert!(validate_leaf_name("nested/wallet.dat", "walletName").is_err());
    }
}
