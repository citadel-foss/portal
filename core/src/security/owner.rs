//! The owner password: the credential that lets someone reach Portal at all, on the web server
//! and in the desktop app alike. One per data root, so on a machine running both, it is the same
//! password in both.
//!
//! Separate from every wallet password, always. Unlocking a wallet proves you can spend; this
//! proves you may use the app, and conflating them would put a wallet passphrase on the path of
//! every request.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;

/// Temporary, not a permanent lockout: someone who can reach the sign-in screen should not be
/// able to lock the owner out of their own wallet.
const LOGIN_BACKOFF: Duration = Duration::from_secs(30);
const MAX_ATTEMPTS: u32 = 10;

/// Where a claim stores the verifier under a data root.
pub fn owner_file(root: &Path) -> PathBuf {
    root.join("portal").join("auth").join("owner")
}

#[derive(Debug, Default)]
struct Attempts {
    count: u32,
    first: Option<Instant>,
}

fn read_verifier(path: Option<&Path>) -> Option<String> {
    path.and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[derive(Debug)]
pub struct OwnerCredential {
    /// Where a claimed owner's verifier is kept. Argon2id output only — never a password.
    store: Option<PathBuf>,
    verifier: Mutex<Option<String>>,
    attempts: Mutex<Attempts>,
}

impl OwnerCredential {
    /// Loads the verifier, from a provisioned file or from where a claim stored it.
    pub fn load(provisioned: Option<&Path>, store: Option<PathBuf>) -> Self {
        // A provisioned credential file wins: the deployment set it deliberately, and a stale
        // claim on disk must not shadow it.
        let verifier = read_verifier(provisioned).or_else(|| read_verifier(store.as_deref()));
        OwnerCredential {
            store,
            verifier: Mutex::new(verifier),
            attempts: Mutex::new(Attempts::default()),
        }
    }

    /// The verifier, re-read from disk while there is none in memory: another Portal process on
    /// the same data root may have claimed the install since this one started, and trusting the
    /// startup snapshot would refuse the real password and accept a second claim over it.
    fn verifier(&self) -> Result<MutexGuard<'_, Option<String>>, &'static str> {
        let mut verifier = self.verifier.lock().map_err(|_| "auth state poisoned")?;
        if verifier.is_none() {
            *verifier = read_verifier(self.store.as_deref());
        }
        Ok(verifier)
    }

    pub fn has_owner(&self) -> bool {
        self.verifier().is_ok_and(|v| v.is_some())
    }

    /// Sets the owner password on an install that has none, atomically: two concurrent claims
    /// cannot both succeed, and once an owner exists this is refused for good.
    pub fn claim(&self, new_password: &str) -> Result<(), &'static str> {
        // The real gate: claiming needs no session, so the UI's own check is not in the path
        // when a claim is scripted.
        crate::security::input::validate_password(new_password, "owner password")
            .map_err(|_| "owner password must be at least 8 characters")?;
        let mut verifier = self.verifier()?;
        if verifier.is_some() {
            return Err("this installation already has an owner");
        }
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(new_password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|_| "could not store credential")?;
        if let Some(path) = &self.store {
            if let Some(parent) = path.parent() {
                crate::security::fs::ensure_private_dir(parent)
                    .map_err(|_| "could not create the credential directory")?;
            }
            crate::security::fs::write_private(path, hash.as_bytes())
                .map_err(|_| "could not persist the credential")?;
        }
        *verifier = Some(hash);
        Ok(())
    }

    /// Checks a password. Throttled globally rather than per account: there is exactly one
    /// owner, so a per-account counter would be the same thing with a lockout attached.
    pub fn verify(&self, password: &str) -> Result<(), &'static str> {
        self.check_throttle()?;
        let stored = self.verifier()?.clone().ok_or("this installation has no owner yet")?;
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
        if let Ok(mut attempts) = self.attempts.lock() {
            *attempts = Attempts::default();
        }
        Ok(())
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trivial_owner_password_is_refused() {
        let owner = OwnerCredential::load(None, None);
        assert!(owner.claim("").is_err());
        assert!(owner.claim("short").is_err());
        assert!(!owner.has_owner(), "a refused claim must not install an owner");
        assert!(owner.claim("long-enough-password").is_ok());
    }

    /// Once claimed, nobody who reaches the sign-in screen later can replace the password.
    #[test]
    fn an_install_is_claimed_once() {
        let owner = OwnerCredential::load(None, None);
        assert!(owner.verify("anything").is_err(), "no owner, nothing to sign in to");
        assert!(owner.claim("new password").is_ok());
        assert!(owner.claim("attacker password").is_err());
        assert!(owner.verify("new password").is_ok());
        assert!(owner.verify("attacker password").is_err());
    }

    /// The desktop app and a web server on one data root: the one that did not take the claim
    /// must still see it.
    #[test]
    fn a_claim_made_by_another_process_is_honoured() {
        let dir = std::env::temp_dir().join(format!("portal-owner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let here = OwnerCredential::load(None, Some(owner_file(&dir)));
        let other = OwnerCredential::load(None, Some(owner_file(&dir)));
        assert!(other.claim("the real password").is_ok());
        assert!(here.has_owner());
        assert!(here.claim("a second claim").is_err());
        assert!(here.verify("the real password").is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn repeated_failures_are_throttled_rather_than_locked_out() {
        let owner = OwnerCredential::load(None, None);
        owner.claim("correct horse").unwrap();
        for _ in 0..MAX_ATTEMPTS {
            assert!(owner.verify("wrong").is_err());
        }
        assert_eq!(owner.verify("correct horse").unwrap_err(), "too many attempts; try again shortly");
    }
}
