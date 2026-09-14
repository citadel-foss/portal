//! Portal's own Tor, never a host one: the app must not depend on someone else's config or
//! disturb it. One instance serves the whole process — `tor_main` cannot run twice, and a
//! single Tor carries every maker's onion service anyway.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use std::io::{BufRead, BufReader, Read, Write};

use openswap::bitcoin::secp256k1::rand::RngCore;

const READINESS_TIMEOUT: Duration = Duration::from_secs(60);
/// `SIGNAL HALT` exits immediately by design, so this only bounds a Tor that has stopped
/// answering its own control port — not the graceful maker and taker teardown before it.
const HALT_TIMEOUT: Duration = Duration::from_secs(10);

/// Ports and control credential of the Tor this process started.
#[derive(Debug, Clone)]
pub struct TorRuntime {
    pub socks_port: u16,
    pub control_port: u16,
    /// Plaintext control-port password. The openswap crate authenticates with
    /// `AUTHENTICATE "<password>"`, so cookie auth would lock the maker out of `ADD_ONION`.
    pub control_password: String,
}

/// Populated the moment Tor is launched, before it is known to be ready, so a retry waits on
/// the instance already starting instead of spawning a second one on a second pair of ports.
static RUNTIME: Mutex<Option<TorRuntime>> = Mutex::new(None);

/// Separate from `RUNTIME` because it is never cleared: `tor_main` cannot run twice in one
/// process, so once a launch has been attempted the only way to another one is a new process.
/// Without this a failed readiness wait — which clears `RUNTIME` — would let the next retry
/// call `tor_main` a second time and take the whole app down with it.
static STARTED: AtomicBool = AtomicBool::new(false);

/// Starts Portal's Tor if it isn't running yet and returns the ports to bind to.
pub fn ensure_tor() -> Result<TorRuntime, String> {
    // The lock is released before the readiness wait: `runtime()` is a status read that the
    // UI makes while Tor is still bootstrapping, and holding it across the wait would block
    // every such caller for the best part of a minute.
    let runtime = {
        let mut slot = RUNTIME.lock().map_err(|e| e.to_string())?;
        match slot.as_ref() {
            Some(runtime) => runtime.clone(),
            None => {
                // Load-then-store rather than swap, and stored only once `tor_main` has actually
                // been spawned: picking ports or creating the Tor directory can fail without
                // ever reaching it, and latching the flag there would answer every later retry
                // with the permanent error below over a transient fault. Safe as two steps
                // because the `RUNTIME` lock is held across both.
                if STARTED.load(Ordering::SeqCst) {
                    return Err(
                        "Portal's Tor already ran once this session and cannot be started again \
                         in place. Quit and reopen Portal to try a fresh one."
                            .to_string(),
                    );
                }
                let (socks_port, control_port) = free_port_pair().map_err(|e| e.to_string())?;
                let control_password = hex_upper(&random_bytes::<16>());
                let hashed = hashed_control_password(&control_password, &random_bytes::<8>());
                start_embedded_tor(&tor_dir()?, socks_port, control_port, &hashed)?;
                STARTED.store(true, Ordering::SeqCst);
                let runtime = TorRuntime {
                    socks_port,
                    control_port,
                    control_password,
                };
                *slot = Some(runtime.clone());
                runtime
            }
        }
    };

    if wait_until_ready(runtime.socks_port, runtime.control_port) {
        return Ok(runtime);
    }
    // Otherwise the cached runtime would be handed to every later call, which would wait on
    // the same dead ports forever — the gate and every maker would stay down until restart.
    // Halting clears the slot, so the next attempt starts over on a fresh pair.
    shutdown();
    Err("Portal's Tor did not open its SOCKS and control ports".to_string())
}

/// Halts Portal's Tor through its own control port. The last thing stopped on quit: the
/// makers and the taker both route through it, so it has to outlive them.
pub fn shutdown() {
    let Some(tor) = runtime() else {
        return;
    };
    // SIGNAL HALT rather than letting the thread die with the process, so Tor runs its own
    // cleanup and writes back the cached consensus the next launch starts from.
    if let Err(e) = signal_halt(&tor) {
        log::warn!("could not halt Portal's Tor cleanly: {e}");
    }
    let deadline = Instant::now() + HALT_TIMEOUT;
    while Instant::now() < deadline && control_responds_as_tor(tor.control_port) {
        std::thread::sleep(Duration::from_millis(100));
    }
    if let Ok(mut slot) = RUNTIME.lock() {
        *slot = None;
    }
}

/// Makes Tor throw away its current attempt and bootstrap from the start.
///
/// `tor_main` only runs once per process, so there is no restarting Tor itself — but
/// `DisableNetwork` is settable at runtime, and dropping the network and restoring it is what
/// Tor Browser does to retry a stalled bootstrap. `SIGNAL RELOAD` is not equivalent: it rereads
/// the config and leaves a wedged bootstrap exactly where it was.
pub fn restart_bootstrap() -> Result<(), String> {
    let tor = runtime().ok_or_else(|| "Portal's Tor has not started yet".to_string())?;
    let addr = SocketAddr::from(([127, 0, 0, 1], tor.control_port));
    let io = |e: std::io::Error| e.to_string();
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(io)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(io)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(io)?;
    let mut reader = BufReader::new(stream.try_clone().map_err(io)?);

    for command in [
        format!("AUTHENTICATE \"{}\"", tor.control_password),
        "SETCONF DisableNetwork=1".to_string(),
        "SETCONF DisableNetwork=0".to_string(),
    ] {
        stream
            .write_all(format!("{command}\r\n").as_bytes())
            .map_err(io)?;
        let mut line = String::new();
        reader.read_line(&mut line).map_err(io)?;
        if !line.starts_with("250") {
            // The reply, never the command — the first one carries the control password.
            return Err(format!("Tor refused the restart: {}", line.trim()));
        }
    }
    Ok(())
}

fn signal_halt(tor: &TorRuntime) -> std::io::Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], tor.control_port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);

    stream.write_all(format!("AUTHENTICATE \"{}\"\r\n", tor.control_password).as_bytes())?;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if !line.starts_with("250") {
        return Err(std::io::Error::other("control-port authentication failed"));
    }

    stream.write_all(b"SIGNAL HALT\r\n")?;
    line.clear();
    reader.read_line(&mut line)?;
    Ok(())
}

/// The ports Tor is on, once started. `None` before the first `ensure_tor`.
pub fn runtime() -> Option<TorRuntime> {
    RUNTIME.lock().ok()?.clone()
}

/// Shared by the taker and every maker, but kept under the taker data dir so the layout
/// stays inside the directories the openswap crate already owns.
fn tor_dir() -> Result<PathBuf, String> {
    openswap::utill::get_taker_dir()
        .map(|dir| dir.join("tor-manager"))
        .map_err(|e| e.to_string())
}

/// Picks a free loopback port pair. Holding both listeners until each port is known stops the
/// OS handing out the same one twice, but they are released for Tor to bind — nothing reserves
/// them in between, so a failed bind is recovered by retrying with a fresh pair.
fn free_port_pair() -> std::io::Result<(u16, u16)> {
    let socks = TcpListener::bind(("127.0.0.1", 0))?;
    let control = TcpListener::bind(("127.0.0.1", 0))?;
    Ok((socks.local_addr()?.port(), control.local_addr()?.port()))
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buffer = [0u8; N];
    openswap::bitcoin::secp256k1::rand::thread_rng().fill_bytes(&mut buffer);
    buffer
}

fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

/// Tor's `HashedControlPassword` value: RFC2440 iterated-and-salted S2K over SHA1, rendered
/// as `16:<salt><indicator><digest>`. `tor --hash-password` produces this; libtor exposes no
/// equivalent, so it has to be computed here.
fn hashed_control_password(password: &str, salt: &[u8; 8]) -> String {
    use openswap::bitcoin::hashes::{sha1, Hash, HashEngine};

    // Tor's own default indicator; expands to (16 + (c & 15)) << ((c >> 4) + 6) = 65536 bytes.
    const INDICATOR: u8 = 0x60;
    let count = (16 + usize::from(INDICATOR & 0x0f)) << ((INDICATOR >> 4) + 6);

    let mut secret = salt.to_vec();
    secret.extend_from_slice(password.as_bytes());
    let mut engine = sha1::Hash::engine();
    let mut remaining = count;
    while remaining > 0 {
        let chunk = remaining.min(secret.len());
        engine.input(&secret[..chunk]);
        remaining -= chunk;
    }
    let digest = sha1::Hash::from_engine(engine);

    format!(
        "16:{}{INDICATOR:02X}{}",
        hex_upper(salt),
        hex_upper(digest.as_byte_array())
    )
}

/// Performs a bounded SOCKS5 greeting against a loopback port.
pub fn socks5_responds(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(700)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    if stream.write_all(&[0x05, 0x01, 0x00]).is_err() {
        return false;
    }
    let mut response = [0u8; 2];
    stream.read_exact(&mut response).is_ok() && response[0] == 0x05 && response[1] != 0xff
}

fn control_responds_as_tor(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(700)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    if stream.write_all(b"PROTOCOLINFO 1\r\n").is_err() {
        return false;
    }
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).is_ok() && line.starts_with("250-PROTOCOLINFO")
}

fn wait_until_ready(socks_port: u16, control_port: u16) -> bool {
    let start = Instant::now();
    while start.elapsed() < READINESS_TIMEOUT {
        if socks5_responds(socks_port) && control_responds_as_tor(control_port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

#[cfg(feature = "embedded-tor")]
fn start_embedded_tor(
    tor_dir: &Path,
    socks_port: u16,
    control_port: u16,
    hashed_password: &str,
) -> Result<(), String> {
    use libtor::{Tor, TorFlag};

    let data_dir = tor_dir.join("data");
    crate::security::fs::ensure_private_dir(tor_dir).map_err(|e| e.message)?;
    crate::security::fs::ensure_private_dir(&data_dir).map_err(|e| e.message)?;

    let handle = Tor::new()
        .flag(TorFlag::DataDirectory(
            data_dir.to_string_lossy().to_string(),
        ))
        .flag(TorFlag::SocksPort(socks_port))
        .flag(TorFlag::ControlPort(control_port))
        .flag(TorFlag::HashedControlPassword(hashed_password.to_string()))
        .flag(TorFlag::LogTo(
            libtor::LogLevel::Notice,
            libtor::LogDestination::File(tor_dir.join("tor.log").to_string_lossy().to_string()),
        ))
        .start_background();

    std::thread::spawn(move || match handle.join() {
        Ok(Ok(code)) => log::info!("embedded tor exited with code {code}"),
        Ok(Err(e)) => log::warn!("embedded tor error: {e:?}"),
        Err(_) => log::warn!("embedded tor thread panicked"),
    });
    Ok(())
}

#[cfg(not(feature = "embedded-tor"))]
fn start_embedded_tor(
    _tor_dir: &Path,
    _socks_port: u16,
    _control_port: u16,
    _hashed_password: &str,
) -> Result<(), String> {
    Err("embedded Tor support was not compiled in (build with --features embedded-tor)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vector produced by `tor --hash-password portal`, with its random salt pinned.
    #[test]
    fn hashed_password_matches_tor_s2k() {
        let salt = [0x37, 0x98, 0x19, 0x45, 0xF4, 0xFF, 0x1F, 0x4E];
        assert_eq!(
            hashed_control_password("portal", &salt),
            "16:37981945F4FF1F4E6076B831F1411EAF1F6A2255EE8B4E1CBE55EE4E2B"
        );
    }

    #[test]
    fn port_pair_is_distinct() {
        let (socks, control) = free_port_pair().unwrap();
        assert_ne!(socks, control);
    }
}
