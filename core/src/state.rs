use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, TryLockError};
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime};

use openswap::maker::MakerServer;
use openswap::taker::offers::OfferSyncClient;
use openswap::taker::Taker;
use openswap::wallet::{SecretMnemonic, Wallet};
use uuid::Uuid;

use crate::error::{AppError, ErrorCode};
use crate::events::EventSink;
use crate::types::{ChainBackendConfig, MakerPhase, MakerSettingsDto, QuitBlockers};

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

/// A new wallet's recovery phrase, held from creation until its creator says it is written down.
/// The crate hands it out exactly once, so losing it here — a reload, a dropped response, a sign
/// out — would lose it for good. Memory only, read only by the creating session, and dropped with
/// that session so nothing waits on a confirmation no one can give.
pub struct PendingPhrase {
    /// Sent back with the confirmation, so a page still showing another phrase cannot clear this one.
    pub id: Uuid,
    pub words: SecretMnemonic,
    pub session: SessionId,
    /// Its unsaved-phrase marker, cleared on confirmation.
    pub marker: PathBuf,
    /// The wallet's or router's data dir lock, so another Portal on this machine, which knows
    /// nothing of the phrase, cannot open or start it meanwhile. Shared with a wallet's open
    /// Taker, so a sign out releases the dir only once the phrase is gone too.
    pub(crate) dir_lock: Arc<crate::storage::WalletDirLock>,
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
    pub pending_phrase: Option<PendingPhrase>,
}

/// Identifies one client of the runtime: a browser session on the web, the window on desktop.
/// Wallets are bound to sessions, never to the process, so two browsers each see only the
/// wallet they unlocked.
pub type SessionId = String;

/// The desktop app's one and only session.
pub const DESKTOP_SESSION: &str = "desktop";

/// One unlocked wallet and everything its Taker needs. Keyed by the wallet's own data dir:
/// the crate keeps its offerbook and config, and Portal its swap journal, per data dir, so two
/// Takers can only share a process safely when each has a directory to itself.
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
    /// Seals this wallet's swap journal, derived from its password at unlock.
    pub journal_key: openswap::security::KeyMaterial,
    pub offer_sync: OfferSyncClient,
    /// Exact chain route this Taker was built against. A later gate choice in another
    /// session never changes it.
    pub chain_backend: ChainBackendConfig,
    pub socks_port: u16,
    /// One swap at a time per wallet.
    pub active_swap: Mutex<Option<ActiveSwap>>,
    /// How far a `prepare_swap` in flight has got. It holds the taker for its whole run, so its
    /// own log lines are the only progress there is.
    pub preparation: Arc<Mutex<Option<crate::types::SwapPreparationDto>>>,
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
    /// Routers with a poll still queued in the crate, including one whose caller gave up waiting:
    /// a retry would only queue a second poll behind it.
    pub polls_in_flight: Arc<Mutex<HashSet<String>>>,
    /// Sessions currently looking at this wallet. The Taker is dropped when the last one leaves,
    /// unless a swap is still running.
    pub sessions: Mutex<HashSet<SessionId>>,
    /// Held for the Taker's whole life, including its release, so another process on the same
    /// files cannot open this wallet while it is still being written.
    pub(crate) dir_lock: Mutex<Option<Arc<crate::storage::WalletDirLock>>>,
}

impl TakerInstance {
    pub fn swap_running(&self) -> bool {
        self.running_swap_id().is_some()
    }

    pub fn running_swap_id(&self) -> Option<String> {
        self.active_swap.lock().ok().and_then(|swap| {
            swap.as_ref()
                .filter(|s| s.phase == SwapLifecycle::Running)
                .map(|s| s.swap_id.clone())
        })
    }
}

pub(crate) enum TakerSlot {
    /// `Taker::init` is running for this wallet. Anyone else asking for it waits rather than
    /// starting a second one over the same files.
    Opening,
    Open(Arc<TakerInstance>),
}

/// States of `router_gate` and `wallet_gate`. Shutdown's close is final; an update's is undone
/// if the update is refused.
pub(crate) const GATE_OPEN: u8 = 0;
const GATE_CLOSED_FOR_UPDATE: u8 = 1;
pub(crate) const GATE_CLOSED_FOR_SHUTDOWN: u8 = 2;

// Both fail only against a shutdown's close, which must stand.
fn close_for_update(gate: &AtomicU8) {
    let _ = gate.compare_exchange(
        GATE_OPEN,
        GATE_CLOSED_FOR_UPDATE,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

fn reopen_after_update(gate: &AtomicU8) {
    let _ = gate.compare_exchange(
        GATE_CLOSED_FOR_UPDATE,
        GATE_OPEN,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
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
    /// Router admission: [`GATE_OPEN`], closed for an update that may still be refused, or
    /// closed for shutdown, which nothing reopens. A start still constructing its server at
    /// shutdown has no runtime for that pass to stop, so it checks this before installing one,
    /// as well as on admission.
    pub(crate) router_gate: AtomicU8,
    /// Wallet admission, changed under `takers`. Closed once an update is committing, so no
    /// wallet starts opening between the last idle check and the restart, and for good at
    /// shutdown, whose teardown would otherwise miss a wallet that opens after it.
    wallet_gate: AtomicU8,
    /// New wallets' phrases by wallet data dir. Kept out of `TakerInstance`, which a sign out
    /// drops while the wallet is still unsaved.
    pub(crate) pending_phrases: Mutex<HashMap<PathBuf, PendingPhrase>>,
    /// Sessions `end_session` has run for. A create still finishing when its session ends must not
    /// hold a phrase for it afterwards.
    /// Pruned after a day, which no create outlasts.
    pub(crate) ended_sessions: Mutex<HashMap<SessionId, Instant>>,
    /// Domain events. Each host subscribes and forwards these onto its own transport, so
    /// publishers never learn whether they are talking to a webview or a browser.
    pub events: EventSink,
}

impl AppState {
    /// Work that quitting or restarting would interrupt rather than finish, so the user is asked
    /// first. Blocking: each wallet read can wait out a sync.
    pub fn running_work(&self) -> QuitBlockers {
        // Read from the wallet rather than `active_swap`: recovery outlives the swap that started it
        // and survives a restart, and quitting mid-recovery is the one thing that actually stalls it.
        let recovering = |taker: &Arc<TakerInstance>| {
            taker
                .wallet
                .read()
                .is_ok_and(|w| !w.list_live_contract_spend_info().is_empty())
        };
        // Before the wallet reads as well as after: a swap that funds and fails during a read releases
        // its slot on the way into recovery, which a wallet already read would no longer show.
        let swap_at_start = self.open_takers().iter().any(|taker| taker.swap_running());
        // Until a snapshot holds no wallet left to read: each read can wait out a sync while the window
        // stays usable, and a wallet opened, or a swap or router started, meanwhile must still count.
        let mut checked: Vec<Arc<TakerInstance>> = Vec::new();
        let mut recovery_running = false;
        let takers = loop {
            let takers = self.open_takers();
            let unseen: Vec<_> = takers
                .iter()
                .filter(|taker| !checked.iter().any(|seen| Arc::ptr_eq(seen, taker)))
                .cloned()
                .collect();
            if unseen.is_empty() {
                break takers;
            }
            recovery_running = recovery_running || unseen.iter().any(recovering);
            checked.extend(unseen);
        };
        let swap_running = swap_at_start || takers.iter().any(|taker| taker.swap_running());
        QuitBlockers {
            swap_running,
            recovery_running,
            running_routers: self.running_routers(),
        }
    }

    /// Keyed on phase, not on `runtime` being present: a server thread that exits on its own
    /// sets Stopped or Failed but leaves its runtime in place, and quitting would then warn
    /// about a maker that finished long ago.
    fn running_routers(&self) -> Vec<String> {
        self.makers
            .lock()
            .map(|makers| {
                makers
                    .values()
                    .filter(|entry| {
                        matches!(
                            entry.phase,
                            MakerPhase::Restoring
                                | MakerPhase::Initializing
                                | MakerPhase::Starting
                                | MakerPhase::Running
                                | MakerPhase::Stopping
                        )
                    })
                    .map(|entry| entry.settings.router_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `running_work` without waiting, for a caller holding gates others are queued on: a wallet
    /// a sync is writing counts as busy instead of being waited out.
    pub fn idle_now(&self) -> bool {
        let wallets_idle = self.open_takers().iter().all(|taker| {
            !taker.swap_running()
                && taker
                    .wallet
                    .try_read()
                    .is_ok_and(|w| w.list_live_contract_spend_info().is_empty())
        });
        wallets_idle && self.running_routers().is_empty()
    }

    /// Stops new wallet opens and router starts ahead of a restart, unless a wallet is already
    /// opening. False leaves both open. Routers admitted earlier still show in `running_work`.
    pub fn close_for_restart(&self) -> Result<bool, AppError> {
        {
            let takers = self.takers.lock()?;
            if takers.values().any(|slot| matches!(slot, TakerSlot::Opening)) {
                return Ok(false);
            }
            close_for_update(&self.wallet_gate);
        }
        let Ok(_makers) = self.makers.lock() else {
            reopen_after_update(&self.wallet_gate);
            return Err(AppError::new(ErrorCode::StatePoisoned, "router registry lock poisoned"));
        };
        close_for_update(&self.router_gate);
        Ok(true)
    }

    /// Undoes `close_for_restart`, but never a shutdown's close.
    pub fn reopen_after_restart_refused(&self) {
        reopen_after_update(&self.wallet_gate);
        reopen_after_update(&self.router_gate);
    }

    /// Why a wallet may not start opening now, if it may not.
    pub(crate) fn wallet_opens_closed(&self) -> Option<&'static str> {
        match self.wallet_gate.load(Ordering::Acquire) {
            GATE_OPEN => None,
            GATE_CLOSED_FOR_UPDATE => Some("Portal is restarting to finish an update"),
            _ => Some("Portal is shutting down"),
        }
    }

    /// Refuses every later wallet open. Under `takers`, so no open is admitted after it.
    pub(crate) fn close_wallets_for_shutdown(&self) {
        let _takers = self.takers.lock();
        self.wallet_gate
            .store(GATE_CLOSED_FOR_SHUTDOWN, Ordering::Release);
    }

    /// Runs `hold` only while `session` is live, under the lock `end_session` marks it ended with,
    /// so a phrase is either held before that session's cleanup or never held at all.
    pub(crate) fn hold_phrase_for(
        &self,
        session: &str,
        hold: impl FnOnce(),
    ) -> Result<(), AppError> {
        let ended = self.ended_sessions.lock()?;
        if !ended.contains_key(session) {
            hold();
        }
        Ok(())
    }

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
    /// The quote `prepare_swap` returned. Router fees exist nowhere else, so without this a
    /// remounted Swap page can never redraw the per-hop amounts for a swap already in flight.
    pub summary: crate::types::SwapSummaryDto,
    pub phase: SwapLifecycle,
    pub backend_fingerprint: String,
    pub started_at: Option<SystemTime>,
    pub error: Option<String>,
    /// Saved to the wallet's swap journal from `start_swap` on.
    pub journal: Arc<Mutex<crate::ops::taker_journal::SwapEntry>>,
    pub funding_waits: Arc<Mutex<FundingWaits>>,
}

/// The txids each funding wait of the running swap polled, one group per leg in route order:
/// the swap waits on its legs one at a time, and on every split of a leg at once. Read off the
/// crate's wait log, the only place a leg's on-chain txids are named before it confirms.
#[derive(Default)]
pub struct FundingWaits {
    pub legs: Vec<Vec<String>>,
    /// The wait now running has named a txid no earlier leg holds, so it is the last leg.
    open: bool,
}

impl FundingWaits {
    /// Each leg is its own wait, so a leg begins where its wait does. Repeats can't mark the
    /// boundary: a txid is only logged while unconfirmed, and a leg confirming within one poll
    /// round never names a txid twice.
    pub fn begin_wait(&mut self) {
        self.open = false;
    }

    /// A txid no earlier leg holds belongs to the running wait's leg, even when it first shows a
    /// round late (a split not yet in our mempool is not logged). Known txids are a wait that
    /// re-checks an earlier leg (Legacy waits on the previous hop again) and add nothing.
    pub fn observe(&mut self, txid: &str) {
        if self.legs.iter().flatten().any(|seen| seen == txid) {
            return;
        }
        match self.legs.last_mut() {
            Some(leg) if self.open => leg.push(txid.to_string()),
            _ => {
                self.legs.push(vec![txid.to_string()]);
                self.open = true;
            }
        }
    }
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

    /// Each slice is one wait: the txid lines logged between two wait-start lines.
    fn replay(waits: &[&[&str]]) -> Vec<Vec<String>> {
        let mut seen = FundingWaits::default();
        for wait in waits {
            seen.begin_wait();
            for txid in *wait {
                seen.observe(txid);
            }
        }
        seen.legs
    }

    #[test]
    fn groups_each_legs_splits_and_ignores_rewaits_on_earlier_legs() {
        let legs = replay(&[
            // Leg 0: two splits polled round after round.
            &["a", "b", "a", "b", "a"],
            // Leg 1, then Legacy re-checks leg 0 in a wait of its own.
            &["c", "d", "c", "d"],
            &["a", "b"],
            // Leg 2: a single split.
            &["e", "e"],
        ]);
        assert_eq!(legs, vec![vec!["a", "b"], vec!["c", "d"], vec!["e"]]);
    }

    /// The common case on a fast chain: the first round logs each split once and the next block
    /// confirms them all, so no txid repeats. A confirmed leg must not absorb the next one.
    #[test]
    fn a_leg_confirmed_in_one_round_keeps_its_own_splits() {
        let legs = replay(&[&["a", "b"], &["c", "d"], &["e"]]);
        assert_eq!(legs, vec![vec!["a", "b"], vec!["c", "d"], vec!["e"]]);
    }

    /// A split not yet in our mempool is first logged a round late; it is still this leg's.
    #[test]
    fn a_split_arriving_late_stays_in_its_leg() {
        let legs = replay(&[&["a", "a", "b"], &["c", "c", "d", "c", "d"]]);
        assert_eq!(legs, vec![vec!["a", "b"], vec!["c", "d"]]);
    }

    /// A wait whose txids were all already confirmed logs none, and must not open a leg.
    #[test]
    fn a_wait_that_names_nothing_adds_no_leg() {
        let legs = replay(&[&["a"], &[], &["b"]]);
        assert_eq!(legs, vec![vec!["a"], vec!["b"]]);
    }
}

#[cfg(test)]
mod phrase_tests {
    use super::AppState;

    /// A create that finishes after its session ended must not hold a phrase no one can confirm.
    #[test]
    fn an_ended_session_is_never_handed_a_phrase() {
        let state = AppState::default();
        state
            .ended_sessions
            .lock()
            .unwrap()
            .insert("gone".into(), std::time::Instant::now());
        let mut held = Vec::new();
        state.hold_phrase_for("gone", || held.push("gone")).unwrap();
        state.hold_phrase_for("live", || held.push("live")).unwrap();
        assert_eq!(held, ["live"]);
    }
}

#[cfg(test)]
mod restart_gate_tests {
    use super::*;

    #[test]
    fn a_refused_update_reopens_routers_but_never_undoes_a_shutdown() {
        let state = AppState::default();
        assert!(state.close_for_restart().unwrap());
        assert!(state.wallet_opens_closed().is_some());
        assert_ne!(state.router_gate.load(Ordering::Acquire), GATE_OPEN);
        state.reopen_after_restart_refused();
        assert!(state.wallet_opens_closed().is_none());
        assert_eq!(state.router_gate.load(Ordering::Acquire), GATE_OPEN);

        assert!(state.close_for_restart().unwrap());
        state
            .router_gate
            .store(GATE_CLOSED_FOR_SHUTDOWN, Ordering::Release);
        state.close_wallets_for_shutdown();
        state.reopen_after_restart_refused();
        assert_eq!(
            state.router_gate.load(Ordering::Acquire),
            GATE_CLOSED_FOR_SHUTDOWN
        );
        assert_eq!(state.wallet_opens_closed(), Some("Portal is shutting down"));
    }

    #[test]
    fn a_wallet_still_opening_keeps_the_restart_gate_open() {
        let state = AppState::default();
        state
            .takers
            .lock()
            .unwrap()
            .insert(PathBuf::from("/wallet"), TakerSlot::Opening);
        assert!(!state.close_for_restart().unwrap());
        assert!(state.wallet_opens_closed().is_none());
        assert_eq!(state.router_gate.load(Ordering::Acquire), GATE_OPEN);
    }
}
