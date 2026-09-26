//! Setup & connectivity commands: wizard prechecks and version info.
//! All blocking I/O runs via `spawn_blocking` — never on the async runtime.
//! Wallet lifecycle (init/restore/backup) lives in `commands::taker_wallet`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::error::{AppError, ErrorCode};
use crate::types::TorStatus;

/// Starts Portal's own Tor if it isn't up yet, then mirrors openswap's control-port
/// handshake against it. Bootstrap < 100% is informational only, not a failure.
pub async fn check_tor() -> Result<TorStatus, AppError> {
    tokio::task::spawn_blocking(|| match crate::tor::ensure_tor() {
        Ok(runtime) => run_tor_handshake(&runtime),
        Err(error) => TorStatus {
            reachable: false,
            socks_reachable: false,
            authenticated: false,
            bootstrap_progress: None,
            bootstrap_summary: None,
            error: Some(error),
            socks_port: None,
            control_port: None,
        },
    })
    .await
    .map_err(AppError::internal)
}

/// Restarts the bootstrap of the Tor already running, for a user who has watched it sit at the
/// same percentage and wants it to start over rather than wait longer.
pub async fn restart_tor_bootstrap() -> Result<(), AppError> {
    tokio::task::spawn_blocking(crate::tor::restart_bootstrap)
        .await
        .map_err(AppError::internal)?
        .map_err(|e| AppError::new(ErrorCode::TorUnreachable, e))
}

fn run_tor_handshake(tor: &crate::tor::TorRuntime) -> TorStatus {
    let (socks_port, control_port) = (tor.socks_port, tor.control_port);
    let unreachable = |socks_reachable: bool, err: String| TorStatus {
        reachable: false,
        socks_reachable,
        authenticated: false,
        bootstrap_progress: None,
        bootstrap_summary: None,
        error: Some(err),
        socks_port: Some(socks_port),
        control_port: Some(control_port),
    };

    if !crate::tor::socks5_responds(socks_port) {
        return unreachable(
            false,
            "configured SOCKS port did not complete a SOCKS5 greeting".into(),
        );
    }

    let addr = match format!("127.0.0.1:{control_port}").to_socket_addrs() {
        Ok(mut addrs) => match addrs.next() {
            Some(a) => a,
            None => return unreachable(true, "could not resolve control port address".into()),
        },
        Err(e) => return unreachable(true, e.to_string()),
    };

    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
        Ok(s) => s,
        Err(e) => return unreachable(true, e.to_string()),
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    let mut reader = match stream.try_clone() {
        Ok(s) => BufReader::new(s),
        Err(e) => return unreachable(true, e.to_string()),
    };

    if stream.write_all(b"PROTOCOLINFO 1\r\n").is_err() {
        return unreachable(true, "failed to send PROTOCOLINFO".into());
    }
    let mut protocol_lines = Vec::new();
    for _ in 0..32 {
        let mut line = String::new();
        let read = (&mut reader).take(8193).read_line(&mut line);
        if !matches!(read, Ok(1..=8192)) || !line.ends_with('\n') {
            return unreachable(true, "invalid Tor PROTOCOLINFO response".into());
        }
        let done = line.starts_with("250 OK");
        protocol_lines.push(line);
        if done {
            break;
        }
    }
    if protocol_lines.is_empty()
        || !protocol_lines[0].starts_with("250-PROTOCOLINFO")
        || !protocol_lines
            .last()
            .is_some_and(|line| line.starts_with("250 OK"))
    {
        return unreachable(true, "control port did not identify itself as Tor".into());
    }

    // Same quoted form the crate uses, so a failure here means the maker's ADD_ONION
    // would fail the same way rather than passing this check and breaking later.
    let command = format!("AUTHENTICATE \"{}\"\r\n", tor.control_password);
    if stream.write_all(command.as_bytes()).is_err() {
        return unreachable(true, "failed to send AUTHENTICATE".into());
    }
    let mut resp = String::new();
    if reader.read_line(&mut resp).is_err() || !resp.starts_with("250") {
        return TorStatus {
            reachable: true,
            socks_reachable: true,
            authenticated: false,
            bootstrap_progress: None,
            bootstrap_summary: None,
            error: Some("Tor control-port authentication failed".into()),
            socks_port: Some(socks_port),
            control_port: Some(control_port),
        };
    }

    if stream
        .write_all(b"GETINFO status/bootstrap-phase\r\n")
        .is_err()
    {
        return TorStatus {
            reachable: true,
            socks_reachable: true,
            authenticated: true,
            bootstrap_progress: None,
            bootstrap_summary: None,
            error: None,
            socks_port: Some(socks_port),
            control_port: Some(control_port),
        };
    }
    resp.clear();
    let _ = reader.read_line(&mut resp);
    // Anything outside 0-100 is reported as unknown rather than clamped: the gate unlocks on
    // exactly 100, so clamping a bogus 101 would call a half-bootstrapped Tor ready.
    let bootstrap_progress = resp
        .split("PROGRESS=")
        .nth(1)
        .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|s| s.parse::<u8>().ok())
        .filter(|progress| *progress <= 100);

    TorStatus {
        reachable: true,
        socks_reachable: true,
        authenticated: true,
        bootstrap_progress,
        bootstrap_summary: quoted_field(&resp, "SUMMARY"),
        error: None,
        socks_port: Some(socks_port),
        control_port: Some(control_port),
    }
}

/// Pulls `KEY="..."` out of a Tor control-port reply. Tor emits these as plain quoted strings
/// with no escaping in the bootstrap phase line, so the first closing quote ends the value.
///
/// Only `SUMMARY` is read. Tor also emits `WARNING`, but its value is frequently a bare reason
/// code — a relay closing cleanly mid-handshake reports `WARNING="DONE"` — and Tor retries
/// past those on its own. `tor.log` keeps them for diagnosis; the gate does not show them.
fn quoted_field(line: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    Some(rest[..rest.find('"')?].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `GETINFO status/bootstrap-phase` reply, both while progressing and while Tor is
    /// reporting trouble — the summary has to survive the extra fields the warning form adds.
    #[test]
    fn reads_the_summary_from_the_bootstrap_phase() {
        let progressing = "250-status/bootstrap-phase=NOTICE BOOTSTRAP PROGRESS=50 \
             TAG=loading_descriptors SUMMARY=\"Loading relay descriptors\"\r\n";
        assert_eq!(
            quoted_field(progressing, "SUMMARY").as_deref(),
            Some("Loading relay descriptors")
        );

        let struggling = "250-status/bootstrap-phase=WARN BOOTSTRAP PROGRESS=10 TAG=conn_done \
             SUMMARY=\"Connected to a relay\" WARNING=\"Connection timed out\" REASON=TIMEOUT \
             COUNT=3 RECOMMENDATION=warn\r\n";
        assert_eq!(
            quoted_field(struggling, "SUMMARY").as_deref(),
            Some("Connected to a relay")
        );
    }
}
