use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, TryLockError};
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime};

use openswap::maker::MakerServer;
use openswap::taker::offers::OfferSyncClient;
use openswap::taker::Taker;
use openswap::wallet::Wallet;
use uuid::Uuid;

use crate::error::AppError;
use crate::events::EventSink;
use crate::types::{ChainBackendConfig, MakerPhase, MakerSettingsDto};

/// Non-blocking taker lock — fails fast with SwapInProgress instead of
/// blocking for however long a running swap holds the mutex.
pub fn try_lock_taker(
    taker: &Mutex<Option<Taker>>,
) -> Result<MutexGuard<'_, Option<Taker>>, AppError> {
    match taker.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::WouldBlock) => Err(AppError::swap_in_progress()),
        Err(TryLockError::Poisoned(poisoned)) => Ok(poisoned.into_inner()),
    }
}

/// Non-blocking maker-registry lock — lifecycle transitions fail fast rather
/// than allowing two create/start/stop calls to overlap.
pub fn try_lock_makers(
    makers: &Mutex<HashMap<String, MakerHandle>>,
) -> Result<MutexGuard<'_, HashMap<String, MakerHandle>>, AppError> {
    match makers.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::WouldBlock) => Err(AppError::maker_busy()),
        Err(TryLockError::Poisoned(poisoned)) => Ok(poisoned.into_inner()),
    }
}

pub struct MakerRuntime {
    pub server: Arc<MakerServer>,
    pub thread: Option<JoinHandle<()>>,
}

/// One persisted maker registration plus its optional process-local runtime.
/// Runtime objects are reconstructed after an app restart and after every
/// explicit stop; only settings and wallet files survive those boundaries.
pub struct MakerHandle {
    pub settings: MakerSettingsDto,
    pub runtime: Option<MakerRuntime>,
    pub phase: MakerPhase,
    /// Prevents an old server/watcher thread from updating a newer lifecycle.
    pub generation: u64,
}

#[derive(Default)]
pub struct AppState {
    /// The Taker. `None` until the setup wizard completes `init_taker`.
    pub taker: Arc<Mutex<Option<Taker>>>,
    /// Cached from `taker.get_wallet()` at init.
    pub wallet: RwLock<Option<Arc<RwLock<Wallet>>>>,
    /// Cached from `taker.offer_sync_client()` at init.
    pub offer_sync: RwLock<Option<OfferSyncClient>>,
    /// Set at `init_taker`; not exposed by `Taker` itself.
    pub data_dir: RwLock<Option<PathBuf>>,
    /// Exact chain route used to construct the active taker. Saved settings may
    /// change, but this stays pinned until verified shutdown/reinitialization.
    pub active_chain_backend: RwLock<Option<ChainBackendConfig>>,
    pub active_socks_port: RwLock<Option<u16>>,
    /// Single active swap slot; one swap at a time by design.
    pub active_swap: Mutex<Option<ActiveSwap>>,
    /// Serializes native approval/file dialogs and their immediately following
    /// private-key or fund-moving operation.
    pub sensitive_operation_active: Arc<AtomicBool>,
    /// Rust-owned local file choices. The renderer receives only a random ID.
    pub pending_file_selections: Mutex<HashMap<Uuid, PendingFileSelection>>,
    /// Aborts an in-flight `Wallet::sync_and_save`. The crate's `sync_no_fail` retries a failing
    /// backend forever and only exits on success or this flag, so without it an Electrum outage
    /// pins a blocking thread for the rest of the process's life.
    pub sync_cancel: Arc<AtomicBool>,
    /// Own bookkeeping for syncs we trigger — the crate doesn't expose this
    /// on the public OfferSyncClient.
    pub is_offerbook_syncing: AtomicBool,
    pub last_offerbook_sync_ts: AtomicU64,
    /// Maker registrations keyed by stable maker ID. Persisted registrations
    /// are loaded into this map on demand; no maker auto-starts at app launch.
    pub makers: Arc<Mutex<HashMap<String, MakerHandle>>>,
    /// Domain events. Each host subscribes and forwards these onto its own transport, so
    /// publishers never learn whether they are talking to a webview or a browser.
    pub events: EventSink,
}

pub struct ActiveSwap {
    pub swap_id: String,
    /// The quote `prepare_swap` returned. Router fees exist nowhere else — not in the crate's
    /// `SwapRecord`, not in `swap_tracker.cbor` — so without this a remounted Swap page can
    /// never redraw the per-hop amounts for a swap already in flight.
    pub summary: crate::types::SwapSummaryDto,
    pub phase: SwapLifecycle,
    pub backend_fingerprint: String,
    pub started_at: Option<SystemTime>,
    pub error: Option<String>,
}

pub struct PendingFileSelection {
    /// Canonical local path chosen by the Rust-owned file dialog.
    pub path: PathBuf,
    /// Used to expire bearer-like selection IDs before a restore consumes them.
    pub created_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapLifecycle {
    Prepared,
    Running,
    Finished,
    Failed,
}
