//! Wallet lifecycle: open, join, release, restore, backup.
//!
//! Wallet load/restore commands must route errors through
//! `from_wallet_join_error`, not `AppError::internal` — a wrong password
//! panics inside the crate instead of returning `Result`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use openswap::bitcoin::{Address, OutPoint, Txid};
use openswap::maker::nostr::NOSTR_RELAYS;
use openswap::taker::api::ConnectionType;
use openswap::taker::{Taker, TakerInitConfig};
use openswap::utill::get_taker_dir;
use openswap::wallet::{AddressType, Wallet};
use uuid::Uuid;

use crate::storage::{self, resolve_data_dir, wallet_path};

use crate::ops::chain_backend;
use crate::error::{from_wallet_join_error, AppError, ErrorCode};
use crate::security::input::validate_leaf_name;
use crate::security::operation::{SensitiveOperation, SensitiveOperationGuard};
use crate::state::{AppState, PendingFileSelection, TakerInstance, TakerSlot};
use crate::types::{
    AddressTypeDto, AddressValidation, BalancesDto, ConnectionTypeDto, FeeEstimate, InitConfig,
    InitResult, NewAddress, Outpoint, PathsDto, PriceEstimate, RestoreSelectionView, SendResult,
    SessionStateDto, TxSummary, UtxoEntry, WalletInfo,
};

const BTC_PRICE_CACHE_FILE: &str = "btc-price-cache.json";
const MAX_PRICE_CACHE_BYTES: u64 = 4096;
static PRICE_CACHE_IO: Mutex<()> = Mutex::new(());

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CachedBtcPrice {
    usd: f64,
    fetched_at: u64,
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn valid_usd_price(usd: f64) -> bool {
    usd.is_finite() && usd > 0.0
}

fn price_cache_path() -> Result<PathBuf, AppError> {
    Ok(get_taker_dir()?.join(BTC_PRICE_CACHE_FILE))
}

fn load_cached_btc_price() -> Result<Option<CachedBtcPrice>, AppError> {
    let _guard = PRICE_CACHE_IO.lock()?;
    let path = price_cache_path()?;
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_PRICE_CACHE_BYTES
    {
        return Ok(None);
    }
    let cached: CachedBtcPrice = match serde_json::from_slice(&std::fs::read(path)?) {
        Ok(cached) => cached,
        Err(error) => {
            log::warn!("ignoring invalid BTC price cache: {error}");
            return Ok(None);
        }
    };
    if !valid_usd_price(cached.usd) {
        return Ok(None);
    }
    Ok(Some(cached))
}

fn save_cached_btc_price(cached: &CachedBtcPrice) -> Result<(), AppError> {
    let _guard = PRICE_CACHE_IO.lock()?;
    let path = price_cache_path()?;
    if let Some(parent) = path.parent() {
        crate::security::fs::ensure_private_dir(parent)?;
    }
    let body = serde_json::to_vec(cached).map_err(AppError::internal)?;
    crate::security::fs::write_private(&path, &body)
}

pub fn list_wallets(data_dir: Option<String>) -> Result<Vec<String>, AppError> {
    storage::list_wallets(&data_dir)
}

/// The host's real wallet locations. Desktop-only: the web host hands the browser opaque
/// wallet IDs and a storage label instead, since a browser has no business seeing server paths.
pub fn get_paths(state: &Arc<AppState>, session: &str) -> Result<PathsDto, AppError> {
    let root = match state.taker_for(session) {
        Ok(taker) => taker.root.clone(),
        Err(_) => resolve_data_dir(&None)?,
    };
    Ok(PathsDto {
        wallets_dir: root.join(storage::WALLET_DATA_DIR).display().to_string(),
        data_dir: root.display().to_string(),
    })
}

/// How often a caller re-checks a wallet another session is still initializing.
const OPENING_POLL: Duration = Duration::from_millis(250);

enum OpenStep {
    Join(Arc<TakerInstance>),
    Wait,
    Init,
}

/// Unlocks a wallet for this session.
///
/// A wallet another session already has open is joined, not reinitialized: two Takers over the
/// same files would each overwrite the other's saves, and each `Taker::init` fails whatever
/// swaps it finds unfinished. Joining still takes the wallet password, checked against the file.
pub async fn init_taker(
    state: &Arc<AppState>,
    session: &str,
    config: InitConfig,
) -> Result<InitResult, AppError> {
    validate_leaf_name(&config.wallet_name, "walletName")?;
    let tor = crate::tor::ensure_tor().map_err(|e| AppError::new(ErrorCode::TorUnreachable, e))?;

    let root = resolve_data_dir(&config.data_dir)?;
    if config.data_dir.is_some() {
        crate::security::fs::require_private_dir(&root)?;
    } else {
        crate::security::fs::ensure_private_dir(&root)?;
    }
    let key = storage::wallet_data_dir(&root, &config.wallet_name);

    // Switching wallets: leave the old one first, which may drop it if nobody else is on it.
    if state.wallet_of(session).is_some_and(|bound| bound != key) {
        release_session(state, session);
    }

    loop {
        let step = {
            let mut takers = state.takers.lock()?;
            match takers.get(&key) {
                Some(TakerSlot::Open(taker)) => OpenStep::Join(taker.clone()),
                Some(TakerSlot::Opening) => OpenStep::Wait,
                None => {
                    takers.insert(key.clone(), TakerSlot::Opening);
                    OpenStep::Init
                }
            }
        };
        match step {
            OpenStep::Join(taker) => return join_taker(state, session, &taker, config, tor.socks_port).await,
            // Bound while waiting, so this session sees the other one's init progress too.
            OpenStep::Wait => {
                state.bindings.lock()?.insert(session.to_string(), key.clone());
                tokio::time::sleep(OPENING_POLL).await;
            }
            OpenStep::Init => break,
        }
    }

    state.bindings.lock()?.insert(session.to_string(), key.clone());
    let opened = open_taker(state, session, &root, &key, config, &tor).await;
    let mut takers = state.takers.lock()?;
    match opened {
        Ok((taker, result)) => {
            takers.insert(key, TakerSlot::Open(taker));
            Ok(result)
        }
        Err(error) => {
            takers.remove(&key);
            drop(takers);
            let mut bindings = state.bindings.lock()?;
            if bindings.get(session) == Some(&key) {
                bindings.remove(session);
            }
            Err(error)
        }
    }
}

/// Starts a Taker for a wallet nobody has open. The caller holds its `Opening` slot.
async fn open_taker(
    state: &Arc<AppState>,
    session: &str,
    root: &Path,
    key: &Path,
    config: InitConfig,
    tor: &crate::tor::TorRuntime,
) -> Result<(Arc<TakerInstance>, InitResult), AppError> {
    // A release still saving this wallet has to finish before it is loaded again.
    let pending = state.releasing.lock()?.remove(key);
    if let Some(handle) = pending {
        tokio::task::spawn_blocking(move || {
            let _ = handle.join();
        })
        .await
        .map_err(AppError::internal)?;
    }

    crate::security::fs::ensure_private_dir(&root.join(storage::WALLET_DATA_DIR))?;
    crate::security::fs::ensure_private_dir(key)?;
    crate::security::fs::ensure_private_dir(&key.join("wallets"))?;
    let dir_lock = storage::lock_wallet_dir(key)?;

    let connection_type = match config.connection_type {
        ConnectionTypeDto::Tor => ConnectionType::Tor,
        ConnectionTypeDto::Clearnet => ConnectionType::Clearnet,
    };
    let chain_config = chain_backend::load(session);
    let init_cfg = TakerInitConfig {
        data_dir: Some(key.to_path_buf()),
        wallet_name: config.wallet_name.clone(),
        backend: chain_backend::resolve_from(
            &chain_config,
            &config.wallet_name,
            Some(tor.socks_port),
        )?,
        control_port: Some(tor.control_port),
        tor_auth_password: Some(tor.control_password.clone()),
        socks_port: tor.socks_port,
        password: config.wallet_password,
        connection_type,
        // `None` leaves the crate's own config in charge, which defaults to off. Screening
        // funding inputs against an address blocklist is a product decision for a privacy
        // tool, not something to switch on because an upstream bump made the field required.
        check_blocklist: None,
        nostr_relays: NOSTR_RELAYS.iter().map(|s| s.to_string()).collect(),
    };

    // Our own dual-role logger, not the crate's setup_taker_logger — see logging.rs. One file
    // per root: the crate's threads carry no wallet in their names, so per-wallet files could
    // not be told apart.
    crate::logging::set_taker_dir(root.to_path_buf());

    // `Taker::init` runs startup recovery inline, which blocks on a block being mined and can
    // hold this call for a block interval; the watcher is what lets the UI say which phase of it
    // is running instead of showing one opaque spinner.
    let sink = state.events.clone();
    let wallet_key = key.to_path_buf();
    let taker = tokio::task::spawn_blocking(move || {
        let _watch = crate::logging::watch_init_phases(sink, wallet_key);
        Taker::init(init_cfg)
    })
    .await
    .map_err(from_wallet_join_error)?
    .map_err(AppError::from)?;

    let instance = Arc::new(TakerInstance {
        wallet_name: config.wallet_name.clone(),
        data_dir: key.to_path_buf(),
        root: root.to_path_buf(),
        wallet: taker.get_wallet().clone(),
        offer_sync: taker.offer_sync_client(),
        taker: Arc::new(Mutex::new(Some(taker))),
        chain_backend: chain_config,
        socks_port: tor.socks_port,
        active_swap: Mutex::new(None),
        sync_cancel: Arc::new(AtomicBool::new(false)),
        is_offerbook_syncing: AtomicBool::new(false),
        last_offerbook_sync_ts: AtomicU64::new(0),
        sessions: Mutex::new(HashSet::from([session.to_string()])),
        dir_lock: Mutex::new(Some(dir_lock)),
    });
    let result = InitResult {
        wallet_name: config.wallet_name,
        data_dir: root.display().to_string(),
        joined: false,
        note: None,
    };
    Ok((instance, result))
}

/// Adds a session to a wallet another session already has open.
async fn join_taker(
    state: &Arc<AppState>,
    session: &str,
    taker: &Arc<TakerInstance>,
    config: InitConfig,
    socks_port: u16,
) -> Result<InitResult, AppError> {
    let path = wallet_path(&taker.data_dir, &taker.wallet_name);
    let password = config.wallet_password.filter(|p| !p.is_empty());
    tokio::task::spawn_blocking(move || verify_wallet_password(&path, password))
        .await
        .map_err(from_wallet_join_error)??;

    let note = check_join_network(session, taker, socks_port).await?;
    taker.sessions.lock()?.insert(session.to_string());
    state
        .bindings
        .lock()?
        .insert(session.to_string(), taker.data_dir.clone());
    Ok(InitResult {
        wallet_name: taker.wallet_name.clone(),
        data_dir: taker.root.display().to_string(),
        joined: true,
        note,
    })
}

/// Proves the password opens the wallet file without loading a second copy of the wallet.
///
/// AES-GCM authenticates, so a wrong password cannot decrypt to anything. The encryption check
/// comes first because without a password the crate reads the file as plaintext, and any CBOR
/// document — an encrypted one included — parses as "something".
fn verify_wallet_password(path: &Path, password: Option<String>) -> Result<(), AppError> {
    use openswap::security::{load_sensitive_struct, SecurityError, SerdeCbor};
    let wrong = || AppError::new(ErrorCode::WalletWrongPassword, "incorrect wallet password");
    if password.is_none() {
        return if Wallet::is_wallet_encrypted(path)? {
            Err(wrong())
        } else {
            Ok(())
        };
    }
    match load_sensitive_struct::<serde::de::IgnoredAny, SerdeCbor>(path, password) {
        Ok(_) => Ok(()),
        Err(SecurityError::Decryption | SecurityError::PasswordRequired) => Err(wrong()),
        Err(e) => Err(AppError::new(ErrorCode::WalletLoadFailed, format!("{e:?}"))),
    }
}

/// A wallet belongs to one chain, and the running Taker's backend is on it — the crate refused
/// to load the wallet otherwise. So a joining session whose own backend reports a different
/// chain has picked the wrong network, exactly as it would have on a fresh init.
async fn check_join_network(
    session: &str,
    taker: &TakerInstance,
    socks_port: u16,
) -> Result<Option<String>, AppError> {
    let own = chain_backend::load(session);
    if chain_backend::fingerprint(&own, Some(socks_port))
        == chain_backend::fingerprint(&taker.chain_backend, Some(taker.socks_port))
    {
        return Ok(None);
    }
    let (mine, running) = tokio::join!(
        chain_backend::check_backend(session, Some(own.clone()), Some(socks_port)),
        chain_backend::check_backend(session, Some(taker.chain_backend.clone()), Some(taker.socks_port)),
    );
    let (Some(mine), Some(running)) = (mine?.chain, running?.chain) else {
        return Err(AppError::new(
            ErrorCode::RpcUnreachable,
            "Could not confirm which network this wallet is on. Check the connection and try again.",
        ));
    };
    if mine != running {
        // The backend reports mainnet as "bitcoin", which reads as the currency.
        let name = |chain: &str| if chain == "bitcoin" { "mainnet".to_string() } else { chain.to_string() };
        let (mine, running) = (name(&mine), name(&running));
        return Err(AppError::new(
            ErrorCode::WalletNetworkMismatch,
            format!(
                "This is a {running} wallet, but this browser is connected to {mine}. Go back \
                 and pick a {running} server."
            ),
        ));
    }
    Ok(Some(format!(
        "This wallet was already open in another browser, so it keeps that browser's connection ({}).",
        chain_backend::describe(&taker.chain_backend)
    )))
}

/// Takes this session off its wallet. The wallet itself is dropped once no session is left on
/// it, unless a swap is still running — the swap thread releases it when the swap settles.
pub fn release_session(state: &Arc<AppState>, session: &str) {
    let Some(key) = state.bindings.lock().ok().and_then(|mut b| b.remove(session)) else {
        return;
    };
    if let Ok(takers) = state.takers.lock() {
        if let Some(TakerSlot::Open(taker)) = takers.get(&key) {
            if let Ok(mut sessions) = taker.sessions.lock() {
                sessions.remove(session);
            }
        }
    }
    release_if_unused(state, &key);
}

/// A session that is gone for good: signed out or expired. Also drops its gate choice, which
/// can hold a node's RPC password.
pub fn end_session(state: &Arc<AppState>, session: &str) {
    release_session(state, session);
    chain_backend::forget_session(session);
}

/// Drops the wallet at `key` if nothing is using it any more.
pub(crate) fn release_if_unused(state: &Arc<AppState>, key: &Path) {
    let Ok(mut takers) = state.takers.lock() else {
        return;
    };
    let idle = match takers.get(key) {
        Some(TakerSlot::Open(taker)) => {
            !taker.swap_running() && taker.sessions.lock().is_ok_and(|s| s.is_empty())
        }
        _ => false,
    };
    if !idle {
        return;
    }
    let Some(TakerSlot::Open(taker)) = takers.remove(key) else {
        return;
    };
    drop(takers);
    spawn_release(state, taker);
}

/// Drops a Taker off the caller's thread.
///
/// The crate's `Drop` joins every background thread, and one mid-way through an offer fetch
/// over Tor only notices the shutdown flag between makers — measured at over twenty seconds.
/// Nothing a user is waiting on should sit behind that. The wallet-dir lock goes with it, and
/// only once the final save is done.
fn spawn_release(state: &Arc<AppState>, taker: Arc<TakerInstance>) {
    // Raised first, so a sync already inside the crate's retry loop unwinds instead of holding
    // the wallet lock that `Drop`'s final save needs.
    taker.sync_cancel.store(true, Ordering::Relaxed);
    let key = taker.data_dir.clone();
    let spawned = std::thread::Builder::new()
        .name("taker-release".into())
        .spawn(move || {
            // Blocking on purpose: a command still inside the crate — a prepare, a poll —
            // finishes before the Taker it is using goes away.
            let inner = taker
                .taker
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            drop(inner);
            if let Ok(mut lock) = taker.dir_lock.lock() {
                lock.take();
            }
        });
    match spawned {
        Ok(handle) => {
            if let Ok(mut releasing) = state.releasing.lock() {
                releasing.retain(|_, handle| !handle.is_finished());
                releasing.insert(key, handle);
            }
        }
        Err(e) => log::error!("could not start the wallet release thread: {e}"),
    }
}

/// Drops every wallet and waits for all of them to finish saving. For the quit paths only.
///
/// A wallet with a running swap is left alone: the swap thread holds its Taker for the whole
/// swap, and the swap's own per-phase writes are what the next launch recovers from.
pub fn shutdown_all(state: &Arc<AppState>) {
    if let Ok(mut bindings) = state.bindings.lock() {
        bindings.clear();
    }
    for taker in state.open_takers() {
        if let Ok(mut sessions) = taker.sessions.lock() {
            sessions.clear();
        }
        if taker.swap_running() {
            log::warn!("{} has a swap running; leaving it to the swap thread", taker.wallet_name);
            continue;
        }
        release_if_unused(state, &taker.data_dir);
    }
    let pending: Vec<_> = state
        .releasing
        .lock()
        .map(|mut releasing| releasing.drain().map(|(_, handle)| handle).collect())
        .unwrap_or_default();
    for handle in pending {
        let _ = handle.join();
    }
}

/// Infallible by design: every branch is a valid answer, so a fresh start never looks broken.
pub fn get_session_state(state: &Arc<AppState>, session: &str) -> SessionStateDto {
    match state.taker_for(session) {
        Ok(taker) => SessionStateDto {
            initialized: true,
            wallet_name: Some(taker.wallet_name.clone()),
            data_dir: Some(taker.root.display().to_string()),
        },
        Err(_) => SessionStateDto {
            initialized: false,
            wallet_name: None,
            data_dir: None,
        },
    }
}

pub fn get_wallet_info(taker: &TakerInstance) -> Result<WalletInfo, AppError> {
    Ok(WalletInfo {
        wallet_path: wallet_path(&taker.data_dir, &taker.wallet_name)
            .display()
            .to_string(),
        wallet_name: taker.wallet_name.clone(),
        data_dir: taker.data_dir.display().to_string(),
    })
}

/// Opens a native picker and retains the selected local path only in Rust.
/// Records a host-selected backup file and hands back an opaque ID. The selection itself is
/// the host's business — a native picker on desktop, an upload on the web — but the validation
/// and the expiring registry are the same either way, so they live here.
///
/// Takes the guard by value rather than acquiring its own: the host already holds one across
/// the file selection, and acquiring a second would deadlock against it.
pub fn register_restore_selection(
    state: &Arc<AppState>,
    _operation: SensitiveOperationGuard,
    path: PathBuf,
) -> Result<RestoreSelectionView, AppError> {
    let metadata = std::fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AppError::new(
            ErrorCode::InvalidFileSelection,
            "restore selection must be a regular file, not a symlink",
        ));
    }
    let canonical = std::fs::canonicalize(path)?;
    let display_name = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("wallet backup")
        .to_string();
    let selection_id = Uuid::new_v4();
    let mut selections = state.pending_file_selections.lock()?;
    selections.retain(|_, item| item.created_at.elapsed() <= Duration::from_secs(300));
    selections.insert(
        selection_id,
        PendingFileSelection {
            path: canonical,
            created_at: std::time::Instant::now(),
        },
    );
    Ok(RestoreSelectionView {
        selection_id,
        display_name,
    })
}

/// Restore from a Rust-selected backup before `init_taker`. Bad password/file panics (caught via
/// from_wallet_join_error); a real WalletError is swallowed by the crate, so check the output.
pub async fn restore_wallet(
    state: &Arc<AppState>,
    session: &str,
    data_dir: Option<String>,
    wallet_name: String,
    socks_port: Option<u16>,
    selection_id: Uuid,
    password: Option<String>,
) -> Result<(), AppError> {
    validate_leaf_name(&wallet_name, "walletName")?;
    let _operation = SensitiveOperationGuard::acquire(
        &state.sensitive_operation_active,
        SensitiveOperation::RestorePrivateKey,
    )?;
    let root = resolve_data_dir(&data_dir)?;
    if data_dir.is_some() {
        crate::security::fs::require_private_dir(&root)?;
    } else {
        crate::security::fs::ensure_private_dir(&root)?;
    }
    crate::security::fs::ensure_private_dir(&root.join(storage::WALLET_DATA_DIR))?;
    let dir = storage::wallet_data_dir(&root, &wallet_name);
    let restored_path = wallet_path(&dir, &wallet_name);

    // Do not consume a one-shot file selection for an error the user can fix by
    // choosing a different wallet name and submitting the same restore again.
    if restored_path.exists() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("a wallet named '{wallet_name}' already exists — pick another name"),
        ));
    }
    let selection = state
        .pending_file_selections
        .lock()?
        .remove(&selection_id)
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::InvalidFileSelection,
                "restore file selection is missing, expired, or already used",
            )
        })?;
    if selection.created_at.elapsed() > Duration::from_secs(300) {
        return Err(AppError::new(
            ErrorCode::InvalidFileSelection,
            "restore file selection expired; choose the file again",
        ));
    }
    crate::security::fs::ensure_private_dir(&dir)?;
    crate::security::fs::ensure_private_dir(&dir.join("wallets"))?;
    let backend = chain_backend::resolve(session, &wallet_name, socks_port)?;
    let backup_path = selection.path;

    tokio::task::spawn_blocking(move || {
        openswap::wallet::ffi::restore_wallet_gui_app(
            Some(dir),
            Some(wallet_name),
            backend,
            backup_path,
            password,
        )
    })
    .await
    .map_err(from_wallet_join_error)?;

    if !restored_path.exists() {
        return Err(AppError::new(
            ErrorCode::WalletLoadFailed,
            "restore did not produce a wallet file — check the app log for the underlying cause",
        ));
    }
    Ok(())
}

/// Backs up to encrypted JSON (xpriv, not a seed phrase). Rust owns the save dialog.
/// Writes the password-encrypted backup to a host-chosen destination. The host owns how that
/// destination was picked and holds the sensitive-operation guard across the choice, so this
/// takes the guard rather than acquiring a second one.
pub async fn write_backup(
    taker: &TakerInstance,
    _operation: SensitiveOperationGuard,
    destination: PathBuf,
    password: String,
) -> Result<String, AppError> {
    let wallet = taker.wallet.clone();
    // The openswap helper always replaces the selected extension with `.json`.
    // Validate and pre-create that actual target so its first write is private too.
    let destination = destination.with_extension("json");
    if destination.exists()
        && std::fs::symlink_metadata(&destination)?
            .file_type()
            .is_symlink()
    {
        return Err(AppError::new(
            ErrorCode::InvalidFileSelection,
            "backup destination cannot be a symlink",
        ));
    }
    let destination_path = destination.display().to_string();
    let display_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("wallet backup")
        .to_string();
    crate::security::fs::write_private(&destination, &[])?;

    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        wallet
            .read()?
            .backup_wallet_gui_app(destination_path, Some(password))?;
        Ok(())
    })
    .await
    .map_err(AppError::internal)??;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(display_name)
}

// --- Wallet operations: balances, addresses, history, UTXOs, send, sync, fees ---

/// Validate address encoding before review without coupling the UI to a
/// particular Bitcoin network. send_to_address performs the authoritative
/// active-wallet network check before constructing a transaction.
pub fn validate_address(address: String) -> AddressValidation {
    let address = address.trim();
    if address.is_empty() {
        return AddressValidation {
            valid: false,
            error: Some("Enter a recipient address.".to_string()),
        };
    }

    match Address::from_str(address) {
        Ok(_) => AddressValidation {
            valid: true,
            error: None,
        },
        Err(_) => AddressValidation {
            valid: false,
            error: Some("Enter a valid Bitcoin address.".to_string()),
        },
    }
}

pub async fn get_balances(taker: &TakerInstance) -> Result<BalancesDto, AppError> {
    let wallet = taker.wallet.clone();
    tokio::task::spawn_blocking(move || -> Result<BalancesDto, AppError> {
        let b = wallet.read()?.get_balances()?;
        Ok(BalancesDto {
            regular: b.regular.to_sat(),
            swap: b.swap.to_sat(),
            contract: b.contract.to_sat(),
            fidelity: b.fidelity.to_sat(),
            spendable: b.spendable.to_sat(),
        })
    })
    .await
    .map_err(AppError::internal)?
}

/// Last address issued per type, cached next to the wallet — the crate's
/// `get_next_external_address` always derives+increments with no "peek" mode, so this is the only
/// way to know what to re-offer instead of burning a fresh gap-limit index every call.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct LastAddresses {
    p2wpkh: Option<String>,
    p2tr: Option<String>,
}

fn last_address_path(taker: &TakerInstance) -> PathBuf {
    taker
        .data_dir
        .join("wallets")
        .join(format!("{}_last_address.json", taker.wallet_name))
}

fn load_last_addresses(path: &PathBuf) -> LastAddresses {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_last_addresses(path: &Path, addrs: &LastAddresses) -> Result<(), AppError> {
    let json = serde_json::to_string_pretty(addrs).map_err(AppError::internal)?;
    crate::security::fs::write_private(path, json.as_bytes())
}

/// Recent transactions scanned to decide whether the last issued address has been paid.
///
/// Only the last address issued is ever under test, so any payment to it is recent by
/// construction. Kept small because the Electrum backend's `list_transactions` fetches every
/// input of every transaction in the window, one round trip each — the window, not the wallet,
/// is what makes that call expensive.
const USED_ADDRESS_LOOKBACK: usize = 10;

/// Reuses the last address issued for this type until it actually receives a payment, matching
/// the old app and standard HD-wallet gap-limit-safe behavior — repeat calls (page reload,
/// clicking Generate again) shouldn't advance the derivation index for no reason.
///
/// Deliberately asks the chain nothing: a cached address is handed back unverified so the
/// receive panel can render immediately, and `verify_last_address` does the expensive part
/// afterwards. Blocking address issuance on that check is what made Receive slow.
pub async fn get_new_address(
    taker: &TakerInstance,
    address_type: AddressTypeDto,
) -> Result<NewAddress, AppError> {
    let wallet = taker.wallet.clone();
    let path = last_address_path(taker);
    let (addr_type, label) = address_kind(address_type);
    tokio::task::spawn_blocking(move || -> Result<NewAddress, AppError> {
        let mut cached = load_last_addresses(&path);
        if let Some(existing) = address_slot(&mut cached, addr_type).clone() {
            return Ok(NewAddress {
                address: existing,
                address_type: label.to_string(),
                verified: false,
            });
        }

        let address = wallet
            .write()?
            .get_next_external_address(addr_type)?
            .to_string();
        *address_slot(&mut cached, addr_type) = Some(address.clone());
        save_last_addresses(&path, &cached)?;
        Ok(NewAddress {
            address,
            address_type: label.to_string(),
            verified: true,
        })
    })
    .await
    .map_err(AppError::internal)?
}

/// Confirms the cached address is still unpaid, issuing a fresh one if it isn't.
///
/// The slow half of address issuance, split out so it runs after the panel has already painted.
/// Returns whatever address the user should be offering, always verified.
pub async fn verify_last_address(
    taker: &TakerInstance,
    address_type: AddressTypeDto,
) -> Result<NewAddress, AppError> {
    let wallet = taker.wallet.clone();
    let path = last_address_path(taker);
    let (addr_type, label) = address_kind(address_type);
    tokio::task::spawn_blocking(move || -> Result<NewAddress, AppError> {
        let mut cached = load_last_addresses(&path);
        let existing = address_slot(&mut cached, addr_type).clone();

        if let Some(existing) = existing {
            let used = wallet
                .read()?
                .get_transactions(Some(USED_ADDRESS_LOOKBACK), None)?
                .into_iter()
                .any(|tx| {
                    tx.detail
                        .address
                        .is_some_and(|a| a.assume_checked().to_string() == existing)
                });
            if !used {
                return Ok(NewAddress {
                    address: existing,
                    address_type: label.to_string(),
                    verified: true,
                });
            }
        }

        let address = wallet
            .write()?
            .get_next_external_address(addr_type)?
            .to_string();
        *address_slot(&mut cached, addr_type) = Some(address.clone());
        save_last_addresses(&path, &cached)?;
        Ok(NewAddress {
            address,
            address_type: label.to_string(),
            verified: true,
        })
    })
    .await
    .map_err(AppError::internal)?
}

fn address_kind(dto: AddressTypeDto) -> (AddressType, &'static str) {
    match dto {
        AddressTypeDto::P2wpkh => (AddressType::P2WPKH, "p2wpkh"),
        AddressTypeDto::P2tr => (AddressType::P2TR, "p2tr"),
    }
}

fn address_slot(cached: &mut LastAddresses, addr_type: AddressType) -> &mut Option<String> {
    match addr_type {
        AddressType::P2WPKH => &mut cached.p2wpkh,
        AddressType::P2TR => &mut cached.p2tr,
    }
}

pub async fn get_transactions(
    taker: &TakerInstance,
    count: Option<usize>,
    skip: Option<usize>,
) -> Result<Vec<TxSummary>, AppError> {
    let wallet = taker.wallet.clone();
    tokio::task::spawn_blocking(move || -> Result<Vec<TxSummary>, AppError> {
        let txs = wallet.read()?.get_transactions(count, skip)?;
        Ok(txs
            .into_iter()
            .map(|tx| TxSummary {
                txid: tx.info.txid.to_string(),
                category: format!("{:?}", tx.detail.category).to_lowercase(),
                amount_sats: tx.detail.amount.to_sat(),
                confirmations: tx.info.confirmations,
                address: tx.detail.address.map(|a| a.assume_checked().to_string()),
                time: tx.info.time,
                fee_sats: tx.detail.fee.map(|f| f.to_sat()),
                label: tx.detail.label,
            })
            .collect())
    })
    .await
    .map_err(AppError::internal)?
}

pub async fn list_utxos(taker: &TakerInstance) -> Result<Vec<UtxoEntry>, AppError> {
    let wallet = taker.wallet.clone();
    let backend = taker.chain_backend.clone();
    let socks_port = Some(taker.socks_port);
    tokio::task::spawn_blocking(move || -> Result<Vec<UtxoEntry>, AppError> {
        let utxos = wallet.read()?.list_all_utxo_spend_info();
        Ok(utxos
            .into_iter()
            .map(|(entry, spend_info)| UtxoEntry {
                txid: entry.txid.to_string(),
                vout: entry.vout,
                amount_sats: entry.amount.to_sat(),
                confirmations: entry.confirmations,
                address: chain_backend::utxo_address(&entry, &backend, socks_port),
                spendable: entry.spendable,
                solvable: entry.solvable,
                spend_type: spend_info.to_string(),
            })
            .collect())
    })
    .await
    .map_err(AppError::internal)?
}

/// `fee_rate` defaults to 2 sat/vB when omitted.
pub async fn send_to_address(
    state: &Arc<AppState>,
    taker: &TakerInstance,
    address: String,
    amount_sats: u64,
    fee_rate: Option<f64>,
    outpoints: Option<Vec<Outpoint>>,
) -> Result<SendResult, AppError> {
    let address = address.trim().to_string();
    if address.is_empty() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "recipient address cannot be empty",
        ));
    }
    if amount_sats == 0 {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "send amount must be greater than zero",
        ));
    }
    if fee_rate.is_some_and(|rate| !rate.is_finite() || rate <= 0.0 || rate > 10_000.0) {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "fee rate must be finite and between 0 and 10,000 sat/vB",
        ));
    }
    let _operation = SensitiveOperationGuard::acquire(
        &state.sensitive_operation_active,
        SensitiveOperation::SendTakerFunds,
    )?;
    let wallet = taker.wallet.clone();
    let outpoints = outpoints
        .map(|list| {
            if list.len() > 10_000 {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "too many selected inputs",
                ));
            }
            list.into_iter()
                .map(|o| -> Result<OutPoint, AppError> {
                    let txid = Txid::from_str(&o.txid)
                        .map_err(|e| AppError::new(ErrorCode::InvalidInput, e.to_string()))?;
                    Ok(OutPoint::new(txid, o.vout))
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    if outpoints.as_ref().is_some_and(|items| {
        items.iter().collect::<std::collections::HashSet<_>>().len() != items.len()
    }) {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "selected inputs contain duplicates",
        ));
    }

    tokio::task::spawn_blocking(move || -> Result<SendResult, AppError> {
        let txid = wallet
            .write()?
            .send_to_address(amount_sats, address, fee_rate, outpoints)?;
        Ok(SendResult {
            txid: txid.to_string(),
        })
    })
    .await
    .map_err(AppError::internal)?
}

pub async fn sync_wallet(taker: &TakerInstance) -> Result<(), AppError> {
    let wallet = taker.wallet.clone();
    let cancel = taker.sync_cancel.clone();
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        wallet.write()?.sync_and_save(&cancel)?;
        Ok(())
    })
    .await
    .map_err(AppError::internal)?
}

/// Hits mempool.space over clearnet regardless of Tor setting.
///
/// Mainnet rates even when the wallet is on signet — mempool.space has per-network
/// endpoints, but a signet rate prices nothing, so the mainnet structure is what gets
/// reported and the caller picks from it.
///
/// Deliberately not the crate's `FeeEstimator`: it averages mempool.space with
/// Blockstream's `/fee-estimates`, which blends historical data and returns
/// sub-1 sat/vB rates, dragging the mean below the 1 sat/vB relay minimum so the
/// resulting transaction can't propagate. Each of its `get_*_priority_rate`
/// calls also re-runs the whole fan-out, costing six HTTP requests per refresh.
pub async fn estimate_fees() -> Result<FeeEstimate, AppError> {
    tokio::task::spawn_blocking(|| -> Result<FeeEstimate, AppError> {
        let response = minreq::get("https://mempool.space/api/v1/fees/recommended")
            .with_timeout(10)
            .send()
            .map_err(AppError::internal)?;
        if !(200..300).contains(&response.status_code) {
            return Err(AppError::new(
                ErrorCode::Internal,
                format!("fee service returned HTTP {}", response.status_code),
            ));
        }
        let body: serde_json::Value = response.json().map_err(AppError::internal)?;
        // Passed through unclamped: these are the three targets mempool.space quotes, and
        // adjusting them would report a rate it never gave us.
        Ok(FeeEstimate {
            high: read_fee(&body, "fastestFee")?,
            mid: read_fee(&body, "halfHourFee")?,
            low: read_fee(&body, "hourFee")?,
        })
    })
    .await
    .map_err(AppError::internal)?
}

fn read_fee(body: &serde_json::Value, key: &str) -> Result<f64, AppError> {
    body.get(key)
        .and_then(serde_json::Value::as_f64)
        // Under the 1 sat/vB relay minimum the value is unusable rather than merely low, so
        // it is rejected like a missing field instead of being rounded up into a fiction.
        .filter(|rate| rate.is_finite() && *rate >= 1.0)
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::Internal,
                format!("fee response missing a valid `{key}` value"),
            )
        })
}

/// Hits mempool.space/api/v1/prices over clearnet, same as estimate_fees — public market data,
/// not swap-sensitive, so it isn't routed through Tor. A successful quote is saved locally;
/// a later network failure falls back to that last known value across app restarts.
pub async fn get_btc_price() -> Result<PriceEstimate, AppError> {
    tokio::task::spawn_blocking(|| -> Result<PriceEstimate, AppError> {
        let live_quote = (|| -> Result<CachedBtcPrice, AppError> {
            let response = minreq::get("https://mempool.space/api/v1/prices")
                .with_timeout(10)
                .send()
                .map_err(AppError::internal)?;
            if !(200..300).contains(&response.status_code) {
                return Err(AppError::new(
                    ErrorCode::Internal,
                    format!("price service returned HTTP {}", response.status_code),
                ));
            }
            let body: serde_json::Value = response.json().map_err(AppError::internal)?;
            let usd = body
                .get("USD")
                .and_then(serde_json::Value::as_f64)
                .filter(|price| valid_usd_price(*price))
                .ok_or_else(|| {
                    AppError::new(
                        ErrorCode::Internal,
                        "price response missing a valid USD value".to_string(),
                    )
                })?;
            Ok(CachedBtcPrice {
                usd,
                fetched_at: unix_timestamp(),
            })
        })();

        match live_quote {
            Ok(quote) => {
                if let Err(error) = save_cached_btc_price(&quote) {
                    log::warn!("could not save BTC/USD price cache: {error:?}");
                }
                Ok(PriceEstimate {
                    usd: quote.usd,
                    cached: false,
                    fetched_at: quote.fetched_at,
                })
            }
            Err(live_error) => match load_cached_btc_price() {
                Ok(Some(quote)) => {
                    log::warn!(
                        "live BTC/USD price unavailable; using cached quote from {}: {live_error:?}",
                        quote.fetched_at
                    );
                    Ok(PriceEstimate {
                        usd: quote.usd,
                        cached: true,
                        fetched_at: quote.fetched_at,
                    })
                }
                Ok(None) => Err(live_error),
                Err(cache_error) => {
                    log::warn!("could not read BTC/USD price cache: {cache_error:?}");
                    Err(live_error)
                }
            },
        }
    })
    .await
    .map_err(AppError::internal)?
}
