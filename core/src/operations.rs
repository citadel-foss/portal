//! Admission for the web host's long-running operations, held in memory only.
//!
//! The problem this exists for: a browser sends a spend, the response is lost, and the user
//! presses the button again. Without a record of what was already accepted, the second press
//! is a second spend. So acceptance is recorded *before* any worker runs, keyed by a UUID the
//! client generated, and a repeat of that key returns the first outcome instead of starting
//! new work.
//!
//! Nothing is written to disk: once an operation has settled, the wallet and the chain are
//! the record, and a copy kept here would only be a trail of addresses and amounts. So a
//! record lives only as long as a lost response can still be chasing it, and a restart
//! forgets everything.
//!
//! What this cannot do is make external execution exactly-once. Broadcasting a transaction
//! and recording that we broadcast it are two steps. That case resolves to
//! [`OperationState::Indeterminate`], which blocks conflicting spends until a human or a
//! chain read settles it — never to a silent retry.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{AppError, ErrorCode};

/// How long a settled record is kept: long enough for a browser that lost the response to
/// come back for it, and no longer.
const SETTLED_RETENTION_SECS: u64 = 15 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationState {
    /// Recorded, not yet owned by a worker.
    Accepted,
    Running,
    Succeeded,
    /// Proven not to have happened.
    Failed,
    /// The effect may have happened and we cannot prove otherwise.
    Indeterminate,
}

impl OperationState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationRecord {
    pub operation_id: String,
    pub kind: String,
    pub wallet_id: Option<String>,
    pub generation: u64,
    /// Hash of the non-secret request. A repeat of the same key with different inputs is a
    /// client bug or an attack, never a retry, so it is refused rather than served.
    pub fingerprint: String,
    pub state: OperationState,
    pub created_at: u64,
    pub updated_at: u64,
    /// The request with secret-bearing fields removed, so a blocked user can be told which
    /// payment is holding things up rather than just that one is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<serde_json::Value>,
    /// Safe result metadata — a txid, a report reference. Never a key or a password.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AppError>,
    /// The owner has seen this unresolved outcome and accepted it. Deliberately separate
    /// from `state`: the outcome is still unknown, and recording it as failed or succeeded
    /// would assert something nobody can prove. Only the gate consults this.
    pub acknowledged: bool,
}

/// Operations that can put a transaction on-chain, and so could conflict with another spend
/// while their outcome is unknown.
///
/// `recover_swap` is deliberately absent. It also broadcasts, but it is the remedy for a
/// stuck swap — blocking it would strand the funds it exists to reclaim.
pub fn moves_funds(kind: &str) -> bool {
    matches!(kind, "send_to_address" | "send_maker_to_address" | "start_swap")
}

fn is_unsettled(record: &OperationRecord) -> bool {
    record.state == OperationState::Indeterminate
}

/// Whether a record has outlived its use. Settled ones go after the retention window; an
/// unresolved one only once the owner has acknowledged it, since until then it is what keeps
/// a second spend from the same wallet out. Work still running is never dropped.
fn expired(record: &OperationRecord, now: u64) -> bool {
    let done = record.state.is_terminal() || (is_unsettled(record) && record.acknowledged);
    done && now.saturating_sub(record.updated_at) >= SETTLED_RETENTION_SECS
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// Field names whose values must never reach the journal. Matched case-insensitively and
/// ignoring `_`, so `walletPassword`, `wallet_password` and `WALLETPASSWORD` all strip.
const SECRET_FIELDS: &[&str] = &[
    "password",
    "walletpassword",
    "newpassword",
    "confirmpassword",
    "passphrase",
    "secret",
    "bootstrapsecret",
    "rpcpassword",
    "torauthpassword",
    "controlpassword",
    "privatekey",
    "seed",
    "mnemonic",
];

fn is_secret_field(name: &str) -> bool {
    let flattened: String = name
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect();
    SECRET_FIELDS.contains(&flattened.as_str())
}

/// Strips secret-bearing fields at every depth, leaving the shape intact so two genuinely
/// different requests still differ.
fn without_secrets(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .filter(|(key, _)| !is_secret_field(key))
                .map(|(key, nested)| (key.clone(), without_secrets(nested)))
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(without_secrets).collect())
        }
        other => other.clone(),
    }
}

/// Stable, non-secret digest of a request.
///
/// Secrets are stripped here rather than trusted to every caller: this is a short, unsalted
/// digest, so a password reaching it would be a guessing oracle. Dropping the credential also
/// makes a retry with a corrected password match its original key, which is what the caller
/// wants anyway.
pub fn fingerprint(kind: &str, payload: &serde_json::Value) -> String {
    let payload = &without_secrets(payload);
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    kind.hash(&mut hasher);
    // Serialized through serde_json so map ordering is canonical rather than insertion-order.
    serde_json::to_string(payload).unwrap_or_default().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// What admission decided. `Replayed` is the whole point of the mechanism.
#[derive(Debug)]
pub enum Admission {
    /// Newly accepted; the caller now owns execution.
    Accepted(OperationRecord),
    /// This exact key and request were already accepted; here is what happened.
    Replayed(OperationRecord),
}

/// One per web host process. Writes are serialized through a single lock so two admissions
/// cannot interleave a read-modify-write on the same key.
#[derive(Debug, Default)]
pub struct Journal {
    index: Mutex<HashMap<String, OperationRecord>>,
}

impl Journal {
    /// The gate every durable operation passes through.
    pub fn admit(
        &self,
        operation_id: &str,
        kind: &str,
        wallet_id: Option<String>,
        generation: u64,
        request: &serde_json::Value,
    ) -> Result<Admission, AppError> {
        if uuid::Uuid::parse_str(operation_id).is_err() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "operation id must be a client-generated UUID",
            ));
        }
        let fingerprint = fingerprint(kind, request);
        let mut index = self.index.lock()?;
        let now = now();
        index.retain(|_, record| !expired(record, now));

        if let Some(existing) = index.get(operation_id) {
            if existing.fingerprint != fingerprint || existing.kind != kind {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "this operation id was already used for a different request",
                ));
            }
            return Ok(Admission::Replayed(existing.clone()));
        }

        let record = OperationRecord {
            operation_id: operation_id.to_string(),
            kind: kind.to_string(),
            wallet_id,
            generation,
            fingerprint,
            request: Some(without_secrets(request)),
            state: OperationState::Accepted,
            created_at: now,
            updated_at: now,
            result: None,
            error: None,
            acknowledged: false,
        };
        index.insert(record.operation_id.clone(), record.clone());
        Ok(Admission::Accepted(record))
    }

    fn update(
        &self,
        operation_id: &str,
        apply: impl FnOnce(&mut OperationRecord),
    ) -> Result<(), AppError> {
        let mut index = self.index.lock()?;
        let current = index.get(operation_id).ok_or_else(|| {
            AppError::new(ErrorCode::InvalidInput, "no such operation")
        })?;
        if current.state.is_terminal() {
            // A resolved financial key can never execute again.
            return Ok(());
        }
        let mut next = current.clone();
        apply(&mut next);
        next.updated_at = now();
        index.insert(operation_id.to_string(), next);
        Ok(())
    }

    pub fn mark_running(&self, operation_id: &str) -> Result<(), AppError> {
        self.update(operation_id, |r| r.state = OperationState::Running)
    }

    pub fn mark_succeeded(
        &self,
        operation_id: &str,
        result: serde_json::Value,
    ) -> Result<(), AppError> {
        self.update(operation_id, |r| {
            r.state = OperationState::Succeeded;
            r.result = Some(result);
        })
    }

    /// Only for a failure proven not to have had an effect. A timeout or a dropped connection
    /// is not proof, and must use [`Self::mark_indeterminate`] instead.
    pub fn mark_failed(&self, operation_id: &str, error: AppError) -> Result<(), AppError> {
        self.update(operation_id, |r| {
            r.state = OperationState::Failed;
            r.error = Some(error);
        })
    }

    pub fn mark_indeterminate(&self, operation_id: &str, error: AppError) -> Result<(), AppError> {
        self.update(operation_id, |r| {
            r.state = OperationState::Indeterminate;
            r.error = Some(error);
        })
    }

    pub fn get(&self, operation_id: &str) -> Option<OperationRecord> {
        self.index.lock().ok()?.get(operation_id).cloned()
    }

    /// Newest first, bounded — the operations list is a UI view, not an export.
    pub fn recent(&self, limit: usize) -> Vec<OperationRecord> {
        let Ok(mut index) = self.index.lock() else {
            return Vec::new();
        };
        let now = now();
        index.retain(|_, record| !expired(record, now));
        let mut all: Vec<_> = index.values().cloned().collect();
        all.sort_unstable_by(|a, b| b.created_at.cmp(&a.created_at));
        all.truncate(limit);
        all
    }

    /// Unresolved work that could actually conflict with a new spend from `wallet_id`. Empty
    /// is the normal case. A record with no wallet recorded
    /// conflicts with every wallet, since nothing says which one it spent from.
    pub fn blocking_conflicts(&self, wallet_id: Option<&str>) -> Vec<OperationRecord> {
        let Ok(index) = self.index.lock() else {
            return Vec::new();
        };
        index
            .values()
            .filter(|r| moves_funds(&r.kind) && is_unsettled(r) && !r.acknowledged)
            .filter(|r| r.wallet_id.is_none() || r.wallet_id.as_deref() == wallet_id)
            .cloned()
            .collect()
    }

    /// Settles a record against evidence the caller gathered.
    ///
    /// Only ever moves a record to `Succeeded`. Nothing available here can prove a broadcast
    /// did *not* happen, so an absent txid leaves the record exactly as it was.
    pub fn reconcile(
        &self,
        operation_id: &str,
        known_txids: &[String],
    ) -> Result<OperationRecord, AppError> {
        let record = self
            .get(operation_id)
            .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "no such operation"))?;
        let txid = record
            .result
            .as_ref()
            .and_then(|result| result.get("txid"))
            .and_then(|txid| txid.as_str())
            .map(str::to_string);
        if let Some(txid) = txid {
            if known_txids.iter().any(|known| known == &txid) {
                self.update(operation_id, |r| r.state = OperationState::Succeeded)?;
            }
        }
        self.get(operation_id)
            .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "no such operation"))
    }

    /// Records that the owner has seen an unresolved outcome and accepts it, so it stops
    /// holding up new work. The state is left alone: the outcome is still unknown.
    /// Not routed through `update`: acknowledging is only ever done to a *terminal* record,
    /// which `update` deliberately refuses to touch.
    pub fn acknowledge(&self, operation_id: &str) -> Result<OperationRecord, AppError> {
        let mut index = self.index.lock()?;
        let current = index
            .get(operation_id)
            .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "no such operation"))?;
        let mut next = current.clone();
        next.acknowledged = true;
        next.updated_at = now();
        index.insert(operation_id.to_string(), next.clone());
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    fn request() -> serde_json::Value {
        serde_json::json!({ "address": "bc1qexample", "amountSats": 10_000 })
    }

    #[test]
    fn a_key_must_be_a_client_generated_uuid() {
        let j = Journal::default();
        assert!(j.admit("not-a-uuid", "send", None, 1, &request()).is_err());
    }

    /// The whole point: a repeated submission returns the first outcome rather than spending
    /// twice.
    #[test]
    fn the_same_key_and_request_replays_instead_of_re_executing() {
        let j = Journal::default();
        let id = key();
        let first = j.admit(&id, "send", None, 1, &request()).unwrap();
        assert!(matches!(first, Admission::Accepted(_)));
        j.mark_succeeded(&id, serde_json::json!({ "txid": "abc" })).unwrap();

        match j.admit(&id, "send", None, 1, &request()).unwrap() {
            Admission::Replayed(record) => {
                assert_eq!(record.state, OperationState::Succeeded);
                assert_eq!(record.result.unwrap()["txid"], "abc");
            }
            Admission::Accepted(_) => panic!("a resolved key must never execute again"),
        }
    }

    #[test]
    fn the_same_key_with_different_inputs_is_refused() {
        let j = Journal::default();
        let id = key();
        j.admit(&id, "send", None, 1, &request()).unwrap();
        let other = serde_json::json!({ "address": "bc1qattacker", "amountSats": 10_000 });
        assert!(j.admit(&id, "send", None, 1, &other).is_err());
        // A different operation kind under the same key is equally not a retry.
        assert!(j.admit(&id, "swap", None, 1, &request()).is_err());
    }

    /// The lockout came from gating on operations that cannot conflict with a spend.
    #[test]
    fn only_fund_moving_operations_block() {
        let j = Journal::default();
        let init = key();
        j.admit(&init, "init_taker", None, 1, &request()).unwrap();
        j.mark_indeterminate(&init, AppError::new(ErrorCode::TorUnreachable, "tor down"))
            .unwrap();
        assert!(
            j.blocking_conflicts(None).is_empty(),
            "an unresolved unlock cannot conflict with a spend"
        );

        let send = key();
        j.admit(&send, "send_to_address", None, 1, &request()).unwrap();
        j.mark_indeterminate(&send, AppError::new(ErrorCode::Io, "connection lost"))
            .unwrap();
        assert_eq!(j.blocking_conflicts(None).len(), 1);
    }

    /// A stuck payment holds up its own wallet's spends, not another wallet's: they share no
    /// coins, so neither can double-spend the other's.
    #[test]
    fn an_unresolved_spend_blocks_only_its_own_wallet() {
        let j = Journal::default();
        let send = key();
        j.admit(&send, "send_to_address", Some("a".into()), 1, &request()).unwrap();
        j.mark_indeterminate(&send, AppError::new(ErrorCode::Io, "connection lost"))
            .unwrap();
        assert_eq!(j.blocking_conflicts(Some("a")).len(), 1);
        assert!(j.blocking_conflicts(Some("b")).is_empty());
    }

    /// Recovery is the remedy for a stuck swap. Blocking it would strand the funds it exists
    /// to reclaim.
    #[test]
    fn recovery_is_never_treated_as_conflicting() {
        assert!(!moves_funds("recover_swap"));
        assert!(moves_funds("send_to_address"));
        assert!(moves_funds("start_swap"));
        assert!(!moves_funds("init_taker"));
    }

    #[test]
    fn reconcile_settles_only_with_evidence() {
        let j = Journal::default();
        let id = key();
        j.admit(&id, "send_to_address", None, 1, &request()).unwrap();
        j.mark_running(&id).unwrap();
        // A result was written, then the outcome became unknowable.
        j.update(&id, |r| r.result = Some(serde_json::json!({ "txid": "abc" })))
            .unwrap();
        j.mark_indeterminate(&id, AppError::new(ErrorCode::Io, "lost")).unwrap();

        let unchanged = j.reconcile(&id, &["other".to_string()]).unwrap();
        assert_eq!(unchanged.state, OperationState::Indeterminate);

        let settled = j.reconcile(&id, &["abc".to_string()]).unwrap();
        assert_eq!(settled.state, OperationState::Succeeded);
        assert!(j.blocking_conflicts(None).is_empty());
    }

    /// Acknowledgement must not claim an outcome nobody can prove.
    #[test]
    fn acknowledging_clears_the_gate_without_claiming_success() {
        let j = Journal::default();
        let id = key();
        j.admit(&id, "send_to_address", None, 1, &request()).unwrap();
        j.mark_running(&id).unwrap();
        j.mark_indeterminate(&id, AppError::new(ErrorCode::Io, "lost")).unwrap();
        assert_eq!(j.blocking_conflicts(None).len(), 1);

        let acknowledged = j.acknowledge(&id).unwrap();
        assert_eq!(acknowledged.state, OperationState::Indeterminate);
        assert!(acknowledged.acknowledged);
        assert!(j.blocking_conflicts(None).is_empty());
    }

    #[test]
    fn an_indeterminate_outcome_is_not_a_failure() {
        let j = Journal::default();
        let id = key();
        j.admit(&id, "send", None, 1, &request()).unwrap();
        j.mark_indeterminate(&id, AppError::new(ErrorCode::Io, "connection lost after broadcast"))
            .unwrap();
        assert_eq!(j.get(&id).unwrap().state, OperationState::Indeterminate);
    }

    #[test]
    fn a_resolved_record_cannot_be_rewritten() {
        let j = Journal::default();
        let id = key();
        j.admit(&id, "send", None, 1, &request()).unwrap();
        j.mark_succeeded(&id, serde_json::json!({ "txid": "abc" })).unwrap();
        j.mark_failed(&id, AppError::new(ErrorCode::Io, "late error")).unwrap();
        assert_eq!(j.get(&id).unwrap().state, OperationState::Succeeded);
    }

    /// Settled records are forgotten once a lost response can no longer be chasing them; an
    /// unresolved spend stays until acknowledged, and running work is never dropped.
    #[test]
    fn records_expire_only_once_they_are_done() {
        let j = Journal::default();
        let settled = key();
        let unresolved = key();
        let running = key();
        for id in [&settled, &unresolved, &running] {
            j.admit(id, "send_to_address", None, 1, &request()).unwrap();
        }
        j.mark_succeeded(&settled, serde_json::json!({ "txid": "abc" })).unwrap();
        j.mark_indeterminate(&unresolved, AppError::new(ErrorCode::Io, "lost")).unwrap();
        j.mark_running(&running).unwrap();
        for record in j.index.lock().unwrap().values_mut() {
            record.updated_at -= SETTLED_RETENTION_SECS;
        }

        let left: Vec<String> = j.recent(10).into_iter().map(|r| r.operation_id).collect();
        assert!(!left.contains(&settled), "settled and past retention");
        assert!(left.contains(&unresolved), "still guarding the wallet");
        assert!(left.contains(&running));

        j.acknowledge(&unresolved).unwrap();
        j.index.lock().unwrap().get_mut(&unresolved).unwrap().updated_at -= SETTLED_RETENTION_SECS;
        assert!(j.recent(10).iter().all(|r| r.operation_id != unresolved));
    }

    /// The digest is short and unsalted, so a password reaching it would be a guessing oracle.
    /// Stripping also means a corrected password still matches its original key rather than
    /// reading as a different request.
    #[test]
    fn secrets_never_reach_the_fingerprint() {
        let a = serde_json::json!({ "walletName": "w", "walletPassword": "hunter2" });
        let b = serde_json::json!({ "walletName": "w", "walletPassword": "different" });
        assert_eq!(fingerprint("init_taker", &a), fingerprint("init_taker", &b));

        // Spelling variants and nesting must not slip past.
        let nested = serde_json::json!({ "cfg": { "rpc_password": "p", "host": "h" } });
        let other = serde_json::json!({ "cfg": { "rpc_password": "q", "host": "h" } });
        assert_eq!(fingerprint("init", &nested), fingerprint("init", &other));

        // A different non-secret field still changes the digest.
        let elsewhere = serde_json::json!({ "cfg": { "rpc_password": "p", "host": "other" } });
        assert_ne!(fingerprint("init", &nested), fingerprint("init", &elsewhere));
    }

    #[test]
    fn fingerprints_are_canonical_and_input_sensitive() {
        let a = serde_json::json!({ "address": "bc1q", "amountSats": 1 });
        let b = serde_json::json!({ "amountSats": 1, "address": "bc1q" });
        assert_eq!(fingerprint("send", &a), fingerprint("send", &b));
        assert_ne!(fingerprint("send", &a), fingerprint("swap", &a));
        assert_ne!(
            fingerprint("send", &a),
            fingerprint("send", &serde_json::json!({ "address": "bc1q", "amountSats": 2 }))
        );
    }
}
