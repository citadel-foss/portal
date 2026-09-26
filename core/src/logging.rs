//! One process logger with dynamic routers. Maker server threads are named
//! `maker-{id}` and most maker records include `[port]`; both signals keep
//! concurrently running makers in separate log files. Wallet records go to the
//! wallet's own `debug.log` when they can be attributed (see `wallet_log_for`);
//! everything else goes to the app's `debug.log` under `~/.openswap`.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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
/// Whether the root logger is currently writing to the file rather than falling back to the
/// console. A host that silences stdout must not do so while this is false, or an unopenable
/// log file would mean no diagnostics anywhere at all.
static LOGS_TO_FILE: AtomicBool = AtomicBool::new(false);
static MAKERS: OnceLock<Mutex<HashMap<String, MakerLogTarget>>> = OnceLock::new();
/// Serializes appends to the router and wallet files, which rotate themselves.
static ROUTED_WRITE: Mutex<()> = Mutex::new(());

/// Open wallets, by name and data dir, for records that name their wallet or come from a
/// thread with no wallet of its own.
static WALLETS: Mutex<Vec<(String, PathBuf)>> = Mutex::new(Vec::new());

thread_local! {
    /// The wallet the code running on this thread is working for. The crate's own log lines
    /// carry no wallet, so this is what attributes the ones logged from our threads.
    static WALLET_SCOPE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

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

fn redact_line(line: &str) -> String {
    let mut redacted = line.to_string();
    for marker in ["xprv", "tprv", "password=", "password:", "AUTHENTICATE "] {
        redact_token(&mut redacted, marker);
    }
    redacted
}

fn rotate_log(path: &Path) {
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

/// Appends one record to a routed log file, rotating it first if it has grown too large.
fn append_line(path: &Path, record: &log::Record<'_>, message: &str) {
    let Ok(_write_guard) = ROUTED_WRITE.lock() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = crate::security::fs::ensure_private_dir(parent);
    }
    rotate_log(path);
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut file) = options.open(path) {
        use log4rs::encode::Encode;
        // The app log's own encoder, so every file reads the same; the record is rebuilt around
        // the redacted message.
        let mut encode = |args: std::fmt::Arguments<'_>| {
            let redacted = log::Record::builder()
                .args(args)
                .level(record.level())
                .target(record.target())
                .module_path(record.module_path())
                .file(record.file())
                .line(record.line())
                .build();
            log4rs::encode::pattern::PatternEncoder::default()
                .encode(&mut log4rs::encode::writer::simple::SimpleWriter(&mut file), &redacted)
        };
        let _ = encode(format_args!("{message}"));
    }
}

/// Marks this thread as working for one wallet until the guard drops.
pub struct WalletScope;

impl Drop for WalletScope {
    fn drop(&mut self) {
        WALLET_SCOPE.with(|scope| *scope.borrow_mut() = None);
    }
}

#[must_use]
pub fn wallet_scope(wallet_dir: PathBuf) -> WalletScope {
    WALLET_SCOPE.with(|scope| *scope.borrow_mut() = Some(wallet_dir));
    WalletScope
}

pub fn register_wallet(name: String, wallet_dir: PathBuf) {
    if let Ok(mut wallets) = WALLETS.lock() {
        wallets.push((name, wallet_dir));
    }
}

pub fn unregister_wallet(wallet_dir: &Path) {
    if let Ok(mut wallets) = WALLETS.lock() {
        wallets.retain(|(_, dir)| dir != wallet_dir);
    }
}

/// The wallet a record belongs to, if that can be told: the thread is working for one, the
/// record names one, or only one wallet is open and no router shares the process. The
/// crate's background threads (watcher, offer sync, recovery) name no wallet, so with several
/// open their lines stay in the app log rather than being guessed.
fn wallet_log_for(message: &str) -> Option<PathBuf> {
    if let Some(dir) = WALLET_SCOPE.with(|scope| scope.borrow().clone()) {
        return Some(dir.join("debug.log"));
    }
    let wallets = WALLETS.lock().ok()?;
    let named = wallets.iter().find(|(name, dir)| {
        message.contains(&format!("\"{name}\""))
            || message.contains(dir.to_string_lossy().as_ref())
    });
    if let Some((_, dir)) = named {
        return Some(dir.join("debug.log"));
    }
    let routers_idle = makers().lock().is_ok_and(|makers| makers.is_empty());
    match wallets.as_slice() {
        [(_, dir)] if routers_idle => Some(dir.join("debug.log")),
        _ => None,
    }
}

#[derive(Debug)]
struct WalletLogRouter;

impl log::Log for WalletLogRouter {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        let message = redact_line(&record.args().to_string());
        if let Some(path) = wallet_log_for(&message) {
            append_line(&path, record, &message);
        }
    }

    fn flush(&self) {}
}

/// Keeps a wallet's records out of the app log once they have gone to the wallet's own.
#[derive(Debug)]
struct NotWalletRecord;

impl log4rs::filter::Filter for NotWalletRecord {
    fn filter(&self, record: &log::Record<'_>) -> log4rs::filter::Response {
        if wallet_log_for(&redact_line(&record.args().to_string())).is_some() {
            log4rs::filter::Response::Reject
        } else {
            log4rs::filter::Response::Neutral
        }
    }
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
        append_line(&target.path, record, &message);
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
    /// The thread running this `Taker::init`. Several wallets can initialize at once, and the
    /// thread is what tells their log lines apart.
    thread: std::thread::ThreadId,
    wallet: PathBuf,
    sink: EventSink,
    at: InitPhase,
}

static INIT_PHASE: Mutex<Vec<InitPhaseWatch>> = Mutex::new(Vec::new());

/// Stops the watch when dropped, so an init that panics — a wrong password can — does not leave
/// its thread's watch behind.
pub struct InitPhaseWatchGuard {
    thread: std::thread::ThreadId,
}

impl Drop for InitPhaseWatchGuard {
    fn drop(&mut self) {
        if let Ok(mut watches) = INIT_PHASE.lock() {
            watches.retain(|watch| watch.thread != self.thread);
        }
    }
}

/// Report the phase of the `Taker::init` about to run on this thread, as an event for `wallet`,
/// for as long as the returned guard lives.
#[must_use]
pub fn watch_init_phases(sink: EventSink, wallet: PathBuf) -> InitPhaseWatchGuard {
    // Phase 0 is announced here rather than by a marker: it begins when `Taker::init` is
    // called, and a restore needs that edge to know its own step has finished.
    let at = InitPhase {
        phase: 0,
        note: None,
    };
    sink.publish_for(&wallet, AppEvent::WalletInitPhase(at));
    let thread = std::thread::current().id();
    if let Ok(mut watches) = INIT_PHASE.lock() {
        watches.push(InitPhaseWatch {
            thread,
            wallet,
            sink,
            at,
        });
    }
    InitPhaseWatchGuard { thread }
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
        let Ok(mut watches) = INIT_PHASE.try_lock() else {
            return;
        };
        // Some markers are logged by threads the crate spawns during init, not by the init
        // thread itself. Those can only be attributed while a single init is running; with
        // several, a skipped phase beats one ticked on the wrong wallet's checklist.
        let thread = std::thread::current().id();
        let index = match watches.iter().position(|watch| watch.thread == thread) {
            Some(index) => index,
            None if watches.len() == 1 => 0,
            None => return,
        };
        let watch = &mut watches[index];
        if watch.at.advance(&record.args().to_string()) {
            watch.sink.publish_for(&watch.wallet, AppEvent::WalletInitPhase(watch.at));
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
            builder = builder.appender(
                Appender::builder()
                    .filter(Box::new(NotWalletRecord))
                    .build("taker_file", Box::new(appender)),
            );
            "taker_file"
        }
        None => "stdout",
    };
    builder = builder.appender(Appender::builder().build("wallet_router", Box::new(WalletLogRouter)));

    builder
        .logger(Logger::builder().build("bitcoincore_rpc", log::LevelFilter::Off))
        // The watchtower re-announces the same handful of txids to every relay on every pass,
        // at Info: in one short session on an empty wallet that was 293 of 390 lines, 270 of
        // them three messages repeated ninety times each. Warn keeps the relay failures, which
        // are the only part anyone reads, and leaves the file usable for everything else.
        .logger(Logger::builder().build(
            "openswap::watch_tower::nostr",
            log::LevelFilter::Warn,
        ))
        .logger(
            Logger::builder()
                .appender("maker_router")
                .additive(false)
                .build("openswap::maker", log::LevelFilter::Info),
        )
        .build(
            Root::builder()
                .appender(root_appender)
                .appender("wallet_router")
                .appender("init_phase")
                .build(log::LevelFilter::Info),
        )
        .expect("logger config references only appenders registered above")
}

fn rebuild() {
    let dir = TAKER_DIR.lock().unwrap().clone();
    let to_file = dir.as_ref().is_some_and(|d| file_appender(d).is_some());
    let config = build_config(dir.as_ref());
    // Set only once a logger is actually installed. A host reads this to decide whether it
    // may silence stdout, and claiming the file is taking the output when installation
    // failed would leave the diagnostics nowhere at all.
    if let Some(handle) = HANDLE.get() {
        handle.set_config(config);
        LOGS_TO_FILE.store(to_file, Ordering::SeqCst);
    } else if let Ok(handle) = log4rs::init_config(config) {
        let _ = HANDLE.set(handle);
        LOGS_TO_FILE.store(to_file, Ordering::SeqCst);
    }
}

/// True when the log file is open and taking the root logger's output, so the console is
/// carrying nothing that would be lost by silencing it.
pub fn logs_to_file() -> bool {
    LOGS_TO_FILE.load(Ordering::SeqCst)
}

/// Where the app's own `debug.log` lives. Set once at startup, before anything is logged.
pub fn set_log_dir(dir: PathBuf) {
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

    /// The only test touching the wallet registry, which is process-wide.
    #[test]
    fn wallet_records_are_attributed_only_when_it_can_be_told() {
        use super::{register_wallet, unregister_wallet, wallet_log_for, wallet_scope};
        use std::path::PathBuf;
        let alpha = PathBuf::from("/tmp/portal-test/takers/alpha");
        let beta = PathBuf::from("/tmp/portal-test/takers/beta");

        assert_eq!(wallet_log_for("Watcher initiated"), None, "no wallet open");

        register_wallet("alpha".into(), alpha.clone());
        assert_eq!(
            wallet_log_for("Watcher initiated"),
            Some(alpha.join("debug.log")),
            "the only wallet open gets unnamed lines"
        );

        register_wallet("beta".into(), beta.clone());
        assert_eq!(wallet_log_for("Watcher initiated"), None, "two open: not guessed");
        assert_eq!(
            wallet_log_for("Sync Started for \"beta\""),
            Some(beta.join("debug.log")),
            "a line naming its wallet goes there"
        );
        {
            let _scope = wallet_scope(alpha.clone());
            assert_eq!(
                wallet_log_for("Watcher initiated"),
                Some(alpha.join("debug.log")),
                "a line from a thread working for a wallet goes there"
            );
        }
        assert_eq!(wallet_log_for("Watcher initiated"), None, "the scope ends with its guard");

        unregister_wallet(&alpha);
        unregister_wallet(&beta);
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
