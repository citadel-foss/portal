//! Portal's own Tor, never a host one: the app must not depend on someone else's config or
//! disturb it. One instance per process — a single Tor carries every maker's onion service
//! anyway — and never shared with another Portal process: a desktop app and a web server each
//! run their own, so quitting one cannot cut the other off.
//!
//! The Tor Project's own `tor` executable is compiled into the binary (see `build.rs`), written
//! out next to Portal's data on first use, and run as a child process.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Child;
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

/// The `tor` process behind `RUNTIME`, kept to tell whether it is still running and to reap it.
static PROCESS: Mutex<Option<Child>> = Mutex::new(None);

/// Starts Portal's Tor if it isn't running yet and returns the ports to bind to.
pub fn ensure_tor() -> Result<TorRuntime, String> {
    // The lock is released before the readiness wait: `runtime()` is a status read that the
    // UI makes while Tor is still bootstrapping, and holding it across the wait would block
    // every such caller for the best part of a minute.
    let runtime = {
        let mut slot = RUNTIME.lock().map_err(|e| e.to_string())?;
        // A Tor that has died since takes its ports with it. Handing them out again would leave
        // every caller waiting on a dead port, so it is replaced by a fresh one instead.
        if slot.is_some() && !process_running() {
            log::warn!("Portal's Tor exited on its own; starting a new one");
            *slot = None;
        }
        match slot.as_ref() {
            Some(runtime) => runtime.clone(),
            None => {
                let (socks_port, control_port) = free_port_pair().map_err(|e| e.to_string())?;
                let control_password = hex_upper(&random_bytes::<16>());
                let hashed = hashed_control_password(&control_password, &random_bytes::<8>());
                start_embedded_tor(&tor_dir()?, socks_port, control_port, &hashed)?;
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
    // Halting clears the slot, so the next attempt starts over on a fresh pair: the likeliest
    // cause is another program taking one of the ports between picking it and Tor binding it.
    shutdown();
    Err(format!(
        "Portal's Tor did not open its SOCKS and control ports (see {})",
        tor_dir()?.join("tor.log").display()
    ))
}

/// Halts Portal's Tor through its own control port. The last thing stopped on quit: the
/// makers and the taker both route through it, so it has to outlive them.
pub fn shutdown() {
    let Some(tor) = RUNTIME.lock().ok().and_then(|mut slot| slot.take()) else {
        return;
    };
    // SIGNAL HALT rather than killing the process, so Tor runs its own cleanup and writes back
    // the cached consensus the next launch starts from.
    if let Err(e) = signal_halt(&tor) {
        log::warn!("could not halt Portal's Tor cleanly: {e}");
    }
    let deadline = Instant::now() + HALT_TIMEOUT;
    while Instant::now() < deadline && process_running() {
        std::thread::sleep(Duration::from_millis(100));
    }
    if let Some(mut child) = PROCESS.lock().ok().and_then(|mut slot| slot.take()) {
        if matches!(child.try_wait(), Ok(None)) {
            log::warn!("Portal's Tor did not halt in time; killing it");
            let _ = child.kill();
        }
        let _ = child.wait();
    }
    // This process's directory goes with it once Tor has let go of it; the startup sweep only
    // reaches directories whose process is gone, and is left for the ones killed outright.
    if let Ok(dir) = tor_dir() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Makes Tor throw away its current attempt and bootstrap from the start.
///
/// Restarting the process would move Tor to new ports under everything already bound to the
/// old ones. `DisableNetwork` is settable at runtime instead, and dropping the network and
/// restoring it is what Tor Browser does to retry a stalled bootstrap. `SIGNAL RELOAD` is not
/// equivalent: it rereads the config and leaves a wedged bootstrap exactly where it was.
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

/// Shared by the taker and every maker in *this* process, but never between processes.
///
/// Suffixed with the process id so a desktop app and a web server on one data root each run
/// their own Tor without fighting over it. Tor refuses to start against a directory another
/// Tor already holds — it waits five seconds, logs "No, it's still there. Exiting.", and dies.
fn tor_dir() -> Result<PathBuf, String> {
    crate::storage::openswap_root()
        .map(|dir| dir.join(format!("tor-manager-{}", std::process::id())))
        .map_err(|e| e.message)
}

/// Where Tor keeps the consensus and microdescriptors, shared across launches.
///
/// This is deliberately *not* under `tor_dir()`. That one is per-process and swept when the
/// process is gone, which meant every launch handed Tor an empty directory and it re-fetched
/// the whole directory — around 40MB over a half-bootstrapped circuit, which is most of what
/// a cold start spends its time on. Only `DataDirectory` needs to be private: it holds the
/// lock, the keys and the state. The cache is exactly what Tor's `CacheDirectory` is for.
#[cfg(feature = "embedded-tor")]
fn tor_cache_dir() -> Result<PathBuf, String> {
    crate::storage::openswap_root()
        .map(|dir| dir.join("tor-cache"))
        .map_err(|e| e.message)
}

/// Removes `tor-manager` directories left by processes that are no longer running.
///
/// Each run gets its own, so without this they accumulate — one per launch, each holding a
/// Tor identity. Called at startup rather than shutdown: a process killed outright never gets
/// to tidy up after itself, and that is exactly when the directory is left behind.
pub fn sweep_stale_tor_dirs() {
    let Ok(root) = crate::storage::openswap_root() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    let mine = std::process::id();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|n| n.strip_prefix("tor-manager-"))
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == mine || process_is_alive(pid) {
            continue;
        }
        let _ = std::fs::remove_dir_all(entry.path());
    }
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    // Signal 0 checks for the process without delivering anything.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
fn process_is_alive(_pid: u32) -> bool {
    // Without a cheap liveness check, keeping the directory is the safe answer: a live Tor
    // losing its data directory is worse than a stale one lingering.
    true
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
/// as `16:<salt><indicator><digest>`. `tor --hash-password` produces this; computing it here
/// keeps the plaintext off a command line that every local process can read.
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
        // A Tor that exited (a port taken, a bad directory) will not open them later.
        if !process_running() {
            return false;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

/// Whether the `tor` process this one started is still running.
fn process_running() -> bool {
    PROCESS
        .lock()
        .ok()
        .and_then(|mut slot| {
            slot.as_mut()
                .map(|child| matches!(child.try_wait(), Ok(None)))
        })
        .unwrap_or(false)
}

#[cfg(feature = "embedded-tor")]
mod bundle {
    // Generated by build.rs: every file of the Tor expert bundle for this target.
    include!(concat!(env!("OUT_DIR"), "/tor_bundle.rs"));
}

/// Writes the embedded Tor bundle out and returns the directory it is in.
///
/// Every launch compares what is on disk with what is embedded and rewrites any file that
/// differs, so an upgrade, a half-written file or a tampered one is replaced before it runs.
/// Each file is written aside and renamed into place: another Portal process may be starting
/// the same `tor` at the same moment, and must never see half of one.
#[cfg(feature = "embedded-tor")]
fn install_bundle() -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;

    let dir = crate::storage::openswap_root()
        .map(|root| root.join("tor-bin"))
        .map_err(|e| e.message)?;
    crate::security::fs::ensure_private_dir(&dir).map_err(|e| e.message)?;
    for (name, bytes) in bundle::FILES {
        let path = dir.join(name);
        if std::fs::read(&path).is_ok_and(|on_disk| on_disk == *bytes) {
            continue;
        }
        let partial = dir.join(format!(".{name}.{}", std::process::id()));
        let written = std::fs::write(&partial, bytes)
            .and_then(|()| {
                std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o700))
            })
            .and_then(|()| std::fs::rename(&partial, &path));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&partial);
            return Err(format!(
                "could not install Portal's Tor at {}: {e}",
                path.display()
            ));
        }
    }
    Ok(dir)
}

#[cfg(feature = "embedded-tor")]
fn start_embedded_tor(
    tor_dir: &Path,
    socks_port: u16,
    control_port: u16,
    hashed_password: &str,
) -> Result<(), String> {
    use std::process::{Command, Stdio};

    let data_dir = tor_dir.join("data");
    let cache_dir = tor_cache_dir()?;
    crate::security::fs::ensure_private_dir(tor_dir).map_err(|e| e.message)?;
    crate::security::fs::ensure_private_dir(&data_dir).map_err(|e| e.message)?;
    crate::security::fs::ensure_private_dir(&cache_dir).map_err(|e| e.message)?;
    let bin_dir = install_bundle()?;

    // Neither torrc exists: `-f` and `--defaults-torrc` point at nothing so Tor never reads the
    // host's, and everything Portal needs is on the command line.
    let no_torrc = tor_dir.join("torrc").to_string_lossy().to_string();
    let mut command = Command::new(bin_dir.join("tor"));
    command
        .args(["--ignore-missing-torrc", "-f", &no_torrc])
        .args(["--defaults-torrc", &no_torrc])
        .arg("--DataDirectory")
        .arg(&data_dir)
        // Survives the per-process data directory, so a relaunch starts from a warm consensus
        // instead of downloading one.
        .arg("--CacheDirectory")
        .arg(&cache_dir)
        .args(["--SocksPort", &format!("127.0.0.1:{socks_port}")])
        .args(["--ControlPort", &format!("127.0.0.1:{control_port}")])
        .args(["--HashedControlPassword", hashed_password])
        .args(["--CookieAuthentication", "0"])
        .arg("--Log")
        .arg(format!("notice file {}", tor_dir.join("tor.log").display()))
        // Tor exits on its own once this process is gone, however it went — a Portal killed
        // outright must not leave a Tor running behind it.
        .args([
            "--__OwningControllerProcess",
            &std::process::id().to_string(),
        ])
        // Tor's startup notices would bury whatever the host printed for the user; the log
        // file above keeps them.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // The Linux bundle carries its own OpenSSL and libevent next to `tor`; the macOS one finds
    // its libevent through `@executable_path` already.
    #[cfg(target_os = "linux")]
    command.env("LD_LIBRARY_PATH", &bin_dir);
    // Its own process group, so a Ctrl-C in the terminal reaches Portal alone. Portal then
    // stops the makers and the taker before halting Tor, rather than losing Tor under them.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);

    let child = command
        .spawn()
        .map_err(|e| format!("could not start Portal's Tor: {e}"))?;
    log::info!("Portal's Tor started (pid {})", child.id());
    *PROCESS.lock().map_err(|e| e.to_string())? = Some(child);
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

    /// Runs the embedded Tor against the real network: bootstraps it, publishes an onion
    /// service through openswap's own `ADD_ONION` the way a maker does, and reaches it back
    /// through the SOCKS port the way a taker does. Needs the network, so only on request:
    ///
    /// `cargo test -p portal-core --features embedded-tor -- --ignored embedded_tor_end_to_end`
    #[cfg(feature = "embedded-tor")]
    #[test]
    #[ignore = "needs the Tor network"]
    fn embedded_tor_end_to_end() {
        // A throwaway home, so the test never touches the real ~/.openswap.
        let home = std::env::temp_dir().join(format!("portal-tor-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_var("HOME", &home);

        let tor = ensure_tor().expect("embedded Tor starts");
        let deadline = Instant::now() + Duration::from_secs(180);
        while bootstrap_progress(&tor) != Some(100) {
            assert!(Instant::now() < deadline, "Tor did not bootstrap in time");
            std::thread::sleep(Duration::from_secs(1));
        }

        let service = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local_port = service.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut stream in service.incoming().flatten() {
                let _ = stream.write_all(b"portal");
            }
        });
        let hostname = openswap::utill::get_ephemeral_address(
            tor.control_port,
            local_port,
            &tor.control_password,
            "NEW:ED25519-V3",
            None,
        )
        .expect("ADD_ONION succeeds");

        // A new onion service takes a while to publish its descriptor; keep trying until then.
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            if let Ok(reply) = fetch_through_socks(tor.socks_port, &hostname) {
                assert_eq!(reply, b"portal");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "onion service never became reachable"
            );
            std::thread::sleep(Duration::from_secs(5));
        }

        shutdown();
        assert!(runtime().is_none());
        assert!(!process_running());
        let _ = std::fs::remove_dir_all(home);
    }

    #[cfg(feature = "embedded-tor")]
    fn bootstrap_progress(tor: &TorRuntime) -> Option<u8> {
        let mut stream = TcpStream::connect(("127.0.0.1", tor.control_port)).ok()?;
        let mut reader = BufReader::new(stream.try_clone().ok()?);
        let command = format!(
            "AUTHENTICATE \"{}\"\r\nGETINFO status/bootstrap-phase\r\n",
            tor.control_password
        );
        stream.write_all(command.as_bytes()).ok()?;
        let mut line = String::new();
        for _ in 0..2 {
            line.clear();
            reader.read_line(&mut line).ok()?;
        }
        line.split("PROGRESS=")
            .nth(1)?
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    }

    /// One SOCKS5 CONNECT by hostname to the openswap port, returning what the service sent.
    #[cfg(feature = "embedded-tor")]
    fn fetch_through_socks(socks_port: u16, hostname: &str) -> std::io::Result<Vec<u8>> {
        use openswap::protocol::common_messages::OPENSWAP_PORT;

        let mut stream = TcpStream::connect(("127.0.0.1", socks_port))?;
        stream.set_read_timeout(Some(Duration::from_secs(120)))?;
        stream.write_all(&[0x05, 0x01, 0x00])?;
        let mut reply = [0u8; 2];
        stream.read_exact(&mut reply)?;
        let mut request = vec![0x05, 0x01, 0x00, 0x03, hostname.len() as u8];
        request.extend_from_slice(hostname.as_bytes());
        request.extend_from_slice(&OPENSWAP_PORT.to_be_bytes());
        stream.write_all(&request)?;
        let mut reply = [0u8; 10];
        stream.read_exact(&mut reply)?;
        if reply[1] != 0x00 {
            return Err(std::io::Error::other(format!("SOCKS reply {}", reply[1])));
        }
        let mut body = Vec::new();
        stream.read_to_end(&mut body)?;
        Ok(body)
    }
}
