//! Portal's own record of every swap a wallet starts, kept beside the wallet as `swaps.journal`.
//!
//! openswap keeps a swap's progress private, so the record is folded from the lines the crate
//! logs on the swap's own thread (`observe`). A line that stops matching after a crate bump
//! leaves a stage behind, never wrong: stages and phases only move forward.
//!
//! It links the wallet's on-chain legs to its routers and amounts, so it is sealed like the
//! wallet: AES-GCM under a key derived from the wallet password.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use openswap::security::{decrypt_struct, encrypt_struct, EncryptedData, KeyMaterial};

use crate::error::{AppError, ErrorCode};
use crate::types::{RouterProgressDto, RouterStageDto, SwapPhaseDto, SwapSummaryDto, SwapTrackerDto};

const JOURNAL_FILE: &str = "swaps.journal";

/// The sealed journal with the salt its key is derived from. `EncryptedData` keeps its own salt
/// private, so the next unlock could not re-derive the key without this copy.
#[derive(serde::Serialize, serde::Deserialize)]
struct SealedJournal {
    salt: [u8; 16],
    sealed: EncryptedData,
}

/// Serializes every read-modify-write of a journal: the swap thread and the recovery reads both
/// save into the same file.
static JOURNAL_WRITE: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwapOutcome {
    Running,
    Completed,
    /// Failed before anything reached the chain; nothing to recover.
    Aborted,
    Recovering,
    Recovered,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SwapEntry {
    pub swap_id: String,
    pub protocol: String,
    pub send_amount_sats: u64,
    pub routers: Vec<String>,
    pub stages: Vec<RouterStageDto>,
    pub phase: SwapPhaseDto,
    pub outcome: SwapOutcome,
    pub outgoing_contract_txids: Vec<String>,
    pub outgoing_confirmed: bool,
    /// The crate began recovering this swap rather than aborting it.
    pub recovery_started: bool,
    pub failure_reason: Option<String>,
    pub failed_at_phase: Option<SwapPhaseDto>,
    pub payment_address: Option<String>,
    pub payment_amount_sats: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The number right after `prefix`, as in "Waiting for maker 2's funding".
fn index_after(line: &str, prefix: &str) -> Option<usize> {
    let rest = line.strip_prefix(prefix)?;
    let digits = rest.split(|c: char| !c.is_ascii_digit()).next()?;
    digits.parse().ok()
}

impl SwapEntry {
    pub fn start(summary: &SwapSummaryDto) -> Self {
        let now = now_secs();
        Self {
            swap_id: summary.swap_id.clone(),
            protocol: summary.protocol.clone(),
            send_amount_sats: summary.send_amount_sats,
            routers: summary.routers.iter().map(|r| r.address.clone()).collect(),
            stages: vec![RouterStageDto::Negotiated; summary.routers.len()],
            phase: SwapPhaseDto::Negotiated,
            outcome: SwapOutcome::Running,
            outgoing_contract_txids: Vec::new(),
            outgoing_confirmed: false,
            recovery_started: false,
            failure_reason: None,
            failed_at_phase: None,
            payment_address: summary.payment.as_ref().map(|p| p.address.clone()),
            payment_amount_sats: summary.payment.as_ref().map(|p| p.amount_sats),
            created_at: now,
            updated_at: now,
        }
    }

    fn reach(&mut self, phase: SwapPhaseDto) {
        self.phase = self.phase.max(phase);
    }

    /// The route is walked in order, so reaching router `index` means every router before it
    /// is past the same pass: routed during the exchange, settled during the key handover.
    ///
    /// The exchange never moves the phase: Legacy talks to the first router before anything is
    /// broadcast, and only the broadcast lines may say funds are on-chain.
    fn hop(&mut self, index: usize, stage: RouterStageDto) {
        if index >= self.stages.len() {
            return;
        }
        let behind = if stage >= RouterStageDto::KeyReceived {
            self.reach(SwapPhaseDto::Finalizing);
            RouterStageDto::Settled
        } else {
            RouterStageDto::Routed
        };
        for earlier in &mut self.stages[..index] {
            *earlier = (*earlier).max(behind);
        }
        self.stages[index] = self.stages[index].max(stage);
    }

    fn all(&mut self, stage: RouterStageDto) {
        for s in &mut self.stages {
            *s = (*s).max(stage);
        }
    }

    /// Folds one crate log line in, reporting whether anything changed.
    pub fn observe(&mut self, line: &str) -> bool {
        use RouterStageDto::*;
        let before = (
            self.phase,
            self.stages.clone(),
            self.routers.clone(),
            self.outgoing_contract_txids.len(),
            self.outgoing_confirmed,
            self.recovery_started,
        );
        if line.starts_with("Created ") && line.ends_with(" outgoing swapcoins for funding") {
            self.reach(SwapPhaseDto::FundingCreated);
        } else if line == "Broadcasting contract transactions..."
            || line == "Broadcasting funding transactions and waiting for confirmation"
        {
            self.reach(SwapPhaseDto::FundsBroadcast);
        } else if let Some(txid) = line.strip_prefix("Broadcast contract tx: ") {
            self.reach(SwapPhaseDto::FundsBroadcast);
            if !self.outgoing_contract_txids.iter().any(|t| t == txid) {
                self.outgoing_contract_txids.push(txid.to_string());
            }
        } else if line == "Contract transactions broadcast and confirmed"
            || line == "Sending ProofOfFunding to maker 0"
        {
            // Taproot logs the first after waiting on our own funding; Legacy sends the first
            // router its proof only once ours has confirmed.
            self.outgoing_confirmed = true;
        } else if let Some(i) = index_after(line, "Sending contract data to maker ") {
            self.hop(i, Handshaking);
        } else if let Some(n) = index_after(line, "Processing maker ") {
            // The one count the crate logs from 1.
            if let Some(i) = n.checked_sub(1) {
                self.hop(i, Handshaking);
            }
        } else if let Some(i) = index_after(line, "Waiting for maker ") {
            self.hop(i, Confirming);
        } else if let Some(i) = index_after(line, "Maker ").filter(|_| {
            line.contains(" funding confirmed at height ") || line.ends_with(" processed successfully")
        }) {
            self.hop(i, Routed);
        } else if line == "Multi-hop Legacy swap contract exchange completed" {
            self.reach(SwapPhaseDto::ContractsExchanged);
            self.all(Routed);
        } else if line == "Finalizing swap..." {
            self.reach(SwapPhaseDto::Finalizing);
            self.all(Routed);
        } else if let Some(i) = index_after(line, "Sending privkey to maker ") {
            self.hop(i, KeyReceived);
        } else if let Some(i) = index_after(line, "Received private key from maker ") {
            self.hop(i, Settled);
        } else if line == "Swap finalized successfully" {
            self.reach(SwapPhaseDto::PrivkeysForwarded);
            self.all(Settled);
        } else if line.starts_with("OpenSwap completed successfully") {
            self.reach(SwapPhaseDto::Completed);
        } else if line == "Starting swap recovery..." {
            self.recovery_started = true;
        } else if let Some(i) = index_after(line, "Substituting maker ")
            .or_else(|| index_after(line, "Next maker ").filter(|_| line.contains("Substituting")))
        {
            if let (Some(address), Some(slot)) =
                (line.rsplit("spare at ").next(), self.routers.get_mut(i))
            {
                *slot = address.trim().to_string();
                self.stages[i] = Negotiated;
            }
        }
        let after = (
            self.phase,
            self.stages.clone(),
            self.routers.clone(),
            self.outgoing_contract_txids.len(),
            self.outgoing_confirmed,
            self.recovery_started,
        );
        before != after
    }

    /// Records how `start_swap` ended.
    pub fn settle(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.reach(SwapPhaseDto::Completed);
                self.outcome = SwapOutcome::Completed;
            }
            Err(reason) => {
                // Funds on-chain go to recovery even when its own log line was missed.
                self.outcome = if self.recovery_started || self.phase >= SwapPhaseDto::FundsBroadcast {
                    SwapOutcome::Recovering
                } else {
                    SwapOutcome::Aborted
                };
                self.failed_at_phase = Some(self.phase);
                self.phase = SwapPhaseDto::Failed;
                self.failure_reason = Some(reason);
            }
        }
        self.updated_at = now_secs();
    }

    /// The outcome, with a swap still marked running that this process isn't running counted as
    /// what the crate made of it at the next unlock: recovery for anything already on-chain.
    pub fn effective_outcome(&self, live_swap: Option<&str>) -> SwapOutcome {
        match self.outcome {
            SwapOutcome::Running if live_swap != Some(self.swap_id.as_str()) => {
                if self.phase >= SwapPhaseDto::FundsBroadcast {
                    SwapOutcome::Recovering
                } else {
                    SwapOutcome::Aborted
                }
            }
            outcome => outcome,
        }
    }

    /// The point the swap reached, failed or not.
    pub fn reached(&self) -> SwapPhaseDto {
        self.failed_at_phase.unwrap_or(self.phase)
    }

    pub fn to_tracker_dto(&self, funding_wait_txids: Vec<Vec<String>>) -> SwapTrackerDto {
        SwapTrackerDto {
            phase: self.phase,
            send_amount_sats: self.send_amount_sats,
            router_count: self.routers.len(),
            failure_reason: self.failure_reason.clone(),
            routers: self
                .routers
                .iter()
                .zip(&self.stages)
                .map(|(address, stage)| RouterProgressDto {
                    address: address.clone(),
                    stage: *stage,
                })
                .collect(),
            outgoing_contract_txids: self.outgoing_contract_txids.clone(),
            outgoing_confirmed: self.outgoing_confirmed,
            protocol: self.protocol.clone(),
            funding_wait_txids,
            payment_address: self.payment_address.clone(),
            payment_amount_sats: self.payment_amount_sats,
        }
    }
}

fn read_sealed(dir: &Path) -> Result<Option<SealedJournal>, AppError> {
    let path = dir.join(JOURNAL_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => serde_cbor::from_slice(&bytes).map(Some).map_err(|e| {
            AppError::internal(format!("cannot read the swap journal {}: {e}", path.display()))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// The journal's key, derived once per unlock. A journal that can't be read gets a fresh key,
/// which no save can use: `save` refuses while the file on disk doesn't load.
pub fn derive_key(dir: &Path, password: Option<String>) -> Result<KeyMaterial, AppError> {
    let password = password.ok_or_else(|| {
        AppError::new(ErrorCode::WalletWrongPassword, "a wallet password is required")
    })?;
    Ok(match read_sealed(dir).ok().flatten() {
        // Encryption takes a fresh nonce every time; this one is never used.
        Some(file) => KeyMaterial::existing(password, [0; 12], file.salt),
        None => KeyMaterial::new_from_password(Some(password))
            .ok_or_else(|| AppError::internal("no key material from a password"))?,
    })
}

/// Every swap this wallet has journaled, by id. Only a missing journal is empty: one that can't
/// be read is an error, so no save ever overwrites the history it holds.
fn load(dir: &Path, key: &KeyMaterial) -> Result<BTreeMap<String, SwapEntry>, AppError> {
    match read_sealed(dir)? {
        Some(file) => decrypt_struct(file.sealed, key).map_err(|e| {
            AppError::internal(format!("cannot open the swap journal: {e}"))
        }),
        None => Ok(BTreeMap::new()),
    }
}

/// For views that only add journal detail to what the wallet itself says: an unreadable journal
/// leaves them the wallet's view rather than none at all.
pub fn load_for_view(dir: &Path, key: &KeyMaterial) -> BTreeMap<String, SwapEntry> {
    load(dir, key).unwrap_or_else(|e| {
        log::warn!("showing swaps without the journal: {}", e.message);
        BTreeMap::new()
    })
}

/// Writes one entry. Never logs: the swap thread calls this from inside the logger.
pub fn save(dir: &Path, key: &KeyMaterial, entry: &SwapEntry) -> Result<(), AppError> {
    let _write = JOURNAL_WRITE.lock()?;
    let mut all = load(dir, key)?;
    all.insert(entry.swap_id.clone(), entry.clone());
    write_all(dir, key, &all)
}

/// Applies `change` to every entry and saves once if any changed.
pub fn update_all(
    dir: &Path,
    key: &KeyMaterial,
    mut change: impl FnMut(&mut SwapEntry) -> bool,
) -> Result<(), AppError> {
    let _write = JOURNAL_WRITE.lock()?;
    let mut all = load(dir, key)?;
    let mut changed = false;
    for entry in all.values_mut() {
        changed |= change(entry);
    }
    if changed {
        write_all(dir, key, &all)?;
    }
    Ok(())
}

fn write_all(
    dir: &Path,
    key: &KeyMaterial,
    all: &BTreeMap<String, SwapEntry>,
) -> Result<(), AppError> {
    let sealed = encrypt_struct(all, key).map_err(|e| AppError::internal(format!("{e:?}")))?;
    let bytes = serde_cbor::to_vec(&SealedJournal {
        salt: key.pbkdf2_salt,
        sealed,
    })
    .map_err(AppError::internal)?;
    // Written aside and renamed in, so a crash mid-write leaves the previous journal whole.
    let staged = dir.join(format!("{JOURNAL_FILE}.partial"));
    crate::security::fs::write_private(&staged, &bytes)?;
    std::fs::rename(&staged, dir.join(JOURNAL_FILE))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RouterFeeInfoDto, SwapSummaryDto};

    fn entry(routers: usize) -> SwapEntry {
        SwapEntry::start(&SwapSummaryDto {
            swap_id: "s".into(),
            protocol: "taproot".into(),
            send_amount_sats: 100_000,
            routers: (0..routers)
                .map(|i| RouterFeeInfoDto {
                    address: format!("r{i}.onion"),
                    protocol: "taproot".into(),
                    base_fee: 0,
                    amount_relative_fee_pct: 0.0,
                    time_relative_fee_pct: 0.0,
                    locktime: 0,
                    estimated_fee_sats: 0,
                })
                .collect(),
            total_estimated_fee_sats: 0,
            estimated_receive_amount_sats: 0,
            router_fee_sats: 0,
            mining_fee_sats: 0,
            payment: None,
        })
    }

    fn stages(e: &SwapEntry) -> Vec<RouterStageDto> {
        e.stages.clone()
    }

    /// Lines in the order openswap logs them for a two-router Taproot swap.
    #[test]
    fn a_taproot_swap_walks_the_route_twice() {
        use RouterStageDto::*;
        let mut e = entry(2);
        e.observe("Created 2 outgoing swapcoins for funding");
        assert_eq!(e.phase, SwapPhaseDto::FundingCreated);
        e.observe("Broadcasting contract transactions...");
        e.observe("Broadcast contract tx: aa");
        e.observe("Broadcast contract tx: bb");
        assert_eq!(e.outgoing_contract_txids, ["aa", "bb"]);
        assert!(e.observe("Contract transactions broadcast and confirmed"));
        assert!(e.outgoing_confirmed);
        e.observe("Sending contract data to maker 0: 2 pubkeys, 2 contract_txs");
        e.observe("Received Taproot contract data from maker 0: 2 contract_txs");
        e.observe("Waiting for maker 0's Taproot funding to confirm (2 tx), keeping the swap alive...");
        assert_eq!(stages(&e), [Confirming, Negotiated]);
        e.observe("Sending contract data to maker 1: 2 pubkeys, 2 contract_txs");
        assert_eq!(stages(&e), [Routed, Handshaking]);
        e.observe("Finalizing swap...");
        assert_eq!(stages(&e), [Routed, Routed]);
        e.observe("Sending privkey to maker 0 and awaiting response");
        e.observe("Received private key from maker 0");
        e.observe("Sending privkey to maker 1 and awaiting response");
        assert_eq!(stages(&e), [Settled, KeyReceived]);
        e.observe("Swap finalized successfully");
        assert_eq!(e.phase, SwapPhaseDto::PrivkeysForwarded);
        assert_eq!(stages(&e), [Settled, Settled]);
        e.observe("OpenSwap completed successfully: TakerReport { .. }");
        assert_eq!(e.phase, SwapPhaseDto::Completed);
    }

    #[test]
    fn legacy_counts_its_routers_from_one_and_confirms_each_hop() {
        use RouterStageDto::*;
        let mut e = entry(2);
        e.observe("Processing maker 1 of 2: r0.onion");
        assert_eq!(stages(&e), [Handshaking, Negotiated]);
        e.observe("Broadcasting funding transactions and waiting for confirmation");
        assert!(!e.outgoing_confirmed);
        e.observe("Sending ProofOfFunding to maker 0");
        assert!(e.outgoing_confirmed);
        e.observe("Waiting for maker 0's funding to be confirmed...");
        e.observe("Maker 0 funding confirmed at height 120 (1 blocks after previous hop)");
        assert_eq!(stages(&e), [Routed, Negotiated]);
        e.observe("Processing maker 2 of 2: r1.onion");
        e.observe("Maker 1 processed successfully");
        e.observe("Multi-hop Legacy swap contract exchange completed");
        assert_eq!(e.phase, SwapPhaseDto::ContractsExchanged);
        assert_eq!(stages(&e), [Routed, Routed]);
    }

    /// Legacy connects to its first router before funding; failing there leaves nothing to
    /// recover.
    #[test]
    fn talking_to_a_router_before_funding_is_not_a_recovery() {
        let mut e = entry(2);
        e.observe("Processing maker 1 of 2: r0.onion");
        e.settle(Err("connection refused".into()));
        assert_eq!(e.outcome, SwapOutcome::Aborted);
    }

    #[test]
    fn a_substituted_router_takes_its_slot() {
        let mut e = entry(2);
        assert!(e.observe("Substituting maker 1 with spare at spare.onion:6102"));
        assert_eq!(e.routers, ["r0.onion", "spare.onion:6102"]);
        e.observe("Next maker 0 failed: Timeout. Substituting with spare at other.onion");
        assert_eq!(e.routers[0], "other.onion");
    }

    #[test]
    fn unrelated_lines_and_stale_indexes_change_nothing() {
        let mut e = entry(1);
        assert!(!e.observe("Sync Started for \"w\""));
        assert!(!e.observe("Waiting for maker 7's funding to be confirmed..."));
    }

    #[test]
    fn a_failure_after_broadcast_goes_to_recovery_and_one_before_does_not() {
        let mut early = entry(1);
        early.settle(Err("refused".into()));
        assert_eq!(early.outcome, SwapOutcome::Aborted);
        assert_eq!(early.failed_at_phase, Some(SwapPhaseDto::Negotiated));

        let mut late = entry(1);
        late.observe("Broadcasting contract transactions...");
        late.settle(Err("timeout".into()));
        assert_eq!(late.outcome, SwapOutcome::Recovering);
        assert_eq!(late.phase, SwapPhaseDto::Failed);
        assert_eq!(late.reached(), SwapPhaseDto::FundsBroadcast);
    }

    #[test]
    fn a_swap_whose_process_died_counts_as_recovering_once_funded() {
        let mut e = entry(1);
        assert_eq!(e.effective_outcome(Some("s")), SwapOutcome::Running);
        assert_eq!(e.effective_outcome(None), SwapOutcome::Aborted);
        e.observe("Broadcast contract tx: aa");
        assert_eq!(e.effective_outcome(None), SwapOutcome::Recovering);
    }

    #[test]
    fn the_journal_round_trips_through_its_file() {
        let dir = std::env::temp_dir().join(format!("portal-journal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let key = derive_key(&dir, Some("pw".into())).unwrap();
        let mut e = entry(1);
        save(&dir, &key, &e).unwrap();
        e.settle(Ok(()));
        save(&dir, &key, &e).unwrap();
        let all = load(&dir, &key).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all["s"].outcome, SwapOutcome::Completed);
        update_all(&dir, &key, |entry| {
            entry.outcome = SwapOutcome::Recovered;
            true
        })
        .unwrap();
        assert_eq!(load(&dir, &key).unwrap()["s"].outcome, SwapOutcome::Recovered);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The next unlock re-derives the key from the password alone; nothing readable is on disk.
    #[test]
    fn the_journal_is_sealed_under_the_wallet_password() {
        let dir = std::env::temp_dir().join(format!("portal-journal-seal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        save(&dir, &derive_key(&dir, Some("pw".into())).unwrap(), &entry(1)).unwrap();
        let bytes = std::fs::read(dir.join(JOURNAL_FILE)).unwrap();
        assert!(!bytes.windows(8).any(|w| w == b"r0.onion"), "router address in the clear");

        let reopened = derive_key(&dir, Some("pw".into())).unwrap();
        assert_eq!(load(&dir, &reopened).unwrap().len(), 1);
        let wrong = derive_key(&dir, Some("not it".into())).unwrap();
        assert!(load(&dir, &wrong).is_err());
        assert!(save(&dir, &wrong, &entry(1)).is_err(), "a wrong key never overwrites");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A journal that can't be parsed must never be replaced by a save of one swap.
    #[test]
    fn an_unreadable_journal_is_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("portal-journal-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let key = derive_key(&dir, Some("pw".into())).unwrap();
        assert!(load(&dir, &key).unwrap().is_empty(), "no journal yet is an empty one");
        std::fs::write(dir.join(JOURNAL_FILE), b"not cbor").unwrap();
        assert!(load(&dir, &key).is_err());
        assert!(load_for_view(&dir, &key).is_empty(), "views fall back to the wallet's own data");
        assert!(save(&dir, &key, &entry(1)).is_err());
        assert_eq!(std::fs::read(dir.join(JOURNAL_FILE)).unwrap(), b"not cbor");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
