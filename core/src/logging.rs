//! One process logger with a dynamic per-maker router. Maker server threads
//! are named `maker-{id}` and most background records include `[port]`; both
//! signals are used so concurrently running makers keep separate log files.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use log4rs::append::console::ConsoleAppender;
use log4rs::append::rolling_file::policy::compound::roll::fixed_window::FixedWindowRoller;
use log4rs::append::rolling_file::policy::compound::trigger::size::SizeTrigger;
use log4rs::append::rolling_file::policy::compound::CompoundPolicy;
use log4rs::append::rolling_file::RollingFileAppender;
use log4rs::config::{Appender, Config, Logger, Root};
use log4rs::Handle;

use crate::events::{AppEvent, EventSink, InitPhase};

static HANDLE: OnceLock<Handle> = OnceLock::new();
static TAKER_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
static MAKERS: OnceLock<Mutex<HashMap<String, MakerLogTarget>>> = OnceLock::new();
static MAKER_WRITE: Mutex<()> = Mutex::new(());

const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
const MAX_TAIL_BYTES: usize = 1024 * 1024;
const MAX_TAIL_LINES: usize = 1000;
const LOG_GENERATIONS: u32 = 3;

#[derive(Debug, Clone)]
struct MakerLogTarget {
    path: PathBuf,
    network_port: u16,
}

fn makers() -> &'static Mutex<HashMap<String, MakerLogTarget>> {
    MAKERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn file_appender(dir: &Path) -> Option<RollingFileAppender> {
    let active = dir.join("debug.log");
    let archive = dir.join("debug.log.{}");
    let roller = FixedWindowRoller::builder()
        .build(archive.to_string_lossy().as_ref(), LOG_GENERATIONS)
        .ok()?;
    let policy = CompoundPolicy::new(Box::new(SizeTrigger::new(MAX_LOG_BYTES)), Box::new(roller));
    RollingFileAppender::builder()
        .build(active, Box::new(policy))
        .ok()
}

fn redact_token(line: &mut String, marker: &str) {
    while let Some(start) = line.find(marker) {
        let value_start = start + marker.len();
        let end = line[value_start..]
            .find(char::is_whitespace)
            .map(|offset| value_start + offset)
            .unwrap_or(line.len());
        line.replace_range(start..end, "[REDACTED_SECRET]");
    }
}

pub fn redact_line(line: &str) -> String {
    let mut redacted = line.to_string();
    for marker in ["xprv", "tprv", "password=", "password:", "AUTHENTICATE "] {
        redact_token(&mut redacted, marker);
    }
    redacted
}

fn rotate_maker_log(path: &Path) {
    if path
        .metadata()
        .map(|m| m.len() < MAX_LOG_BYTES)
        .unwrap_or(true)
    {
        return;
    }
    let oldest = path.with_file_name("debug.log.3");
    let _ = std::fs::remove_file(oldest);
    for generation in (1..LOG_GENERATIONS).rev() {
        let from = path.with_file_name(format!("debug.log.{generation}"));
        let to = path.with_file_name(format!("debug.log.{}", generation + 1));
        let _ = std::fs::rename(from, to);
    }
    let _ = std::fs::rename(path, path.with_file_name("debug.log.1"));
}

#[derive(Debug)]
struct MakerLogRouter;

impl MakerLogRouter {
    fn maker_id_from_thread() -> Option<String> {
        std::thread::current()
            .name()
            .and_then(|name| name.strip_prefix("maker-"))
            .map(str::to_string)
    }

    fn port_from_message(message: &str) -> Option<u16> {
        let start = message.find('[')? + 1;
        let end = message[start..].find(']')? + start;
        message[start..end].parse().ok()
    }

    fn target_for(message: &str) -> Option<MakerLogTarget> {
        let targets = makers().lock().ok()?;
        if let Some(id) = Self::maker_id_from_thread() {
            if let Some(target) = targets.get(&id) {
                return Some(target.clone());
            }
        }
        if let Some(port) = Self::port_from_message(message) {
            if let Some(target) = targets.values().find(|target| target.network_port == port) {
                return Some(target.clone());
            }
        }
        (targets.len() == 1)
            .then(|| targets.values().next().cloned())
            .flatten()
    }
}

impl log::Log for MakerLogRouter {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        let message = redact_line(&record.args().to_string());
        let Some(target) = Self::target_for(&message) else {
            return;
        };
        let Ok(_write_guard) = MAKER_WRITE.lock() else {
            return;
        };
        if let Some(parent) = target.path.parent() {
            let _ = crate::security::fs::ensure_private_dir(parent);
        }
        rotate_maker_log(&target.path);
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        if let Ok(mut file) = options.open(target.path) {
            let _ = writeln!(file, "{} {} - {}", record.level(), record.target(), message);
        }
    }

    fn flush(&self) {}
}

/// Steps the frontend checklist renders for `Taker::init`, keyed by the ordered markers the
/// crate logs on its way between them. `Taker::init` is one opaque blocking call with no
/// progress hook, so its own log stream is the only signal for which phase is running; a marker
/// that stops matching after a crate upgrade leaves that step spinning rather than failing init.
const INIT_PHASE_MARKERS: &[(&str, u8)] = &[
    // `load_or_init` logs only on completion, so the backend connection and the decrypt that
    // follows it share one window and tick together.
    ("successfully loaded.", 2),
    ("New Wallet created at :", 2),
    ("Watcher initiated", 3),
    ("Checking wallet for unresolved swap contracts", 4),
];

/// What the last phase is doing once every step has ticked. Recovery has no step of its own —
/// it does nothing on most launches — but it blocks on a block being mined, so when it is the
/// reason init has not returned it has to say so. Only these fixed strings reach the webview:
/// never log text, which would carry whatever the crate logged into the UI.
const INIT_PHASE_NOTES: &[(&str, Option<&str>)] = &[
    (
        "Checking wallet for unresolved swap contracts",
        Some("Checking for interrupted swaps"),
    ),
    (
        "confirmation(s) on",
        Some("Reclaiming funds from an interrupted swap — waiting for a block to confirm it"),
    ),
];

impl InitPhase {
    /// Folds one crate log line in, reporting whether it moved. Phases only ever advance: the
    /// recovery pass re-logs lines the earlier phases also emit, and those must not walk the
    /// checklist backwards.
    fn advance(&mut self, message: &str) -> bool {
        let before = *self;
        if let Some((_, phase)) = INIT_PHASE_MARKERS
            .iter()
            .filter(|(marker, _)| message.contains(marker))
            .max_by_key(|(_, phase)| *phase)
        {
            if *phase > self.phase {
                self.phase = *phase;
                self.note = None;
            }
        }
        if let Some((_, note)) = INIT_PHASE_NOTES
            .iter()
            .find(|(marker, _)| message.contains(marker))
        {
            self.note = *note;
        }
        *self != before
    }
}

struct InitPhaseWatch {
    sink: EventSink,
    at: InitPhase,
}

static INIT_PHASE: Mutex<Option<InitPhaseWatch>> = Mutex::new(None);

/// Report `Taker::init`'s phase through `sink` until [`stop_watching_init_phases`].
pub fn watch_init_phases(sink: EventSink) {
    // Phase 0 is announced here rather than by a marker: it begins when `Taker::init` is
    // called, and a restore needs that edge to know its own step has finished.
    let at = InitPhase {
        phase: 0,
        note: None,
    };
    sink.publish(AppEvent::WalletInitPhase(at));
    if let Ok(mut guard) = INIT_PHASE.lock() {
        *guard = Some(InitPhaseWatch { sink, at });
    }
}

pub fn stop_watching_init_phases() {
    if let Ok(mut guard) = INIT_PHASE.lock() {
        *guard = None;
    }
}

#[derive(Debug)]
struct InitPhaseWatcher;

impl log::Log for InitPhaseWatcher {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        // `try_lock`, so a record logged from inside `emit` cannot deadlock against the guard
        // this call already holds.
        let Ok(mut guard) = INIT_PHASE.try_lock() else {
            return;
        };
        let Some(watch) = guard.as_mut() else {
            return;
        };
        if watch.at.advance(&record.args().to_string()) {
            watch.sink.publish(AppEvent::WalletInitPhase(watch.at));
        }
    }

    fn flush(&self) {}
}

fn build_config(taker_dir: Option<&PathBuf>) -> Config {
    let mut builder = Config::builder()
        .appender(Appender::builder().build("stdout", Box::new(ConsoleAppender::builder().build())))
        .appender(Appender::builder().build("maker_router", Box::new(MakerLogRouter)))
        .appender(Appender::builder().build("init_phase", Box::new(InitPhaseWatcher)));

    let root_appender = match taker_dir.and_then(|dir| file_appender(dir)) {
        Some(appender) => {
            builder = builder.appender(Appender::builder().build("taker_file", Box::new(appender)));
            "taker_file"
        }
        None => "stdout",
    };

    builder
        .logger(Logger::builder().build("bitcoincore_rpc", log::LevelFilter::Off))
        .logger(
            Logger::builder()
                .appender("maker_router")
                .additive(false)
                .build("openswap::maker", log::LevelFilter::Info),
        )
        .build(
            Root::builder()
                .appender(root_appender)
                .appender("init_phase")
                .build(log::LevelFilter::Info),
        )
        .expect("logger config references only appenders registered above")
}

fn rebuild() {
    let config = build_config(TAKER_DIR.lock().unwrap().as_ref());
    if let Some(handle) = HANDLE.get() {
        handle.set_config(config);
    } else if let Ok(handle) = log4rs::init_config(config) {
        let _ = HANDLE.set(handle);
    }
}

pub fn set_taker_dir(dir: PathBuf) {
    *TAKER_DIR.lock().unwrap() = Some(dir);
    rebuild();
}

pub fn register_maker(router_id: String, dir: PathBuf, network_port: u16) {
    makers().lock().unwrap().insert(
        router_id,
        MakerLogTarget {
            path: dir.join("debug.log"),
            network_port,
        },
    );
    rebuild();
}

pub fn unregister_maker(router_id: &str) {
    makers().lock().unwrap().remove(router_id);
}

/// Reads the last `want` lines without loading an unbounded log into memory.
pub fn tail_lines(path: &Path, want: usize) -> std::io::Result<Vec<String>> {
    let want = want.min(MAX_TAIL_LINES);
    if want == 0 || !path.exists() {
        return Ok(Vec::new());
    }
    let mut file = std::fs::File::open(path)?;
    let mut position = file.metadata()?.len();
    let mut bytes = Vec::new();
    const CHUNK: u64 = 8192;

    while position > 0
        && bytes.len() < MAX_TAIL_BYTES
        && bytes.iter().filter(|byte| **byte == b'\n').count() <= want
    {
        let remaining = (MAX_TAIL_BYTES - bytes.len()) as u64;
        let size = CHUNK.min(position).min(remaining);
        position -= size;
        file.seek(SeekFrom::Start(position))?;
        let mut chunk = vec![0; size as usize];
        file.read_exact(&mut chunk)?;
        chunk.extend(bytes);
        bytes = chunk;
    }

    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<_> = text.lines().collect();
    let start = lines.len().saturating_sub(want);
    Ok(lines[start..]
        .iter()
        .map(|line| redact_line(line))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{redact_line, InitPhase, MakerLogRouter};

    #[test]
    fn extracts_leading_bracketed_port() {
        assert_eq!(
            MakerLogRouter::port_from_message("[6102] started"),
            Some(6102)
        );
        assert_eq!(MakerLogRouter::port_from_message("started"), None);
    }

    /// Lines taken verbatim from a real `Taker::init`, including the recovery pass that
    /// re-emits wallet lines the earlier phases already matched.
    #[test]
    fn init_phases_advance_over_a_real_startup() {
        let mut at = InitPhase {
            phase: 0,
            note: None,
        };
        let seen = |at: &mut InitPhase, line: &str| {
            at.advance(line);
            (at.phase, at.note)
        };

        assert_eq!(
            seen(
                &mut at,
                "Wallet file at \"/Users/x/.openswap/taker/wallets/w\" successfully loaded."
            ),
            (2, None)
        );
        assert_eq!(seen(&mut at, "Watcher initiated"), (3, None));
        assert_eq!(seen(&mut at, "Rebuilding 3 watches from the wallet"), (3, None));
        // Must not match the wallet marker, which differs only by case and a trailing period.
        assert_eq!(
            seen(
                &mut at,
                "Successfully loaded config file from : /Users/x/.openswap/taker/config.toml"
            ),
            (3, None)
        );
        assert_eq!(
            seen(
                &mut at,
                "Successfully loaded offerbook at \"/Users/x/.openswap/taker/offerbook.json\""
            ),
            (3, None)
        );

        // Every step has ticked from here on; only the note still moves.
        assert_eq!(
            seen(&mut at, "Checking wallet for unresolved swap contracts..."),
            (4, Some("Checking for interrupted swaps"))
        );
        let (phase, note) = seen(
            &mut at,
            "Waiting for 1 confirmation(s) on 1 transaction(s)...",
        );
        assert_eq!(phase, 4);
        assert!(note.is_some_and(|note: &str| note.contains("waiting for a block")));
        // The recovery pass re-logs a wallet line from an earlier phase; nothing may rewind.
        assert_eq!(seen(&mut at, "Sync Started for \"w\""), (phase, note));
    }

    #[test]
    fn secrets_are_redacted_from_log_output() {
        let line = "password=hunter2 xprv123 AUTHENTICATE DEADBEEF";
        let redacted = redact_line(line);
        assert!(!redacted.contains("hunter2"));
        assert!(!redacted.contains("xprv123"));
        assert!(!redacted.contains("DEADBEEF"));
    }
}
