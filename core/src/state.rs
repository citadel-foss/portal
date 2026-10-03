use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
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
    /// The chain route the server was built against, pinned for the same reason a taker's is.
    pub chain_backend: ChainBackendConfig,
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

/// Identifies one client of the runtime: a browser session on the web, the window on desktop.
/// Wallets are bound to sessions, never to the process, so two browsers each see only the
/// wallet they unlocked.
pub type SessionId = String;

/// The desktop app's one and only session.
pub const DESKTOP_SESSION: &str = "desktop";

/// One unlocked wallet and everything its Taker needs. Keyed by the wallet's own data dir:
/// the crate keeps its swap tracker and offerbook per data dir, so two Takers can only share a
/// process safely when each has a directory to itself.
pub struct TakerInstance {
    pub wallet_name: String,
    /// The crate's `data_dir` for this wallet, and the registry key.
    pub data_dir: PathBuf,
    /// The root the wallet was listed from, which is what the frontend keys its caches by.
    pub root: PathBuf,
    /// `None` only once a release has taken it for dropping.
    pub taker: Arc<Mutex<Option<Taker>>>,
    /// Cached from `taker.get_wallet()` so wallet reads never contend with the taker mutex.
    pub wallet: Arc<RwLock<Wallet>>,
    pub offer_sync: OfferSyncClient,
    /// Exact chain route this Taker was built against. A later gate choice in another
    /// session never changes it.
    pub chain_backend: ChainBackendConfig,
    pub socks_port: u16,
    /// One swap at a time per wallet.
    pub active_swap: Mutex<Option<ActiveSwap>>,
    /// Aborts an in-flight `Wallet::sync_and_save`. The crate's `sync_no_fail` retries a failing
    /// backend forever and only exits on success or this flag, so without it an Electrum outage
    /// pins a blocking thread for the rest of the process's life.
    pub sync_cancel: Arc<AtomicBool>,
    /// Held for as long as a sync's blocking worker runs, which outlives the call that started
    /// it: a sync abandoned at its deadline keeps the wallet's write lock until it notices the
    /// cancel flag, and a second worker started meanwhile would simply queue behind that lock
    /// with a deadline of its own.
    pub sync_in_flight: Arc<AtomicBool>,
    /// Own bookkeeping for syncs we trigger — the crate doesn't expose this
    /// on the public OfferSyncClient.
    pub is_offerbook_syncing: AtomicBool,
    /// Sessions currently looking at this wallet. The Taker is dropped when the last one leaves,
    /// unless a swap is still running.
    pub sessions: Mutex<HashSet<SessionId>>,
    /// Held for the Taker's whole life, including its release, so another process on the same
    /// files cannot open this wallet while it is still being written.
    pub(crate) dir_lock: Mutex<Option<crate::storage::WalletDirLock>>,
}

impl TakerInstance {
    pub fn swap_running(&self) -> bool {
        self.active_swap
            .lock()
            .is_ok_and(|swap| swap.as_ref().is_some_and(|s| s.phase == SwapLifecycle::Running))
    }
}

pub(crate) enum TakerSlot {
    /// `Taker::init` is running for this wallet. Anyone else asking for it waits rather than
    /// starting a second one over the same files.
    Opening,
    Open(Arc<TakerInstance>),
}

#[derive(Default)]
pub struct AppState {
    pub(crate) takers: Mutex<HashMap<PathBuf, TakerSlot>>,
    /// Which wallet each session is on. Set as soon as an open begins, so the session also
    /// receives that wallet's initialization progress.
    pub(crate) bindings: Mutex<HashMap<SessionId, PathBuf>>,
    /// Takers dropping on a background thread. Reopening one waits here first, or its final
    /// `save_to_disk` could overwrite the wallet the new Taker just loaded.
    pub(crate) releasing: Mutex<HashMap<PathBuf, JoinHandle<()>>>,
    /// Serializes native approval/file dialogs and their immediately following
    /// private-key or fund-moving operation.
    pub sensitive_operation_active: Arc<AtomicBool>,
    /// Rust-owned local file choices. The renderer receives only a random ID.
    pub pending_file_selections: Mutex<HashMap<Uuid, PendingFileSelection>>,
    /// Maker registrations keyed by stable maker ID. Persisted registrations
    /// are loaded into this map on demand; no maker auto-starts at app launch.
    pub makers: Arc<Mutex<HashMap<String, MakerHandle>>>,
    /// Domain events. Each host subscribes and forwards these onto its own transport, so
    /// publishers never learn whether they are talking to a webview or a browser.
    pub events: EventSink,
}

impl AppState {
    /// The wallet this session has unlocked.
    ///
    /// A binding alone is not enough: a session waiting on another's init is bound before its
    /// own password has been checked. Membership is only granted by opening or joining, both of
    /// which check the password first.
    pub fn taker_for(&self, session: &str) -> Result<Arc<TakerInstance>, AppError> {
        let key = self
            .bindings
            .lock()?
            .get(session)
            .cloned()
            .ok_or_else(AppError::not_initialized)?;
        let instance = match self.takers.lock()?.get(&key) {
            Some(TakerSlot::Open(instance)) => instance.clone(),
            _ => return Err(AppError::not_initialized()),
        };
        if !instance.sessions.lock()?.contains(session) {
            return Err(AppError::not_initialized());
        }
        Ok(instance)
    }

    /// The wallet a session is bound to, open or still opening. Used to scope events.
    pub fn wallet_of(&self, session: &str) -> Option<PathBuf> {
        self.bindings.lock().ok()?.get(session).cloned()
    }

    /// Every open wallet, for the quit paths.
    pub fn open_takers(&self) -> Vec<Arc<TakerInstance>> {
        self.takers
            .lock()
            .map(|takers| {
                takers
                    .values()
                    .filter_map(|slot| match slot {
                        TakerSlot::Open(instance) => Some(instance.clone()),
                        TakerSlot::Opening => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
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
    pub outgoing: Arc<OutgoingConfirmation>,
    pub funding_waits: Arc<Mutex<FundingWaits>>,
}

/// The txids each funding wait of the running swap polled, one group per leg in route order:
/// the swap waits on its legs one at a time, and on every split of a leg at once. Read off the
/// crate's wait log, because a leg's contract reaches the tracker only after it confirms, and a
/// Legacy leg's never does: its tracker txid is a contract broadcast only on failure.
#[derive(Default)]
pub struct FundingWaits {
    pub legs: Vec<Vec<String>>,
    complete: bool,
}

impl FundingWaits {
    /// Each round re-checks every txid of the leg, so one coming round again means the leg's
    /// group is whole, and the next unseen txid is the next leg's. A later wait that re-checks
    /// an earlier leg (Legacy waits on the previous hop again) repeats known txids only.
    pub fn observe(&mut self, txid: &str) {
        if self.legs.iter().flatten().any(|seen| seen == txid) {
            self.complete = true;
            return;
        }
        match self.legs.last_mut() {
            Some(leg) if !self.complete => leg.push(txid.to_string()),
            _ => {
                self.legs.push(vec![txid.to_string()]);
                self.complete = false;
            }
        }
    }
}

/// Whether our taproot funding has confirmed. Finding out is an Electrum round trip, so it runs
/// off the progress poll's request path, one check at a time, and latches: one failed or
/// skipped check must not walk the circuit back a hop.
#[derive(Default)]
pub struct OutgoingConfirmation {
    pub confirmed: AtomicBool,
    pub checking: AtomicBool,
}

pub struct PendingFileSelection {
    /// Canonical local path chosen by the Rust-owned file dialog.
    pub path: PathBuf,
    /// The host wrote this file itself (a web upload) rather than the user pointing at one of
    /// theirs, so it is deleted once the restore is done with it.
    pub staged: bool,
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

#[cfg(test)]
mod funding_waits_tests {
    use super::FundingWaits;

    #[test]
    fn groups_each_legs_splits_and_ignores_rewaits_on_earlier_legs() {
        let mut waits = FundingWaits::default();
        // Leg 0: two splits polled round after round.
        for txid in ["a", "b", "a", "b", "a"] {
            waits.observe(txid);
        }
        // Leg 1 starts; Legacy then re-checks leg 0 before moving on.
        for txid in ["c", "d", "c", "a", "b", "d"] {
            waits.observe(txid);
        }
        // Leg 2: a single split.
        for txid in ["e", "e"] {
            waits.observe(txid);
        }
        assert_eq!(waits.legs, vec![vec!["a", "b"], vec!["c", "d"], vec!["e"]]);
    }
}
