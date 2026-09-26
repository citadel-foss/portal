//! Browser sessions and CSRF, earned with the owner password (`portal_core::security::owner`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portal_core::security::owner::OwnerCredential;
use rand::rngs::OsRng;
use rand::RngCore;

/// No idle timeout: an open tab stays signed in, since a swap can run for hours with nobody at
/// the screen. Only a tab that has gone away is timed out.
const ABSOLUTE_EXPIRY: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const DETACHED_EXPIRY: Duration = Duration::from_secs(2 * 60 * 60);
const MAX_SESSIONS: usize = 8;

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
    /// Event streams this session has open. A tab holds one for as long as it is open, so
    /// this is how the server tells an open tab from one that was closed.
    open_streams: u32,
    /// When `open_streams` last fell to zero, or `created` if no stream has opened yet.
    detached_since: Instant,
}

type SessionEndHook = Box<dyn Fn(&str) + Send + Sync>;

/// Sessions live in memory only, so a process restart revokes every one of them. That is the
/// intended behavior while wallet unlock is also memory-only: a restarted server cannot act
/// on a wallet anyway.
pub struct Auth {
    owner: OwnerCredential,
    sessions: Mutex<HashMap<String, Session>>,
    /// Told the id of every session that ends, however it ends: logout, expiry, eviction. The
    /// runtime detaches that session from its wallet, and a wallet nobody is on is closed.
    on_end: Option<SessionEndHook>,
}

#[derive(Debug)]
pub struct Issued {
    pub token: String,
    pub csrf: String,
}

/// A live session and the CSRF token bound to it.
#[derive(Debug, Clone)]
pub struct SessionRef {
    pub csrf: String,
    /// Stable for the session's life and safe to hand around: it is the lookup digest, never
    /// the cookie.
    pub id: String,
}

impl Auth {
    pub fn load(verifier_file: Option<&Path>, store: Option<std::path::PathBuf>) -> Self {
        Auth {
            owner: OwnerCredential::load(verifier_file, store),
            sessions: Mutex::new(HashMap::new()),
            on_end: None,
        }
    }

    pub fn on_session_end(mut self, hook: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.on_end = Some(Box::new(hook));
        self
    }

    /// Called with the sessions lock released: the hook takes runtime locks of its own.
    fn ended(&self, ids: Vec<String>) {
        if let Some(hook) = &self.on_end {
            for id in ids {
                hook(&id);
            }
        }
    }

    fn sweep(sessions: &mut HashMap<String, Session>, now: Instant) -> Vec<String> {
        let dead: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| !Self::alive(s, now))
            .map(|(k, _)| k.clone())
            .collect();
        for key in &dead {
            sessions.remove(key);
        }
        dead
    }

    /// Ends every session past its expiry. Run on a timer: a tab that closed and never came
    /// back makes no request that would notice.
    pub fn reap(&self) {
        let dead = match self.sessions.lock() {
            Ok(mut sessions) => Self::sweep(&mut sessions, Instant::now()),
            Err(_) => return,
        };
        self.ended(dead);
    }

    pub fn has_owner(&self) -> bool {
        self.owner.has_owner()
    }

    pub fn claim(&self, new_password: &str) -> Result<(), &'static str> {
        self.owner.claim(new_password)
    }

    pub fn login(&self, password: &str) -> Result<Issued, &'static str> {
        self.owner.verify(password)?;
        self.issue()
    }

    fn issue(&self) -> Result<Issued, &'static str> {
        let mut sessions = self.sessions.lock().map_err(|_| "auth state poisoned")?;
        let now = Instant::now();
        let mut ended = Self::sweep(&mut sessions, now);
        if sessions.len() >= MAX_SESSIONS {
            // A closed tab goes before an open one, then oldest first, so a forgotten tab
            // cannot lock the owner out of a new one.
            if let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, s)| (s.open_streams > 0, s.detached_since))
                .map(|(k, _)| k.clone())
            {
                sessions.remove(&oldest);
                ended.push(oldest);
            }
        }
        let token = random_token();
        let csrf = random_token();
        sessions.insert(
            digest(&token),
            Session {
                csrf: csrf.clone(),
                created: now,
                open_streams: 0,
                detached_since: now,
            },
        );
        drop(sessions);
        self.ended(ended);
        Ok(Issued { token, csrf })
    }

    fn alive(session: &Session, now: Instant) -> bool {
        now.duration_since(session.created) < ABSOLUTE_EXPIRY
            && (session.open_streams > 0
                || now.duration_since(session.detached_since) < DETACHED_EXPIRY)
    }

    /// Validates a token and returns the session it names.
    pub fn validate(&self, token: &str) -> Option<SessionRef> {
        let key = digest(token);
        let mut sessions = self.sessions.lock().ok()?;
        let session = sessions.get(&key)?;
        if Self::alive(session, Instant::now()) {
            return Some(SessionRef {
                csrf: session.csrf.clone(),
                id: key,
            });
        }
        sessions.remove(&key);
        drop(sessions);
        self.ended(vec![key]);
        None
    }

    /// Counts an event stream against the session for as long as the returned guard lives.
    /// `None` if the session is not live.
    pub fn open_stream(self: &Arc<Self>, token: &str) -> Option<StreamGuard> {
        let key = digest(token);
        let mut sessions = self.sessions.lock().ok()?;
        let session = sessions.get_mut(&key)?;
        if Self::alive(session, Instant::now()) {
            session.open_streams += 1;
            return Some(StreamGuard { auth: self.clone(), key });
        }
        sessions.remove(&key);
        drop(sessions);
        self.ended(vec![key]);
        None
    }

    pub fn logout(&self, token: &str) {
        let key = digest(token);
        let removed = self
            .sessions
            .lock()
            .is_ok_and(|mut sessions| sessions.remove(&key).is_some());
        if removed {
            self.ended(vec![key]);
        }
    }
}

/// Held by an open event stream. Dropped when the stream ends, which for a closed tab is the
/// first keepalive the server fails to write.
pub struct StreamGuard {
    auth: Arc<Auth>,
    key: String,
}

impl StreamGuard {
    /// The session this stream belongs to, which is what decides the wallet events it gets.
    pub fn session_id(&self) -> &str {
        &self.key
    }
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        let Ok(mut sessions) = self.auth.sessions.lock() else { return };
        if let Some(session) = sessions.get_mut(&self.key) {
            session.open_streams = session.open_streams.saturating_sub(1);
            if session.open_streams == 0 {
                session.detached_since = Instant::now();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned() -> Auth {
        let auth = Auth::load(None, None);
        auth.claim("correct horse").unwrap();
        auth
    }

    #[test]
    fn login_issues_a_session_with_its_own_csrf_token() {
        let auth = owned();
        let issued = auth.login("correct horse").expect("password is right");
        assert_eq!(auth.validate(&issued.token).map(|s| s.csrf), Some(issued.csrf.clone()));
        assert!(auth.login("wrong").is_err());
    }

    #[test]
    fn logout_revokes_only_that_session() {
        let auth = owned();
        let a = auth.login("correct horse").unwrap();
        let b = auth.login("correct horse").unwrap();
        auth.logout(&a.token);
        assert!(auth.validate(&a.token).is_none());
        assert!(auth.validate(&b.token).is_some());
    }

    #[test]
    fn an_unknown_token_is_never_valid() {
        assert!(owned().validate("not a real token").is_none());
    }

    /// Moves a session's clock back instead of waiting.
    fn age(auth: &Auth, token: &str, created: Duration, detached: Duration) {
        let now = Instant::now();
        let mut sessions = auth.sessions.lock().unwrap();
        let session = sessions.get_mut(&digest(token)).unwrap();
        session.created = now - created;
        session.detached_since = now - detached;
    }

    const MINUTE: Duration = Duration::from_secs(60);
    const DAY: Duration = Duration::from_secs(24 * 60 * 60);

    #[test]
    fn an_open_tab_is_never_idled_out() {
        let auth = Arc::new(owned());
        let issued = auth.login("correct horse").unwrap();
        let _stream = auth.open_stream(&issued.token).unwrap();
        age(&auth, &issued.token, 6 * DAY, 6 * DAY);
        assert!(auth.validate(&issued.token).is_some());
    }

    #[test]
    fn an_open_tab_still_ends_at_the_absolute_limit() {
        let auth = Arc::new(owned());
        let issued = auth.login("correct horse").unwrap();
        let _stream = auth.open_stream(&issued.token).unwrap();
        age(&auth, &issued.token, 7 * DAY, 7 * DAY);
        assert!(auth.validate(&issued.token).is_none());
    }

    #[test]
    fn a_closed_tab_expires_after_the_detached_window() {
        let auth = Arc::new(owned());
        let issued = auth.login("correct horse").unwrap();
        drop(auth.open_stream(&issued.token).unwrap());
        age(&auth, &issued.token, 119 * MINUTE, 119 * MINUTE);
        assert!(auth.validate(&issued.token).is_some());
        age(&auth, &issued.token, 120 * MINUTE, 120 * MINUTE);
        assert!(auth.validate(&issued.token).is_none());
    }

    /// The window runs from the last stream closing, not from login, and only once every
    /// stream the session has open is gone.
    #[test]
    fn the_detached_window_starts_when_the_last_stream_closes() {
        let auth = Arc::new(owned());
        let issued = auth.login("correct horse").unwrap();
        let first = auth.open_stream(&issued.token).unwrap();
        let second = auth.open_stream(&issued.token).unwrap();
        age(&auth, &issued.token, DAY, DAY);
        drop(first);
        assert!(auth.validate(&issued.token).is_some(), "one stream is still open");
        drop(second);
        assert!(auth.validate(&issued.token).is_some(), "the window restarts on close");
    }

    /// Every way a session ends has to reach the runtime, or its wallet stays open forever.
    #[test]
    fn every_ending_is_reported_once() {
        let ended = Arc::new(Mutex::new(Vec::new()));
        let seen = ended.clone();
        let auth = Arc::new(owned().on_session_end(move |id| seen.lock().unwrap().push(id.to_string())));

        let logged_out = auth.login("correct horse").unwrap();
        auth.logout(&logged_out.token);
        auth.logout(&logged_out.token);

        let expired = auth.login("correct horse").unwrap();
        age(&auth, &expired.token, DAY, DAY);
        auth.reap();
        auth.reap();

        assert_eq!(
            *ended.lock().unwrap(),
            vec![digest(&logged_out.token), digest(&expired.token)]
        );
    }

    #[test]
    fn a_full_table_evicts_a_closed_tab_before_an_open_one() {
        let auth = Arc::new(owned());
        let open = auth.login("correct horse").unwrap();
        let _stream = auth.open_stream(&open.token).unwrap();
        age(&auth, &open.token, DAY, DAY);
        let closed = auth.login("correct horse").unwrap();
        for _ in 2..MAX_SESSIONS {
            auth.login("correct horse").unwrap();
        }
        age(&auth, &closed.token, MINUTE, MINUTE);
        auth.login("correct horse").unwrap();
        assert!(auth.validate(&open.token).is_some());
        assert!(auth.validate(&closed.token).is_none());
    }
}
