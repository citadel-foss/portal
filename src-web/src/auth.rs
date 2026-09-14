//! Application login, sessions and CSRF.
//!
//! This credential is separate from every wallet password, always. Unlocking a wallet proves
//! you can spend; logging in proves you may reach this server at all, and conflating them
//! would put a wallet passphrase on the network path of every request.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand::RngCore;

/// Defaults, adjustable once measured against real use.
const IDLE_EXPIRY: Duration = Duration::from_secs(30 * 60);
const ABSOLUTE_EXPIRY: Duration = Duration::from_secs(12 * 60 * 60);
const MAX_SESSIONS: usize = 8;
/// Temporary, not a permanent lockout: an attacker who can reach the login page should not be
/// able to lock the owner out of their own wallet.
const LOGIN_BACKOFF: Duration = Duration::from_secs(30);
const MAX_ATTEMPTS: u32 = 10;

pub const SESSION_COOKIE: &str = "portal_session";

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Stored server-side as a digest so a memory disclosure does not hand over live cookies.
fn digest(token: &str) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    // Tokens are 256-bit random values, so this only has to be a lookup key, never a
    // password hash: there is nothing to brute-force back out of it.
    let mut hasher = DefaultHasher::new();
    token.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[derive(Debug)]
struct Session {
    csrf: String,
    created: Instant,
    last_seen: Instant,
}

#[derive(Debug, Default)]
struct Attempts {
    count: u32,
    first: Option<Instant>,
}

/// Sessions live in memory only, so a process restart revokes every one of them. That is the
/// intended behavior while wallet unlock is also memory-only: a restarted server cannot act
/// on a wallet anyway.
#[derive(Debug)]
pub struct Auth {
    /// Where a claimed owner's verifier is kept. Argon2id output only — never a password.
    store: Option<std::path::PathBuf>,
    verifier: Mutex<Option<String>>,
    bootstrap: Mutex<Option<String>>,
    sessions: Mutex<HashMap<String, Session>>,
    attempts: Mutex<Attempts>,
}

#[derive(Debug)]
pub struct Issued {
    pub token: String,
    pub csrf: String,
}

impl Auth {
    /// Loads a provisioned owner verifier and one-time bootstrap secret from private files.
    /// Neither is ever echoed back, and the bootstrap file is consumed on use.
    pub fn load(
        verifier_file: Option<&Path>,
        bootstrap_file: Option<&Path>,
        store: Option<std::path::PathBuf>,
    ) -> Self {
        let read = |path: Option<&Path>| {
            path.and_then(|p| std::fs::read_to_string(p).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        // A provisioned credential file wins: the deployment set it deliberately, and a
        // stale claim on disk must not shadow it.
        let verifier = read(verifier_file).or_else(|| read(store.as_deref()));
        Auth {
            store,
            verifier: Mutex::new(verifier),
            bootstrap: Mutex::new(read(bootstrap_file)),
            sessions: Mutex::new(HashMap::new()),
            attempts: Mutex::new(Attempts::default()),
        }
    }

    pub fn has_owner(&self) -> bool {
        self.verifier.lock().is_ok_and(|v| v.is_some())
    }

    pub fn hash_password(password: &str) -> Result<String, String> {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|e| e.to_string())
    }

    /// Consumes the bootstrap secret and installs the owner verifier, atomically: two
    /// concurrent claims cannot both succeed.
    pub fn bootstrap(&self, secret: &str, new_password: &str) -> Result<(), &'static str> {
        let mut bootstrap = self.bootstrap.lock().map_err(|_| "auth state poisoned")?;
        let mut verifier = self.verifier.lock().map_err(|_| "auth state poisoned")?;
        if verifier.is_some() {
            return Err("this installation already has an owner");
        }
        let expected = bootstrap.as_deref().ok_or("bootstrap is not enabled")?;
        if !constant_time_eq(expected, secret) {
            return Err("bootstrap secret is not valid");
        }
        let hash = Auth::hash_password(new_password).map_err(|_| "could not store credential")?;
        if let Some(path) = &self.store {
            if let Some(parent) = path.parent() {
                portal_core::security::fs::ensure_private_dir(parent)
                    .map_err(|_| "could not create the credential directory")?;
            }
            portal_core::security::fs::write_private(path, hash.as_bytes())
                .map_err(|_| "could not persist the credential")?;
        }
        *verifier = Some(hash);
        *bootstrap = None;
        Ok(())
    }

    /// Verifies a password and issues a session. Throttled globally rather than per account:
    /// there is exactly one owner, so a per-account counter would be the same thing with a
    /// lockout attached.
    pub fn login(&self, password: &str) -> Result<Issued, &'static str> {
        self.check_throttle()?;
        let stored = {
            let verifier = self.verifier.lock().map_err(|_| "auth state poisoned")?;
            verifier.clone().ok_or("this installation has no owner yet")?
        };
        let parsed = PasswordHash::new(&stored).map_err(|_| "stored credential is unreadable")?;
        if Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_err()
        {
            self.record_failure();
            // Deliberately identical to every other failure: the message must not say whether
            // an owner exists or what was wrong with the password.
            return Err("login failed");
        }
        self.clear_failures();
        self.issue()
    }

    fn issue(&self) -> Result<Issued, &'static str> {
        let mut sessions = self.sessions.lock().map_err(|_| "auth state poisoned")?;
        let now = Instant::now();
        sessions.retain(|_, s| Self::alive(s, now));
        if sessions.len() >= MAX_SESSIONS {
            // Oldest first, so a forgotten tab cannot lock the owner out of a new one.
            if let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, s)| s.last_seen)
                .map(|(k, _)| k.clone())
            {
                sessions.remove(&oldest);
            }
        }
        let token = random_token();
        let csrf = random_token();
        sessions.insert(
            digest(&token),
            Session {
                csrf: csrf.clone(),
                created: now,
                last_seen: now,
            },
        );
        Ok(Issued { token, csrf })
    }

    fn alive(session: &Session, now: Instant) -> bool {
        now.duration_since(session.created) < ABSOLUTE_EXPIRY
            && now.duration_since(session.last_seen) < IDLE_EXPIRY
    }

    /// Returns the session's CSRF token. `refresh` is false for background polling and SSE,
    /// so a tab left open cannot keep a session alive indefinitely without a real operator.
    pub fn validate(&self, token: &str, refresh: bool) -> Option<String> {
        let mut sessions = self.sessions.lock().ok()?;
        let now = Instant::now();
        let key = digest(token);
        let session = sessions.get_mut(&key)?;
        if !Self::alive(session, now) {
            sessions.remove(&key);
            return None;
        }
        if refresh {
            session.last_seen = now;
        }
        Some(session.csrf.clone())
    }

    pub fn logout(&self, token: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(&digest(token));
        }
    }

    fn check_throttle(&self) -> Result<(), &'static str> {
        let mut attempts = self.attempts.lock().map_err(|_| "auth state poisoned")?;
        if let Some(first) = attempts.first {
            if first.elapsed() > LOGIN_BACKOFF {
                *attempts = Attempts::default();
            } else if attempts.count >= MAX_ATTEMPTS {
                return Err("too many attempts; try again shortly");
            }
        }
        Ok(())
    }

    fn record_failure(&self) {
        if let Ok(mut attempts) = self.attempts.lock() {
            attempts.first.get_or_insert_with(Instant::now);
            attempts.count += 1;
        }
    }

    fn clear_failures(&self) {
        if let Ok(mut attempts) = self.attempts.lock() {
            *attempts = Attempts::default();
        }
    }
}

/// Length-independent comparison for the bootstrap secret.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(1));
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned() -> Auth {
        let auth = Auth::load(None, None, None);
        *auth.verifier.lock().unwrap() = Some(Auth::hash_password("correct horse").unwrap());
        auth
    }

    #[test]
    fn login_issues_a_session_with_its_own_csrf_token() {
        let auth = owned();
        let issued = auth.login("correct horse").expect("password is right");
        assert_eq!(auth.validate(&issued.token, true).as_deref(), Some(issued.csrf.as_str()));
        assert!(auth.login("wrong").is_err());
    }

    #[test]
    fn logout_revokes_only_that_session() {
        let auth = owned();
        let a = auth.login("correct horse").unwrap();
        let b = auth.login("correct horse").unwrap();
        auth.logout(&a.token);
        assert!(auth.validate(&a.token, true).is_none());
        assert!(auth.validate(&b.token, true).is_some());
    }

    #[test]
    fn an_unknown_token_is_never_valid() {
        assert!(owned().validate("not a real token", true).is_none());
    }

    #[test]
    fn bootstrap_claims_once_and_cannot_be_replayed() {
        let auth = Auth::load(None, None, None);
        *auth.bootstrap.lock().unwrap() = Some("one-time".into());
        assert!(!auth.has_owner());
        assert!(auth.bootstrap("wrong", "new password").is_err());
        assert!(auth.bootstrap("one-time", "new password").is_ok());
        assert!(auth.has_owner());
        // The secret is consumed, so a second claim cannot take over the installation.
        assert!(auth.bootstrap("one-time", "attacker").is_err());
        assert!(auth.login("new password").is_ok());
    }

    #[test]
    fn an_unprovisioned_server_cannot_be_claimed_anonymously() {
        let auth = Auth::load(None, None, None);
        assert!(auth.bootstrap("", "mine now").is_err());
        assert!(auth.login("anything").is_err());
    }

    #[test]
    fn repeated_failures_are_throttled_rather_than_locked_out() {
        let auth = owned();
        for _ in 0..MAX_ATTEMPTS {
            assert!(auth.login("wrong").is_err());
        }
        assert_eq!(auth.login("correct horse").unwrap_err(), "too many attempts; try again shortly");
    }

    /// Background polling and SSE must not keep a session alive on their own.
    #[test]
    fn only_refreshing_reads_move_the_idle_deadline() {
        let auth = owned();
        let issued = auth.login("correct horse").unwrap();
        let before = auth.sessions.lock().unwrap()[&digest(&issued.token)].last_seen;
        auth.validate(&issued.token, false);
        assert_eq!(auth.sessions.lock().unwrap()[&digest(&issued.token)].last_seen, before);
    }
}
