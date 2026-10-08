//! Multi-maker lifecycle. Registrations and wallets persist on disk; live
//! `MakerServer` objects exist only in this app process and never auto-start.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::events::AppEvent;
use openswap::maker::{start_server, MakerServer, MakerServerConfig};
use openswap::utill::MIN_RELAY_FEE_RATE;
use openswap::wallet::Wallet;

use crate::error::{from_wallet_join_error, AppError, ErrorCode};
use crate::ops::chain_backend;
use crate::ops::maker_settings;
use crate::security::input::{validate_leaf_name, validate_password};
use crate::security::operation::{SensitiveOperation, SensitiveOperationGuard};
use crate::state::{try_lock_makers, AppState, MakerHandle, MakerRuntime};
use crate::storage::wallet_path;
use crate::types::{
    MakerInitConfig, MakerPhase, MakerPhaseEvent, MakerSettingsDto, MakerStatusDto, WalletInfo,
};

/// The crate's `MAX_MAKER_NAME_LEN`, which it keeps `pub(crate)`. Wallets refuse an offer whose
/// name breaks the rule, so a router that ignored it would run and never be picked.
pub(crate) const MAX_ROUTER_NAME_LEN: usize = 32;

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn resolve_maker_data_dir(config: &MakerInitConfig) -> Result<PathBuf, AppError> {
    match &config.data_dir {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => crate::storage::maker_data_dir(&config.router_id),
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
    let name_len = config.name.chars().count();
    if name_len == 0 || name_len > MAX_ROUTER_NAME_LEN || config.name.chars().any(char::is_control)
    {
        return Err(invalid(format!(
            "name must be 1 to {MAX_ROUTER_NAME_LEN} characters, with no control characters"
        )));
    }

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
    // The crate only clamps this when parsing config.toml; a value handed to it directly goes
    // into the bond transaction as-is, and below the relay floor that transaction never confirms.
    if !config.fidelity_feerate.is_finite() || config.fidelity_feerate < MIN_RELAY_FEE_RATE {
        return Err(invalid(format!(
            "fidelityFeerate must be at least {MIN_RELAY_FEE_RATE} sats/vB"
        )));
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

/// What a new router should be configured with, taken from the crate rather than restated.
pub fn router_defaults() -> crate::types::RouterDefaultsDto {
    let core = MakerServerConfig::default();
    crate::types::RouterDefaultsDto {
        fidelity_amount: core.fidelity_amount,
        min_fidelity_amount: maker_settings::min_fidelity_amount(),
        fidelity_timelock: core.fidelity_timelock,
        fidelity_feerate: core.fidelity_feerate,
        required_confirms: core.required_confirms,
        base_fee: core.base_fee,
        amount_relative_fee_pct: core.amount_relative_fee_pct,
        time_relative_fee_pct: core.time_relative_fee_pct,
    }
}

/// The way `makerd` builds it: the crate reads the router's `config.toml`, applying its own checks
/// (such as the fidelity bond minimum), and only what a file cannot hold is filled in here — the
/// session's chain route, the wallet password, and Portal's own Tor.
fn build_config(
    chain: &crate::types::ChainBackendConfig,
    config: MakerInitConfig,
    data_dir: PathBuf,
) -> Result<MakerServerConfig, AppError> {
    let tor = crate::tor::ensure_tor().map_err(|e| AppError::new(ErrorCode::TorUnreachable, e))?;
    let backend = chain_backend::resolve_from(chain, &config.wallet_name, Some(tor.socks_port))?;
    let mut server = MakerServerConfig::new(Some(&data_dir.join("config.toml")))?;
    server.data_dir = data_dir;
    server.backend = backend;
    server.wallet_name = config.wallet_name;
    server.password = config.wallet_password;
    server.control_port = tor.control_port;
    server.socks_port = tor.socks_port;
    server.tor_auth_password = tor.control_password;
    // Always on: the crate skips screening while the list is empty, so the list alone decides
    // whether anything is refused.
    server.check_blocklist = true;
    Ok(server)
}

async fn construct_server(
    session: &str,
    config: MakerInitConfig,
    data_dir: PathBuf,
) -> Result<Arc<MakerServer>, AppError> {
    let socks_port = crate::tor::ensure_tor()
        .map_err(|e| AppError::new(ErrorCode::TorUnreachable, e))?
        .socks_port;
    // Read once: the network recorded must be the one this server was built on, even if the
    // session's backend changes while `MakerServer::init` runs.
    let chain = chain_backend::load(session);
    // Before the crate opens the wallet: its startup recovery writes reports straight away.
    crate::storage::move_sidecars_out(&data_dir, &config.wallet_name)?;
    let server_config = build_config(&chain, config, data_dir.clone())?;
    let server = tokio::task::spawn_blocking(move || MakerServer::init(server_config))
        .await
        .map_err(from_wallet_join_error)?
        .map_err(AppError::from)?;
    chain_backend::record_network_once(session, data_dir, chain, socks_port);
    Ok(Arc::new(server))
}

/// Unwinds a failed `init_maker` so the attempt can be retried.
///
/// `created` is the wallet this attempt made, if it made one: `MakerServer::init` goes through
/// `Wallet::load_or_init` and can create the wallet before a later step (sync, watch service,
/// report load) fails, and leaving it behind would reject every retry of the same name. A
/// wallet that was already there — a removed router being registered again — is `None`, and
/// is never touched: it holds that router's funds and bond.
fn abort_failed_creation(
    state: &AppState,
    router_id: &str,
    created: Option<&Path>,
    error: &AppError,
) -> Result<(), AppError> {
    if let Some(wallet_file) = created {
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
    state
        .events
        .publish(AppEvent::RouterPhaseChanged(MakerPhaseEvent {
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
    session: &str,
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
    // directory, so a wallet sitting here belongs to a router that was removed. Registering it
    // again — only with the password that opens it — brings back its funds and bond.
    let wallet_file = wallet_path(&data_dir, &config.wallet_name);
    let restore = config.restore_selection.take();
    if restore.is_some() && wallet_file.exists() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "this router folder already has a wallet — create the router with its password, \
             or pick another router ID to restore into",
        ));
    }
    let mut readding = restore.is_none() && wallet_file.exists();
    if readding {
        let (path, password) = (wallet_file.clone(), password.to_string());
        tokio::task::spawn_blocking(move || {
            crate::ops::taker_wallet::verify_wallet_password(&path, Some(password))
        })
        .await
        .map_err(AppError::internal)??;
    }
    let settings = MakerSettingsDto::from_init(&config, &data_dir);
    {
        let mut makers = try_lock_makers(&state.makers)?;
        if makers.contains_key(&router_id) {
            return Err(AppError::new(
                ErrorCode::RouterBusy,
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
        let phase = if restore.is_some() {
            MakerPhase::Restoring
        } else {
            MakerPhase::Initializing
        };
        makers.insert(
            router_id.clone(),
            MakerHandle {
                settings: settings.clone(),
                runtime: None,
                phase: phase.clone(),
                generation: 0,
            },
        );
        drop(makers);
        emit_phase(state, &router_id, phase);
    }

    crate::logging::register_maker(router_id.clone(), data_dir.clone(), settings.network_port);
    // Registered first, as Initializing, so the router's log can be followed while the restore
    // scans, which takes minutes.
    if let Some(selection_id) = restore {
        let restored = async {
            let _operation = SensitiveOperationGuard::acquire(
                &state.sensitive_operation_active,
                SensitiveOperation::RestorePrivateKey,
            )?;
            let backup =
                crate::ops::taker_wallet::take_restore_selection(state, selection_id, password)
                    .await?;
            let socks_port = crate::tor::ensure_tor()
                .map_err(|e| AppError::new(ErrorCode::TorUnreachable, e))?
                .socks_port;
            let backend = chain_backend::resolve(session, &config.wallet_name, Some(socks_port))?;
            crate::ops::taker_wallet::restore_backup(
                backup,
                data_dir.clone(),
                config.wallet_name.clone(),
                backend,
                password.to_string(),
            )
            .await
        }
        .await;
        if let Err(error) = restored {
            // Restore rejects an existing wallet above, so cleanup can only remove this attempt's file.
            abort_failed_creation(state, &router_id, Some(&wallet_file), &error)?;
            return Err(error);
        }
        // The restored wallet is opened like a re-added one, with the backup's password.
        readding = true;
    }
    let created = (!readding).then_some(wallet_file.as_path());
    // Written before the server is built, which reads it back through the crate. A file this
    // attempt created goes with a failed attempt: left behind, the rejected values would be what
    // the corrected retry reads.
    let config_file = data_dir.join("config.toml");
    let fresh_config = !config_file.exists();
    let abort = |error: &AppError| -> Result<(), AppError> {
        if fresh_config {
            let _ = std::fs::remove_file(&config_file);
        }
        abort_failed_creation(state, &router_id, created, error)
    };
    if let Err(error) = maker_settings::write_runtime_config(&settings) {
        abort(&error)?;
        return Err(error);
    }
    let server = match construct_server(session, config, data_dir.clone()).await {
        Ok(server) => server,
        Err(error) => {
            abort(&error)?;
            return Err(error);
        }
    };
    if let Err(error) = maker_settings::save(&settings) {
        server.watch_service.shutdown();
        drop(server);
        abort(&error)?;
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
        has_bond: None,
    })
}

/// Keeps a stopped router reserved until the backup has released its wallet.
pub struct BackupWallet {
    // Fields drop in declaration order: release the wallet before allowing another open.
    wallet: Arc<std::sync::RwLock<Wallet>>,
    _reservation: Option<RouterReservation>,
}

impl From<Arc<std::sync::RwLock<Wallet>>> for BackupWallet {
    fn from(wallet: Arc<std::sync::RwLock<Wallet>>) -> Self {
        Self {
            wallet,
            _reservation: None,
        }
    }
}

impl AsRef<std::sync::RwLock<Wallet>> for BackupWallet {
    fn as_ref(&self) -> &std::sync::RwLock<Wallet> {
        &self.wallet
    }
}

struct RouterReservation {
    state: Arc<AppState>,
    router_id: String,
    previous: MakerPhase,
}

impl Drop for RouterReservation {
    fn drop(&mut self) {
        let mut makers = self.state.makers.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = makers.get_mut(&self.router_id) {
            entry.phase = self.previous.clone();
        }
    }
}

/// A router's wallet for a one-off read such as a backup. A running router's is its live one. A
/// stopped router's is opened from its file with `wallet_password`, the way `init_maker` re-adds
/// a router, while the router is held busy so a start cannot open the same file meanwhile.
pub async fn router_wallet(
    state: &Arc<AppState>,
    session: &str,
    router_id: &str,
    wallet_password: Option<String>,
) -> Result<BackupWallet, AppError> {
    if let Ok(wallet) = crate::ops::maker_wallet::get_maker_wallet_handle(state, router_id) {
        return Ok(wallet.into());
    }
    if !state.makers.lock()?.contains_key(router_id) {
        let settings =
            maker_settings::load(router_id)?.ok_or_else(|| AppError::maker_not_found(router_id))?;
        insert_saved_registration(state, settings)?;
    }
    let (settings, previous) = {
        let mut makers = try_lock_makers(&state.makers)?;
        let entry = makers
            .get_mut(router_id)
            .ok_or_else(|| AppError::maker_not_found(router_id))?;
        if !matches!(
            entry.phase,
            MakerPhase::Stopped | MakerPhase::Failed { .. } | MakerPhase::NotConfigured
        ) {
            return Err(AppError::maker_busy());
        }
        let previous = std::mem::replace(&mut entry.phase, MakerPhase::Initializing);
        (entry.settings.clone(), previous)
    };
    let reservation = RouterReservation {
        state: state.clone(),
        router_id: router_id.to_string(),
        previous,
    };
    let session = session.to_string();
    let router_id = router_id.to_string();
    // Cancellation must not release the reservation while construct_server is still opening
    // the file on its blocking thread.
    tokio::spawn(async move {
        let config = settings.into_init(wallet_password);
        let data_dir = resolve_maker_data_dir(&config)?;
        // `MakerServer::init` creates a wallet where none exists; this must only ever open one.
        let file = wallet_path(&data_dir, &config.wallet_name);
        if !file.exists() {
            return Err(AppError::new(
                ErrorCode::WalletLoadFailed,
                format!("router '{router_id}' has no wallet file"),
            ));
        }
        let password = config.wallet_password.clone();
        tokio::task::spawn_blocking(move || {
            crate::ops::taker_wallet::verify_wallet_password(&file, password)
        })
        .await
        .map_err(AppError::internal)??;
        let server = construct_server(&session, config, data_dir).await?;
        server.watch_service.shutdown();
        Ok(BackupWallet {
            wallet: server.wallet.clone(),
            _reservation: Some(reservation),
        })
    })
    .await
    .map_err(AppError::internal)?
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
                ErrorCode::RouterBusy,
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

fn insert_saved_registration(
    state: &Arc<AppState>,
    settings: MakerSettingsDto,
) -> Result<(), AppError> {
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
    session: &str,
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
            MakerPhase::Restoring
            | MakerPhase::Starting
            | MakerPhase::Initializing
            | MakerPhase::Stopping => return Err(AppError::maker_busy()),
            MakerPhase::Running => {
                return Err(AppError::new(
                    ErrorCode::RouterAlreadyRunning,
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
                "Router {label} port {port} is already in use — most likely this router is already running in another Portal on this machine (the desktop app or the web server). Manage it there. Do not change the network port of a fidelity-bonded router."
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
    // A router imported from Maker Dashboard may have no file yet, and the crate would fill one
    // with its own defaults — ports and fees other than the ones registered.
    if !data_dir.join("config.toml").exists() {
        maker_settings::write_runtime_config(&settings)?;
    }
    crate::logging::register_maker(router_id.clone(), data_dir.clone(), settings.network_port);
    let server = match construct_server(session, config, data_dir.clone()).await {
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
        chain_backend: chain_backend::load(session),
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
    thread::spawn(move || {
        watch_startup(&watch_state, &router_id, generation, &server);
        sync_while_running(&watch_state, &router_id, generation, &server);
    });
    Ok(())
}

/// The taker wallet's cadence. The crate's server syncs its wallet only after a sweep, and the
/// offer's maximum size is refreshed by a sync, so without this a deposit stays unadvertised.
const WALLET_SYNC_INTERVAL: Duration = Duration::from_secs(2 * 60);

fn sync_while_running(
    state: &Arc<AppState>,
    router_id: &str,
    generation: u64,
    server: &MakerServer,
) {
    let current = || {
        state
            .makers
            .lock()
            .ok()
            .and_then(|makers| {
                makers
                    .get(router_id)
                    .map(|e| e.generation == generation && matches!(e.phase, MakerPhase::Running))
            })
            .unwrap_or(false)
    };
    let chain = state.makers.lock().ok().and_then(|makers| {
        Some(
            makers
                .get(router_id)?
                .runtime
                .as_ref()?
                .chain_backend
                .clone(),
        )
    });
    let Some(chain) = chain else { return };
    loop {
        // Checked every second so a stop is never held up by a sleeping timer.
        let mut waited = Duration::ZERO;
        while waited < WALLET_SYNC_INTERVAL {
            if server.shutdown.load(Ordering::Relaxed) || !current() {
                return;
            }
            thread::sleep(Duration::from_secs(1));
            waited += Duration::from_secs(1);
        }
        // The crate's sync retries an unreachable backend while holding the wallet lock, which
        // would stall every swap this router is serving.
        let status = chain_backend::probe(&chain, None);
        if !status.reachable {
            log::warn!(
                "router {router_id} wallet sync skipped, chain backend unreachable: {}",
                status.error.unwrap_or_default()
            );
            continue;
        }
        let result = server
            .wallet
            .write()
            .map_err(|_| AppError::internal("router wallet lock poisoned"))
            .and_then(|mut wallet| {
                wallet
                    .sync_and_save(&server.shutdown)
                    .map_err(AppError::from)
            });
        if let Err(e) = result {
            log::warn!("router {router_id} wallet sync failed: {}", e.message);
        }
    }
}

fn watch_startup(state: &Arc<AppState>, router_id: &str, generation: u64, server: &MakerServer) {
    loop {
        if server.is_setup_complete.load(Ordering::Relaxed) {
            if let Ok(mut makers) = state.makers.lock() {
                if let Some(entry) = makers.get_mut(router_id) {
                    if entry.generation == generation && matches!(entry.phase, MakerPhase::Starting)
                    {
                        entry.phase = MakerPhase::Running;
                        drop(makers);
                        emit_phase(state, router_id, MakerPhase::Running);
                    }
                }
            }
            break;
        }
        let keep_watching = state
            .makers
            .lock()
            .ok()
            .and_then(|makers| {
                makers
                    .get(router_id)
                    .map(|e| e.generation == generation && matches!(e.phase, MakerPhase::Starting))
            })
            .unwrap_or(false);
        if !keep_watching {
            break;
        }
        thread::sleep(Duration::from_millis(250));
    }
}

pub async fn stop_maker(state: &Arc<AppState>, router_id: String) -> Result<(), AppError> {
    let thread = {
        let mut makers = try_lock_makers(&state.makers)?;
        let entry = makers
            .get_mut(&router_id)
            .ok_or_else(|| AppError::maker_not_found(&router_id))?;
        if matches!(
            entry.phase,
            MakerPhase::Restoring | MakerPhase::Initializing | MakerPhase::Stopping
        ) {
            return Err(AppError::maker_busy());
        }
        if !matches!(entry.phase, MakerPhase::Starting | MakerPhase::Running) {
            return Err(AppError::new(
                ErrorCode::RouterNotRunning,
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
    // Never waits: a status poll must not queue behind a sync holding the wallet.
    let has_bond = entry.runtime.as_ref().and_then(|runtime| {
        runtime.server.wallet.try_read().ok().map(|wallet| {
            wallet
                .get_fidelity_bonds()
                .iter()
                .any(|bond| !bond.is_spent())
        })
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
        has_bond,
    })
}

pub fn get_maker_info(state: &Arc<AppState>, router_id: String) -> Result<WalletInfo, AppError> {
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

    #[tokio::test]
    async fn backup_reservation_blocks_another_open_and_restores_phase() {
        let state = Arc::new(AppState::default());
        let router_id = "backup-reservation".to_string();
        let missing =
            std::env::temp_dir().join(format!("portal-backup-test-{}", uuid::Uuid::new_v4()));
        let settings = MakerSettingsDto::from_init(&named("Test"), &missing);
        state.makers.lock().unwrap().insert(
            router_id.clone(),
            MakerHandle {
                settings,
                runtime: None,
                phase: MakerPhase::Initializing,
                generation: 0,
            },
        );
        let reservation = RouterReservation {
            state: state.clone(),
            router_id: router_id.clone(),
            previous: MakerPhase::Failed {
                message: "previous failure".to_string(),
            },
        };
        let result = router_wallet(&state, "test", &router_id, None).await;
        assert!(matches!(
            result,
            Err(AppError {
                code: ErrorCode::RouterBusy,
                ..
            })
        ));
        drop(reservation);
        assert!(matches!(&state.makers.lock().unwrap()[&router_id].phase,
            MakerPhase::Failed { message } if message == "previous failure"));
        let result = router_wallet(&state, "test", &router_id, None).await;
        assert!(matches!(
            result,
            Err(AppError {
                code: ErrorCode::WalletLoadFailed,
                ..
            })
        ));
        assert!(matches!(&state.makers.lock().unwrap()[&router_id].phase,
            MakerPhase::Failed { message } if message == "previous failure"));
    }

    #[test]
    fn maker_ids_are_path_safe() {
        assert!(valid_id("maker_01-test"));
        assert!(!valid_id("../maker"));
        assert!(!valid_id("maker one"));
    }

    fn named(name: &str) -> MakerInitConfig {
        MakerInitConfig {
            router_id: "router-1".to_string(),
            wallet_name: "router-1".to_string(),
            wallet_password: None,
            name: name.to_string(),
            network_port: 6102,
            rpc_port: 6103,
            socks_port: 9050,
            control_port: 9051,
            fidelity_amount: 10_000,
            fidelity_timelock: MakerServerConfig::default().fidelity_timelock,
            fidelity_feerate: MIN_RELAY_FEE_RATE,
            required_confirms: 1,
            base_fee: 100,
            amount_relative_fee_pct: 0.1,
            time_relative_fee_pct: 0.005,
            data_dir: None,
            restore_selection: None,
        }
    }

    /// Wallets refuse an offer whose name breaks this rule, so a router that got past it would
    /// run and never be picked.
    #[test]
    fn router_names_follow_the_rule_wallets_enforce() {
        assert!(validate_maker_config(&named("Asteroid Destroyer")).is_ok());
        assert!(validate_maker_config(&named(&"x".repeat(MAX_ROUTER_NAME_LEN))).is_ok());
        // Counted in characters, not bytes.
        assert!(validate_maker_config(&named(&"é".repeat(MAX_ROUTER_NAME_LEN))).is_ok());
        assert!(validate_maker_config(&named(&"x".repeat(MAX_ROUTER_NAME_LEN + 1))).is_err());
        assert!(validate_maker_config(&named("")).is_err());
        assert!(validate_maker_config(&named("line\nbreak")).is_err());
        assert!(validate_maker_config(&named("\u{1b}[2J")).is_err());
    }

    #[test]
    fn wallet_names_are_single_components() {
        assert!(validate_leaf_name("wallet.dat", "walletName").is_ok());
        assert!(validate_leaf_name("../wallet.dat", "walletName").is_err());
        assert!(validate_leaf_name("nested/wallet.dat", "walletName").is_err());
    }
}
