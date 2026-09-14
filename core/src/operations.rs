//! Durable admission for operations that move money or change persistent state.
//!
//! The problem this exists for: a browser sends a spend, the response is lost, and the user
//! presses the button again. Without a durable record of what was already accepted, the
//! second press is a second spend. So acceptance is written to disk *before* any worker runs,
//! keyed by a UUID the client generated, and a repeat of that key returns the first outcome
//! instead of starting new work.
//!
//! What this cannot do is make external execution exactly-once. Broadcasting a transaction
//! and recording that we broadcast it are two steps, and a process can die between them. That
//! case resolves to [`OperationState::Indeterminate`], which blocks conflicting spends until
//! a human or a chain read settles it — never to a silent retry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{AppError, ErrorCode};

/// Bumped when the record shape changes incompatibly; a newer schema is refused rather than
/// silently misread.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperationState {
    /// Persisted, not yet owned by a worker. A crash here leaves `Interrupted`.
    Accepted,
    Running,
    Succeeded,
    /// Proven not to have happened.
    Failed,
    /// The process stopped while this was accepted or running.
    Interrupted,
    /// The effect may have happened and we cannot prove otherwise.
    Indeterminate,
}

impl OperationState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationRecord {
    pub schema_version: u32,
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
    /// Safe result metadata — a txid, a report reference. Never a key or a password.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AppError>,
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
/// Secrets are stripped here rather than trusted to every caller: this is a short,
/// unsalted digest written to disk, so a password reaching it would turn the journal into
/// an offline guessing oracle for anyone who can read the data directory. Dropping the
/// credential also makes a retry with a corrected password match its original key, which is
/// what the caller wants anyway.
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
    /// Newly accepted and persisted; the caller now owns execution.
    Accepted(OperationRecord),
    /// This exact key and request were already accepted; here is what happened.
    Replayed(OperationRecord),
}

/// One journal per data root. Writes are serialized through a single lock so two admissions
/// cannot interleave a read-modify-write on the same key.
#[derive(Debug)]
pub struct Journal {
    dir: PathBuf,
    index: Mutex<HashMap<String, OperationRecord>>,
}

impl Journal {
    /// Loads existing records and marks anything left mid-flight as interrupted. A crash
    /// never turns into an automatic retry: the record survives so the next reader can
    /// reconcile it deliberately.
    pub fn open(root: &Path) -> Result<Self, AppError> {
        let dir = root.join("portal").join("operations");
        crate::security::fs::ensure_private_dir(&dir)?;
        let mut index = HashMap::new();
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = std::fs::read(&path)?;
            let mut record: OperationRecord = serde_json::from_slice(&bytes).map_err(|e| {
                // Refused, not discarded: a corrupt record may be the only trace of a spend.
                AppError::new(
                    ErrorCode::Io,
                    format!("operation record {} is unreadable: {e}", path.display()),
                )
            })?;
            if record.schema_version > SCHEMA_VERSION {
                return Err(AppError::new(
                    ErrorCode::Io,
                    format!(
                        "operation record {} was written by a newer Portal (schema {})",
                        path.display(),
                        record.schema_version
                    ),
                ));
            }
            if matches!(record.state, OperationState::Accepted | OperationState::Running) {
                // Running means a worker owned it when the process died. Whether the effect
                // landed is exactly what we cannot know here.
                record.state = OperationState::Interrupted;
                record.updated_at = now();
            }
            index.insert(record.operation_id.clone(), record);
        }
        Ok(Journal {
            dir,
            index: Mutex::new(index),
        })
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Replaces a record atomically: written to a private temporary file, synced, then
    /// renamed over the target. A torn write would be worse than no write at all, and the
    /// plain truncating helper cannot give that guarantee.
    fn persist(&self, record: &OperationRecord) -> Result<(), AppError> {
        let target = self.path_for(&record.operation_id);
        let temp = target.with_extension("json.partial");
        let body = serde_json::to_vec(record).map_err(AppError::internal)?;
        crate::security::fs::write_private(&temp, &body)?;
        {
            let file = std::fs::File::open(&temp)?;
            file.sync_all()?;
        }
        std::fs::rename(&temp, &target)?;
        // Directory sync so the rename itself survives power loss, not just the bytes.
        if let Ok(dir) = std::fs::File::open(&self.dir) {
            let _ = dir.sync_all();
        }
        Ok(())
    }

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

        if let Some(existing) = index.get(operation_id) {
            if existing.fingerprint != fingerprint || existing.kind != kind {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "this operation id was already used for a different request",
                ));
            }
            // Deliberately before any generation check: a browser that reconnects after a
            // restart must still be able to read what its pre-restart operation did. New
            // effects are what require a current generation, not reading an old outcome.
            return Ok(Admission::Replayed(existing.clone()));
        }

        let record = OperationRecord {
            schema_version: SCHEMA_VERSION,
            operation_id: operation_id.to_string(),
            kind: kind.to_string(),
            wallet_id,
            generation,
            fingerprint,
            state: OperationState::Accepted,
            created_at: now(),
            updated_at: now(),
            result: None,
            error: None,
        };
        // Persisted before the caller is told it owns the work: a crash between these two
        // must leave evidence, never a silent gap.
        self.persist(&record)?;
        index.insert(record.operation_id.clone(), record.clone());
        Ok(Admission::Accepted(record))
    }

    fn update(
        &self,
        operation_id: &str,
        apply: impl FnOnce(&mut OperationRecord),
    ) -> Result<(), AppError> {
        let mut index = self.index.lock()?;
        let record = index.get_mut(operation_id).ok_or_else(|| {
            AppError::new(ErrorCode::InvalidInput, "no such operation")
        })?;
        if record.state.is_terminal() {
            // A resolved financial key can never execute again.
            return Ok(());
        }
        apply(record);
        record.updated_at = now();
        let snapshot = record.clone();
        drop(index);
        self.persist(&snapshot)
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
        let Ok(index) = self.index.lock() else {
            return Vec::new();
        };
        let mut all: Vec<_> = index.values().cloned().collect();
        all.sort_unstable_by(|a, b| b.created_at.cmp(&a.created_at));
        all.truncate(limit);
        all
    }

    /// True while any operation could still have moved funds without us knowing. New spending
    /// is refused until that is settled.
    pub fn has_unresolved(&self) -> bool {
        self.index.lock().is_ok_and(|index| {
            index.values().any(|r| {
                matches!(
                    r.state,
                    OperationState::Indeterminate | OperationState::Interrupted
                )
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal() -> (Journal, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "portal-journal-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        (Journal::open(&root).unwrap(), root)
    }

    fn key() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    fn request() -> serde_json::Value {
        serde_json::json!({ "address": "bc1qexample", "amountSats": 10_000 })
    }

    #[test]
    fn a_key_must_be_a_client_generated_uuid() {
        let (j, _) = journal();
        assert!(j.admit("not-a-uuid", "send", None, 1, &request()).is_err());
    }

    /// The whole point: a repeated submission returns the first outcome rather than spending
    /// twice.
    #[test]
    fn the_same_key_and_request_replays_instead_of_re_executing() {
        let (j, _) = journal();
        let id = key();
        let first = j.admit(&id, "send", None, 1, &request()).unwrap();
        assert!(matches!(first, Admission::Accepted(_)));
        j.mark_succeeded(&id, serde_json::json!({ "txid": "abc" })).unwrap();

        let again = j.admit(&id, "send", None, 1, &request()).unwrap();
        match again {
            Admission::Replayed(record) => {
                assert_eq!(record.state, OperationState::Succeeded);
                assert_eq!(record.result.unwrap()["txid"], "abc");
            }
            Admission::Accepted(_) => panic!("a resolved key must never execute again"),
        }
    }

    #[test]
    fn the_same_key_with_different_inputs_is_refused() {
        let (j, _) = journal();
        let id = key();
        j.admit(&id, "send", None, 1, &request()).unwrap();
        let other = serde_json::json!({ "address": "bc1qattacker", "amountSats": 10_000 });
        assert!(j.admit(&id, "send", None, 1, &other).is_err());
        // A different operation kind under the same key is equally not a retry.
        assert!(j.admit(&id, "swap", None, 1, &request()).is_err());
    }

    /// A lost response must be recoverable with the key the client already has, even after a
    /// restart that moved the generation on.
    #[test]
    fn a_pre_restart_operation_is_readable_by_its_original_key() {
        let (j, root) = journal();
        let id = key();
        j.admit(&id, "send", None, 1, &request()).unwrap();
        j.mark_succeeded(&id, serde_json::json!({ "txid": "abc" })).unwrap();

        let reopened = Journal::open(&root).unwrap();
        match reopened.admit(&id, "send", None, 99, &request()).unwrap() {
            Admission::Replayed(record) => assert_eq!(record.state, OperationState::Succeeded),
            Admission::Accepted(_) => panic!("must not re-execute across a restart"),
        }
    }

    #[test]
    fn a_crash_while_running_reopens_as_interrupted_not_as_retryable() {
        let (j, root) = journal();
        let id = key();
        j.admit(&id, "send", None, 1, &request()).unwrap();
        j.mark_running(&id).unwrap();
        drop(j);

        let reopened = Journal::open(&root).unwrap();
        let record = reopened.get(&id).unwrap();
        assert_eq!(record.state, OperationState::Interrupted);
        assert!(reopened.has_unresolved(), "must block new spending until settled");
        // And it still replays rather than starting fresh work.
        assert!(matches!(
            reopened.admit(&id, "send", None, 1, &request()).unwrap(),
            Admission::Replayed(_)
        ));
    }

    #[test]
    fn an_indeterminate_outcome_is_not_a_failure() {
        let (j, _) = journal();
        let id = key();
        j.admit(&id, "send", None, 1, &request()).unwrap();
        j.mark_indeterminate(&id, AppError::new(ErrorCode::Io, "connection lost after broadcast"))
            .unwrap();
        assert_eq!(j.get(&id).unwrap().state, OperationState::Indeterminate);
        assert!(j.has_unresolved());
    }

    #[test]
    fn a_resolved_record_cannot_be_rewritten() {
        let (j, _) = journal();
        let id = key();
        j.admit(&id, "send", None, 1, &request()).unwrap();
        j.mark_succeeded(&id, serde_json::json!({ "txid": "abc" })).unwrap();
        j.mark_failed(&id, AppError::new(ErrorCode::Io, "late error")).unwrap();
        assert_eq!(j.get(&id).unwrap().state, OperationState::Succeeded);
    }

    #[test]
    fn a_record_from_a_newer_schema_is_refused_rather_than_misread() {
        let (j, root) = journal();
        let id = key();
        j.admit(&id, "send", None, 1, &request()).unwrap();
        let path = root.join("portal").join("operations").join(format!("{id}.json"));
        let mut record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        record["schemaVersion"] = serde_json::json!(SCHEMA_VERSION + 1);
        std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(Journal::open(&root).is_err());
    }

    #[test]
    fn a_corrupt_record_blocks_rather_than_being_discarded() {
        let (j, root) = journal();
        let id = key();
        j.admit(&id, "send", None, 1, &request()).unwrap();
        let path = root.join("portal").join("operations").join(format!("{id}.json"));
        std::fs::write(&path, b"{ truncated").unwrap();
        assert!(Journal::open(&root).is_err());
    }

    /// The journal is on disk and the digest is short and unsalted, so a password reaching
    /// it would be an offline guessing oracle. Stripping also means a corrected password
    /// still matches its original key rather than reading as a different request.
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

    /// Secrets must never reach the journal, so the fingerprint is over what the caller
    /// passed and nothing more.
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
