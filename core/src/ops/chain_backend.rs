//! Which chain data source the wallet talks to: the bundled Electrum server or a
//! Bitcoin Core node the user added themselves.
//!
//! Held on the Rust side rather than in the frontend because four unrelated call sites need
//! it — taker init, wallet restore, maker server construction, and the connectivity probe —
//! and only the first of those gets a config object from the UI.
//!
//! Nothing here is written to disk. Every launch starts from the constants below and the
//! connection gate; an edit lives for the session only, so a node's RPC password is never at
//! rest. `remove_legacy_config` deletes the file earlier versions did persist.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use openswap::bitcoin::{Address, Network};
use openswap::bitcoind::bitcoincore_rpc::bitcoincore_rpc_json::ListUnspentResultEntry;
use openswap::bitcoind::bitcoincore_rpc::jsonrpc::{self, simple_http};
use openswap::bitcoind::bitcoincore_rpc::{Auth, Client, RpcApi};
use openswap::utill::get_taker_dir;
use openswap::utill::MIN_RELAY_FEE_RATE;
use openswap::wallet::{
    AnyBlockchain, BackendConfig, Blockchain, CoreRpcConfig, ElectrumConfig, FeePriority,
    WalletError,
};

use crate::error::{AppError, ErrorCode};
use crate::types::{
    BackendStatus, ChainBackendConfig, ChainBackendKind, ChainBackendView, ElectrumBackendDto,
    ElectrumPresetDto, FeeEstimate, NodeBackendDto, NodeBackendViewDto,
};

/// One bounded attempt is enough for a probe. The backend's own defaults (a 120s proxied
/// read timeout plus `max_retries` reconnects) would stall the UI for minutes before verdict.
const PROBE_TIMEOUT_SECS: u8 = 15;

/// Written by versions that persisted the backend, including a node's RPC password.
const LEGACY_FILE_NAME: &str = "backend.json";

/// Each session's backend, seeded from the defaults on first read. Per session so one browser
/// passing the gate with a different backend never repoints another's.
static SESSIONS: Mutex<Option<HashMap<String, ChainBackendConfig>>> = Mutex::new(None);

/// Sessions that adopted a backend at the connection gate. Separate from `SESSIONS`, which `load`
/// seeds with defaults on any read: having a config there does not mean the gate was passed.
static PASSED_GATE: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);

/// Whether this session has been through the connection gate. Asked after a reload, so a page
/// that needs no wallet (a router's) is not sent back to the gate it already cleared.
pub fn passed_gate(session: &str) -> bool {
    PASSED_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|passed| passed.contains(session))
}

/// Cached by complete endpoint/route fingerprint, never by process first-use.
/// The servers the gate offers, newest verified list. Every mainnet entry was probed against
/// the live chain before landing and agreed on the same tip; a preset that does not answer is
/// worse than no preset, so `fulcrum.sethforprivacy.com` was dropped for timing out. Each also
/// has to present a CA-signed certificate: the crate validates the domain of every clearnet
/// server, so a self-signed one such as `electrum.emzy.de` never connects.
///
/// Reference data, deliberately not configuration: nothing here is written to disk and the
/// choice is not remembered between launches. The signet entry is `DEFAULT_ELECTRUM_URL`
/// itself rather than a second copy of the same string.
const ELECTRUM_PRESETS: &[(&str, &str, &str)] = &[
    (
        "Portal signet",
        crate::types::DEFAULT_ELECTRUM_URL,
        "signet",
    ),
    (
        "Blockstream",
        "ssl://electrum.blockstream.info:50002",
        "bitcoin",
    ),
    ("DIY Nodes", "ssl://electrum.diynodes.com:50002", "bitcoin"),
    ("Grey", "ssl://fulcrum.grey.pw:51002", "bitcoin"),
];

pub fn electrum_presets() -> Vec<ElectrumPresetDto> {
    ELECTRUM_PRESETS
        .iter()
        .map(|(label, url, network)| ElectrumPresetDto {
            label: (*label).to_string(),
            url: (*url).to_string(),
            network: (*network).to_string(),
        })
        .collect()
}

static ELECTRUM_NETWORK: Mutex<Option<(String, Option<Network>)>> = Mutex::new(None);

/// Portal's own chain reads — the liveness probe, a recovery contract's confirmations, a fee
/// tier off the session's own server — cannot borrow the wallet's connection: `Wallet::blockchain`
/// is private to the crate. Building one per call charged a handshake, and over Tor a whole
/// circuit, to every one of them; one is held here per route instead and lent to each in turn.
///
/// A few entries, most recently used first: one route is the common case, but two sessions on
/// different servers — or two Core wallets, which cannot share a client — would otherwise evict
/// each other on every alternation and reconnect every time. Bounded so it cannot grow into a
/// cache nothing evicts.
const MAX_HELD: usize = 4;
static SIDE_CHAIN: Mutex<Vec<(String, AnyBlockchain)>> = Mutex::new(Vec::new());

/// A liveness verdict is the same for every caller that asks within a few seconds — the shell on
/// mount, a router preflight, the check before each wallet sync — so the saved route's is held
/// briefly, exactly as a fee estimate is. Short enough that a server going down is still noticed.
const PROBE_CACHE_TTL: Duration = Duration::from_secs(15);
static PROBE_CACHE: Mutex<Option<(String, std::time::Instant, BackendStatus)>> = Mutex::new(None);

/// Opaque identity for a Core node's credentials.
///
/// [`fingerprint`] names a node by host and ports only, so a config that differs from the last
/// one by password alone would otherwise reuse both a connection authenticated with the old
/// credentials and a verdict taken under them. Hashed rather than carried: this ends up in a
/// cache key, and a password has no business being one. Only a lookup key, never a password
/// hash — there is nothing here an attacker does not already supply.
fn credential_identity(node: &NodeBackendDto) -> String {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    node.username.hash(&mut hasher);
    node.password.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Route identity for the caches here: [`fingerprint`] plus, for a node, who it logs in as.
fn route_identity(config: &ChainBackendConfig, socks_port: Option<u16>) -> String {
    let route = fingerprint(config, socks_port);
    match (&config.kind, &config.node) {
        (ChainBackendKind::CoreRpc, Some(node)) => {
            format!("{route}|{}", credential_identity(node))
        }
        _ => route,
    }
}

/// Deletes the config earlier versions wrote. Called once at startup: ceasing to write it
/// would otherwise leave a plaintext RPC password on disk forever.
pub fn remove_legacy_config() {
    let Ok(dir) = get_taker_dir() else { return };
    let path = dir.join(LEGACY_FILE_NAME);
    match std::fs::remove_file(&path) {
        Ok(()) => log::info!("removed persisted backend config at {}", path.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("could not remove {}: {e}", path.display()),
    }
}

/// This session's backend, seeded from the code defaults on first read.
pub(crate) fn load(session: &str) -> ChainBackendConfig {
    let mut sessions = SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
    sessions
        .get_or_insert_with(HashMap::new)
        .entry(session.to_string())
        .or_default()
        .clone()
}

fn store(session: &str, config: ChainBackendConfig) {
    SESSIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(session.to_string(), config);
}

/// Drops an ended session's choice, including any node password it held.
pub fn forget_session(session: &str) {
    if let Some(sessions) = SESSIONS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        sessions.remove(session);
    }
    if let Some(passed) = PASSED_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_mut()
    {
        passed.remove(session);
    }
}

/// The crate cannot resolve an onion host without a proxy, so Tor is not optional there
/// regardless of what the user picked.
fn electrum_needs_tor(dto: &ElectrumBackendDto) -> bool {
    let host = dto
        .url
        .split_once("://")
        .map_or(dto.url.as_str(), |(_, rest)| rest);
    dto.use_tor
        || host
            .rsplit_once(':')
            .map_or(host, |(h, _)| h)
            .ends_with(".onion")
}

/// Notification ping cadence for a direct connection. The crate's own default there is one
/// second, which for a watchtower connection is a request per second per router for the life of
/// the process; its proxied default is ten, and its own note is that every consumer of
/// watchtower latency reacts on a block timescale. Five seconds sits inside that and costs a
/// fifth of the traffic. Left to the crate on a proxied route, which already paces itself.
const DIRECT_PING_SECS: u64 = 5;

fn electrum_config(dto: &ElectrumBackendDto, socks_port: Option<u16>) -> ElectrumConfig {
    let socks_port = live_socks_port(socks_port);
    let socks5 = electrum_needs_tor(dto)
        .then(|| socks_port.map(|port| format!("127.0.0.1:{port}")))
        .flatten();
    ElectrumConfig {
        url: dto.url.clone(),
        poll_interval_secs: socks5.is_none().then_some(DIRECT_PING_SECS),
        socks5,
        timeout: None,
        max_retries: 3,
    }
}

fn core_rpc_config(dto: &NodeBackendDto, wallet_name: &str) -> CoreRpcConfig {
    CoreRpcConfig {
        url: format!("{}:{}", dto.host, dto.port),
        auth: Auth::UserPass(dto.username.clone(), dto.password.clone()),
        wallet_name: wallet_name.to_string(),
        zmq_addr: format!("tcp://{}:{}", dto.host, dto.zmq_port),
    }
}

/// For a one-off read the UI waits on: the backend's defaults would hold a dead Electrum server
/// for minutes before the caller's fallback could run.
pub(crate) fn resolve_bounded(
    config: &ChainBackendConfig,
    wallet_name: &str,
    socks_port: Option<u16>,
) -> Result<BackendConfig, AppError> {
    let mut backend = resolve_from(config, wallet_name, socks_port)?;
    if let BackendConfig::Electrum(electrum) = &mut backend {
        electrum.max_retries = 0;
        electrum.timeout = Some(PROBE_TIMEOUT_SECS);
    }
    Ok(backend)
}

/// Which held connection serves this caller.
fn pool_key(config: &ChainBackendConfig, wallet_name: &str, socks_port: Option<u16>) -> String {
    match config.kind {
        // Electrum has no server-side wallet, so one connection serves every wallet on a route.
        ChainBackendKind::Electrum => route_identity(config, socks_port),
        // Core's client is bound to a named watch-only wallet, so one connection per wallet.
        ChainBackendKind::CoreRpc => {
            format!("{}|{wallet_name}", route_identity(config, socks_port))
        }
    }
}

/// Keeps `chain` as the most recently used entry, dropping the least recently used past the cap.
///
/// Any earlier entry for the same route goes: with the lock released across the I/O, two callers
/// on one route can each have opened a connection, and keeping both would hold a route twice
/// over while evicting another.
fn hold(held: &mut Vec<(String, AnyBlockchain)>, key: String, chain: AnyBlockchain) {
    held.retain(|(route, _)| route != &key);
    held.insert(0, (key, chain));
    held.truncate(MAX_HELD);
}

/// Takes this route's connection out of the set, if one is held.
fn take(key: &str) -> Option<AnyBlockchain> {
    let mut held = SIDE_CHAIN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let at = held.iter().position(|(route, _)| route == key)?;
    Some(held.remove(at).1)
}

/// Puts a working connection back as the most recently used entry.
fn put(key: String, chain: AnyBlockchain) {
    let mut held = SIDE_CHAIN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    hold(&mut held, key, chain);
}

/// Runs one chain read against the connection held for this route, opening it if there is none.
///
/// Blocking: call it from `spawn_blocking` or a thread, never on the async runtime.
///
/// The set's lock is taken only to pick a connection up and to put it back, never across the
/// handshake or the read. One route's server going quiet can stall a read for
/// `PROBE_TIMEOUT_SECS`, and holding the lock through that would stall every other route's
/// reads — a Core wallet's balance behind a silent Electrum server — for the same duration.
/// Reads on one route therefore run concurrently; what this avoids is a handshake per read, not
/// overlapping reads.
///
/// Opened with [`resolve_bounded`], so a read the UI waits on stays bounded and nothing sits
/// through the crate's own reconnect holding the next caller behind it. At most [`MAX_HELD`]
/// routes are kept, most recently used first.
///
/// A read that fails on a *reused* connection is retried once on a fresh one. An idle Electrum
/// server dropping the session is routine — the crate says so itself — and with no retries
/// configured its `call` reports that dropped socket as the backend being unreachable. Without
/// the second attempt, every idle gap would cost one spurious "chain backend unreachable": a
/// connection chip flipped to down, and a wallet sync cycle refused by the check before it.
/// A connection opened *in this call* is not retried: the server really is not answering.
pub(crate) fn with_chain<T>(
    config: &ChainBackendConfig,
    wallet_name: &str,
    socks_port: Option<u16>,
    read: impl Fn(&AnyBlockchain) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let key = pool_key(config, wallet_name, socks_port);
    // Taken out of the set while in use and put back only on success, so a connection a read has
    // just broken is never left behind for the next caller to inherit.
    let (chain, reused) = match take(&key) {
        Some(chain) => (chain, true),
        None => (open(config, wallet_name, socks_port)?, false),
    };
    let outcome = read(&chain);
    if outcome.is_ok() {
        put(key, chain);
        return outcome;
    }
    drop(chain);
    let Err(failed) = outcome else { unreachable!("the Ok arm returned above") };
    if !reused {
        // Opened for this very call and it still failed: the server is not answering, and a
        // second attempt would only pay another handshake to be told so again.
        return Err(failed);
    }
    // Info, not Debug: this happens once per dropped socket, not once per read, and it is the
    // only evidence that the held connection is being recycled rather than silently failing.
    // At Debug it is invisible in the default log, where the question is actually asked.
    log::info!("reopening the held chain connection after: {}", failed.message);
    let fresh = open(config, wallet_name, socks_port)?;
    let retried = read(&fresh);
    if retried.is_ok() {
        put(key, fresh);
    }
    retried
}

fn open(
    config: &ChainBackendConfig,
    wallet_name: &str,
    socks_port: Option<u16>,
) -> Result<AnyBlockchain, AppError> {
    let backend = resolve_bounded(config, wallet_name, socks_port)?;
    Ok(AnyBlockchain::from_config(&backend)?)
}

/// Build the wallet backend the user selected. `wallet_name` names the watch-only
/// wallet on their node; Electrum has no server-side wallet so it ignores it.
pub(crate) fn resolve(
    session: &str,
    wallet_name: &str,
    socks_port: Option<u16>,
) -> Result<BackendConfig, AppError> {
    let config = load(session);
    resolve_from(&config, wallet_name, socks_port)
}

/// Resolves a previously loaded config so callers can use the same snapshot for
/// initialization and session bookkeeping.
pub(crate) fn resolve_from(
    config: &ChainBackendConfig,
    wallet_name: &str,
    socks_port: Option<u16>,
) -> Result<BackendConfig, AppError> {
    match config.kind {
        ChainBackendKind::Electrum => Ok(BackendConfig::Electrum(electrum_config(
            &config.electrum,
            socks_port,
        ))),
        ChainBackendKind::CoreRpc => {
            let node = config.node.as_ref().ok_or_else(|| {
                AppError::new(
                    ErrorCode::InvalidInput,
                    "no Bitcoin node is configured — add one in Settings or switch back to Electrum",
                )
            })?;
            Ok(BackendConfig::CoreRpc(core_rpc_config(node, wallet_name)))
        }
    }
}

fn validate(config: &ChainBackendConfig) -> Result<(), AppError> {
    let invalid = |msg: &str| AppError::new(ErrorCode::InvalidInput, msg.to_string());
    if config.electrum.url.chars().any(char::is_control) {
        return Err(invalid("Electrum URL contains control characters"));
    }
    let Some((scheme, authority)) = config.electrum.url.split_once("://") else {
        return Err(invalid(
            "Electrum URL must include a scheme, e.g. tcp://host:50001",
        ));
    };
    if !matches!(scheme, "tcp" | "ssl") || authority.is_empty() || !authority.contains(':') {
        return Err(invalid(
            "Electrum URL must use tcp:// or ssl:// with an explicit port",
        ));
    }
    if authority.contains('@') || authority.contains('#') {
        return Err(invalid(
            "Electrum URL cannot contain credentials or a fragment",
        ));
    }
    match &config.node {
        Some(node) => {
            if node.host.trim().is_empty() {
                return Err(invalid("node host cannot be empty"));
            }
            if node.port == 0 || node.zmq_port == 0 {
                return Err(invalid("node RPC and ZMQ ports must be set"));
            }
        }
        None if config.kind == ChainBackendKind::CoreRpc => {
            return Err(invalid("cannot select a node before one is added"));
        }
        None => {}
    }
    Ok(())
}

/// Falls back to the port Portal's own Tor is running on, for probes that happen before the
/// taker has pinned a route. `None` only before Tor starts, when no Tor route can work anyway.
fn live_socks_port(pinned: Option<u16>) -> Option<u16> {
    pinned.or_else(|| crate::tor::runtime().map(|tor| tor.socks_port))
}

pub(crate) fn fingerprint(config: &ChainBackendConfig, socks_port: Option<u16>) -> String {
    match config.kind {
        ChainBackendKind::Electrum => format!(
            "electrum|{}|{}|{:?}",
            config.electrum.url,
            electrum_needs_tor(&config.electrum),
            live_socks_port(socks_port)
        ),
        ChainBackendKind::CoreRpc => config.node.as_ref().map_or_else(
            || "core|missing".to_string(),
            |node| format!("core|{}|{}|{}", node.host, node.port, node.zmq_port),
        ),
    }
}

/// Names a backend for a sentence shown to the user. Never includes a credential.
pub(crate) fn describe(config: &ChainBackendConfig) -> String {
    match (&config.kind, &config.node) {
        (ChainBackendKind::CoreRpc, Some(node)) => format!("node {}:{}", node.host, node.port),
        _ => config.electrum.url.clone(),
    }
}

pub(crate) async fn preflight_active(
    taker: &crate::state::TakerInstance,
) -> Result<String, AppError> {
    let config = taker.chain_backend.clone();
    let socks_port = Some(taker.socks_port);
    let route_fingerprint = fingerprint(&config, socks_port);
    let status = tokio::task::spawn_blocking(move || probe(&config, socks_port))
        .await
        .map_err(AppError::internal)?;
    if !status.reachable {
        return Err(AppError::new(
            ErrorCode::RpcUnreachable,
            status
                .error
                .unwrap_or_else(|| "active chain backend preflight failed".to_string()),
        ));
    }
    Ok(route_fingerprint)
}

fn to_view(config: ChainBackendConfig) -> ChainBackendView {
    ChainBackendView {
        kind: config.kind,
        electrum: config.electrum,
        node: config.node.map(|node| NodeBackendViewDto {
            host: node.host,
            port: node.port,
            username: node.username,
            zmq_port: node.zmq_port,
        }),
    }
}

/// Fills in the password the browser was never sent, but only for the node it was saved
/// against. Matching on identity is the whole point: without it, a caller can name any host
/// and leave the password blank, and the saved credential is handed to that host instead
/// — in the clear, since Core RPC is plaintext Basic auth.
fn merge_preserved_password(session: &str, mut config: ChainBackendConfig) -> ChainBackendConfig {
    if let Some(node) = config.node.as_mut() {
        if node.password.is_empty() {
            if let Some(saved) = load(session).node {
                if saved.host == node.host
                    && saved.port == node.port
                    && saved.username == node.username
                {
                    node.password = saved.password;
                }
            }
        }
    }
    config
}

pub fn get_chain_backend(session: &str) -> ChainBackendView {
    to_view(load(session))
}

/// Adopts a backend for this session.
pub fn set_chain_backend(session: &str, config: ChainBackendConfig) -> Result<(), AppError> {
    let config = merge_preserved_password(session, config);
    validate(&config)?;
    store(session, config);
    PASSED_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(std::collections::HashSet::new)
        .insert(session.to_string());
    Ok(())
}

/// Records the chain a wallet or router has just opened on, unless Portal already has it. Off
/// the open path: it costs a chain round trip, and nothing waits on the answer.
pub fn record_network_once(
    session: &str,
    dir: std::path::PathBuf,
    config: ChainBackendConfig,
    socks_port: u16,
) {
    if crate::storage::recorded_network(&dir).is_some() {
        return;
    }
    let session = session.to_string();
    tokio::spawn(async move {
        let Ok(BackendStatus {
            chain: Some(chain), ..
        }) = check_backend(&session, Some(config), Some(socks_port)).await
        else {
            return;
        };
        if let Err(e) = crate::storage::record_network(&dir, &chain) {
            log::warn!("could not record the network of {}: {e:?}", dir.display());
        }
    });
}

pub async fn check_backend(
    session: &str,
    caller_supplied: Option<ChainBackendConfig>,
    socks_port: Option<u16>,
) -> Result<BackendStatus, AppError> {
    // Only the saved route is answered from the cache. A caller-supplied config is probed live
    // every time: the settings page asks precisely because the user just changed something, and
    // a verdict taken seconds ago would describe the server they replaced.
    let cacheable = caller_supplied.is_none();
    let config = match caller_supplied {
        Some(config) => merge_preserved_password(session, config),
        None => load(session),
    };
    // The same rules `set_chain_backend` applies. Probing reaches the network with a
    // credential attached, so it cannot be the lenient path — a caller-chosen host would
    // otherwise receive the saved RPC password, and the reachability of arbitrary addresses
    // would make this a scanner for whatever the server can see.
    validate(&config)?;
    let route = fingerprint(&config, socks_port);
    if cacheable {
        if let Some(fresh) = cached_probe(&route) {
            return Ok(fresh);
        }
    }
    let status = tokio::task::spawn_blocking(move || probe(&config, socks_port))
        .await
        .map_err(AppError::internal)?;
    if cacheable {
        if let Ok(mut cache) = PROBE_CACHE.lock() {
            *cache = Some((route, std::time::Instant::now(), status.clone()));
        }
    }
    Ok(status)
}

/// The verdict for `route` if one was taken inside [`PROBE_CACHE_TTL`].
fn cached_probe(route: &str) -> Option<BackendStatus> {
    let cache = PROBE_CACHE.lock().ok()?;
    let (cached_route, at, status) = cache.as_ref()?;
    (cached_route == route && at.elapsed() < PROBE_CACHE_TTL).then(|| status.clone())
}

/// One bounded attempt. Also what stands between a caller and the crate's sync, which retries
/// an unreachable backend until it answers or the wallet's owner shuts down.
pub(crate) fn probe(config: &ChainBackendConfig, socks_port: Option<u16>) -> BackendStatus {
    match config.kind {
        ChainBackendKind::Electrum => probe_electrum(config, socks_port),
        ChainBackendKind::CoreRpc => match &config.node {
            Some(node) => probe_core_rpc(node),
            None => unreachable("no Bitcoin node is configured".to_string()),
        },
    }
}

/// Mainnet rates through the crate's estimator, on every chain: a test network's fee market is
/// empty, so its own estimate would tell nobody what the same transaction costs for real. The
/// session's server is asked when it is itself on mainnet, a mainnet preset otherwise.
pub async fn estimate_fees(session: &str) -> Result<FeeEstimate, AppError> {
    let config = load(session);
    let route = fingerprint(&config, None);
    if let Some((_, _, fees)) = FEE_ESTIMATES
        .lock()?
        .as_ref()
        .filter(|(cached, at, _)| *cached == route && at.elapsed() < FEE_ESTIMATE_TTL)
    {
        return Ok(fees.clone());
    }
    let fees = estimate_fees_uncached(config).await?;
    *FEE_ESTIMATES.lock()? = Some((route, std::time::Instant::now(), fees.clone()));
    Ok(fees)
}

/// Every fee picker asks on mount, and a non-mainnet session's answer comes from a remote
/// mainnet server; one answer serves them all for this long.
const FEE_ESTIMATE_TTL: Duration = Duration::from_secs(60);
static FEE_ESTIMATES: Mutex<Option<(String, std::time::Instant, FeeEstimate)>> = Mutex::new(None);

async fn estimate_fees_uncached(config: ChainBackendConfig) -> Result<FeeEstimate, AppError> {
    tokio::task::spawn_blocking(move || -> Result<FeeEstimate, AppError> {
        let use_tor =
            config.kind == ChainBackendKind::Electrum && electrum_needs_tor(&config.electrum);
        let socks_port = live_socks_port(None);
        if use_tor && socks_port.is_none() {
            return Err(AppError::new(
                ErrorCode::TorUnreachable,
                "Tor is not available for fee estimates",
            ));
        }
        match config.kind {
            ChainBackendKind::CoreRpc => {
                if let Some(node) = &config.node {
                    if let Ok(client) = bounded_core_client(node) {
                        if client
                            .get_blockchain_info()
                            .is_ok_and(|info| info.chain == Network::Bitcoin)
                        {
                            // CoreRPC keeps its transport private. Use the same targets and
                            // conversion as the crate, with a bounded client for these UI reads.
                            if let Ok(fees) = fee_tiers(|priority| {
                                let estimate = client.estimate_smart_fee(priority as u16, None)?;
                                estimate
                                    .fee_rate
                                    .map(|rate| rate.to_sat() as f64 / 1000.0)
                                    .ok_or_else(|| {
                                        WalletError::General("no fee estimate".to_string())
                                    })
                            }) {
                                return Ok(fees);
                            }
                        }
                    }
                }
            }
            ChainBackendKind::Electrum => {
                // Both reads on the connection already held for this route: the fee pickers ask
                // on mount, and a handshake apiece is what that used to cost.
                let own = with_chain(&config, "", socks_port, |chain| {
                    // A session not on mainnet is not a failure — the presets below are its
                    // answer — but a read that failed is, and it has to reach `with_chain` for
                    // the connection to be recycled.
                    if chain.get_blockchain_info()?.chain != Network::Bitcoin {
                        return Ok(None);
                    }
                    fee_tiers(|priority| chain.estimate_feerate(priority)).map(Some)
                });
                if let Ok(Some(fees)) = own {
                    return Ok(fees);
                }
            }
        }
        let mut failure = None;
        for (_, url, _) in ELECTRUM_PRESETS
            .iter()
            .filter(|(_, _, network)| *network == "bitcoin")
        {
            let mut electrum = electrum_config(
                &ElectrumBackendDto {
                    url: (*url).to_string(),
                    use_tor,
                },
                socks_port,
            );
            electrum.max_retries = 0;
            electrum.timeout = Some(PROBE_TIMEOUT_SECS);
            match AnyBlockchain::from_config(&BackendConfig::Electrum(electrum))
                .map_err(AppError::from)
                .and_then(|chain| fee_tiers(|priority| chain.estimate_feerate(priority)))
            {
                Ok(fees) => return Ok(fees),
                Err(e) => failure = Some(e),
            }
        }
        Err(failure.unwrap_or_else(|| AppError::internal("no mainnet Electrum preset")))
    })
    .await
    .map_err(AppError::internal)?
}

fn fee_tiers(
    estimate: impl Fn(FeePriority) -> Result<f64, WalletError>,
) -> Result<FeeEstimate, AppError> {
    let mut rates = [None; 3];
    for (slot, priority) in
        rates
            .iter_mut()
            .zip([FeePriority::Urgent, FeePriority::Medium, FeePriority::Low])
    {
        match estimate(priority) {
            // The relay floor, as the crate's own recovery rates apply it: a lower rate would
            // not relay.
            Ok(rate) => *slot = Some(rate.max(MIN_RELAY_FEE_RATE)),
            Err(e @ WalletError::ElectrumUnreachable { .. }) => return Err(e.into()),
            Err(e) => log::debug!("no {priority:?} fee estimate: {e:?}"),
        }
    }
    if rates.iter().all(Option::is_none) {
        return Err(AppError::new(
            ErrorCode::RpcUnreachable,
            "the chain server returned no fee estimate",
        ));
    }
    let [fast, medium, slow] = rates;
    Ok(FeeEstimate { fast, medium, slow })
}

fn unreachable(error: String) -> BackendStatus {
    failed(ErrorCode::RpcUnreachable, error)
}

fn failed(failure: ErrorCode, error: String) -> BackendStatus {
    BackendStatus {
        reachable: false,
        error: Some(error),
        failure: Some(failure),
        chain: None,
        blocks: None,
        synced: false,
        subversion: None,
        verification_progress: None,
        signet_challenge: None,
    }
}

fn probe_electrum(config: &ChainBackendConfig, socks_port: Option<u16>) -> BackendStatus {
    // On the held connection: the first probe completes the Electrum handshake and checks the
    // server's genesis hash — which a raw socket probe cannot — and every one after it is a
    // single round trip on that same socket. A failed read drops the connection, so a server
    // that went away is still reported as gone.
    match with_chain(config, "", socks_port, |chain| {
        chain.get_blockchain_info().map_err(AppError::from)
    }) {
        Ok(info) => BackendStatus {
            reachable: true,
            failure: None,
            error: None,
            chain: Some(info.chain.to_string()),
            blocks: Some(info.blocks),
            synced: true,
            subversion: None,
            verification_progress: Some(1.0),
            signet_challenge: None,
        },
        Err(e) => unreachable(format!("{e:?}")),
    }
}

fn bounded_core_client(node: &NodeBackendDto) -> Result<Client, AppError> {
    let url = format!("http://{}:{}", node.host, node.port);
    let transport = simple_http::Builder::new()
        .url(&url)
        .map_err(AppError::internal)?
        .auth(node.username.clone(), Some(node.password.clone()))
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS as u64))
        .build();
    Ok(Client::from_jsonrpc(jsonrpc::Client::with_transport(
        transport,
    )))
}

fn probe_core_rpc(node: &NodeBackendDto) -> BackendStatus {
    let url = format!("http://{}:{}", node.host, node.port);
    let client = match bounded_core_client(node) {
        Ok(client) => client,
        Err(e) => return unreachable(e.message),
    };
    let info = match client.get_blockchain_info() {
        Ok(i) => i,
        Err(e) => {
            let msg = format!("{e:?}");
            // Already distinguished here; it was being flattened into "unreachable" on the
            // way out, leaving the gate unable to tell a wrong password from a dead node.
            if msg.contains("401") || msg.to_lowercase().contains("auth") {
                return failed(
                    ErrorCode::RpcAuthFailed,
                    format!("{url} rejected the RPC username or password"),
                );
            }
            return unreachable(msg);
        }
    };
    BackendStatus {
        reachable: true,
        failure: None,
        error: None,
        chain: Some(info.chain.to_string()),
        blocks: Some(info.blocks),
        synced: !info.initial_block_download && info.blocks == info.headers,
        subversion: client.get_network_info().ok().map(|n| n.subversion),
        verification_progress: Some(info.verification_progress),
        signet_challenge: signet_challenge(&client, info.chain),
    }
}

/// `GetBlockchainInfoResult` has no field for it, so the raw JSON is re-read rather than the
/// typed call being replaced. Only on signet: every other chain omits the key entirely.
fn signet_challenge(client: &Client, chain: Network) -> Option<String> {
    if chain != Network::Signet {
        return None;
    }
    client
        .call::<serde_json::Value>("getblockchaininfo", &[])
        .ok()?
        .get("signet_challenge")?
        .as_str()
        .map(str::to_string)
}

/// Address of a UTXO, re-derived from its scriptPubKey when the backend left it unset:
/// Electrum's `list_unspent` fills only the script (Core RPC fills the address), so the
/// wallet's UTXOs would otherwise have no address at all.
pub(crate) fn utxo_address(
    entry: &ListUnspentResultEntry,
    backend: &ChainBackendConfig,
    socks_port: Option<u16>,
) -> Option<String> {
    if let Some(address) = &entry.address {
        return Some(address.clone().assume_checked().to_string());
    }
    let network = electrum_network(backend, socks_port)?;
    Address::from_script(&entry.script_pub_key, network)
        .ok()
        .map(|a| a.to_string())
}

/// `Wallet` keeps its network private, so it comes from the Electrum handshake. Successful
/// route-specific probes are cached; failures are retried on the next UTXO listing so a
/// temporary Electrum outage does not leave addresses blank until process restart.
fn electrum_network(config: &ChainBackendConfig, socks_port: Option<u16>) -> Option<Network> {
    if config.kind != ChainBackendKind::Electrum {
        return None;
    }
    let route = fingerprint(config, socks_port);
    if let Ok(cache) = ELECTRUM_NETWORK.lock() {
        if let Some((cached_route, network)) = cache.as_ref() {
            if cached_route == &route {
                return *network;
            }
        }
    }
    let network = match with_chain(config, "", socks_port, |chain| {
        chain.get_blockchain_info().map_err(AppError::from)
    }) {
        Ok(info) => Some(info.chain),
        Err(e) => {
            log::warn!("could not read network from Electrum; UTXO addresses stay blank: {e:?}");
            None
        }
    };
    if network.is_some() {
        if let Ok(mut cache) = ELECTRUM_NETWORK.lock() {
            *cache = Some((route, network));
        }
    }
    network
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tor_fee_estimates_fail_closed_without_a_proxy() {
        let mut config = ChainBackendConfig::default();
        config.electrum.use_tor = true;
        let error = estimate_fees_uncached(config).await.unwrap_err();
        assert!(matches!(error.code, ErrorCode::TorUnreachable));
    }

    #[test]
    fn core_fee_and_chain_info_requests_time_out() {
        let checks: Vec<_> = [false, true]
            .into_iter()
            .map(|fee_request| {
                std::thread::spawn(move || {
                    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                    let port = listener.local_addr().unwrap().port();
                    let (release, released) = std::sync::mpsc::channel();
                    let server = std::thread::spawn(move || {
                        listener.set_nonblocking(true).unwrap();
                        let mut streams = Vec::new();
                        let deadline = std::time::Instant::now() + Duration::from_secs(60);
                        while std::time::Instant::now() < deadline {
                            if released.try_recv().is_ok() {
                                break;
                            }
                            if let Ok((stream, _)) = listener.accept() {
                                streams.push(stream);
                            }
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    });
                    let config = node("127.0.0.1", port, "test", "test");
                    let client = bounded_core_client(config.node.as_ref().unwrap()).unwrap();
                    let start = std::time::Instant::now();
                    let failed = if fee_request {
                        client
                            .estimate_smart_fee(FeePriority::Medium as u16, None)
                            .is_err()
                    } else {
                        client.get_blockchain_info().is_err()
                    };
                    let elapsed = start.elapsed();
                    let _ = release.send(());
                    server.join().unwrap();
                    assert!(failed);
                    // simple_http retries a failed header read once on a fresh socket.
                    assert!(
                        elapsed < Duration::from_secs(2 * PROBE_TIMEOUT_SECS as u64 + 5),
                        "{elapsed:?}"
                    );
                })
            })
            .collect();
        for check in checks {
            check.join().unwrap();
        }
    }

    /// A reload asks this to decide whether to send the session back to the gate; a router
    /// session has no wallet to prove it otherwise.
    #[test]
    fn the_gate_is_remembered_until_the_session_ends() {
        let session = format!("gate-test-{}", std::process::id());
        assert!(!passed_gate(&session));
        load(&session);
        assert!(
            !passed_gate(&session),
            "reading the seeded default is not passing the gate"
        );
        set_chain_backend(&session, ChainBackendConfig::default()).unwrap();
        assert!(passed_gate(&session));
        forget_session(&session);
        assert!(!passed_gate(&session));
    }

    fn node(host: &str, port: u16, username: &str, password: &str) -> ChainBackendConfig {
        ChainBackendConfig {
            kind: ChainBackendKind::CoreRpc,
            electrum: ChainBackendConfig::default().electrum,
            node: Some(NodeBackendDto {
                host: host.into(),
                port,
                username: username.into(),
                password: password.into(),
                zmq_port: 28332,
            }),
        }
    }

    /// A node elsewhere on the network — an Umbrel's, a LAN box — is the operator's to choose.
    /// What must not happen is a saved password following a host it was not given for, which
    /// `a_saved_password_never_follows_a_changed_destination` covers.
    #[test]
    fn any_node_host_is_accepted() {
        assert!(validate(&node("127.0.0.1", 8332, "u", "p")).is_ok());
        assert!(validate(&node("10.21.21.8", 8332, "umbrel", "p")).is_ok());
        assert!(validate(&node("node.local", 8332, "u", "p")).is_ok());
        assert!(validate(&node(" ", 8332, "u", "p")).is_err());
    }

    /// The blank password means "reuse what I already gave you", and that answer is only
    /// correct for the node it was given for.
    #[test]
    fn a_saved_password_never_follows_a_changed_destination() {
        store("t", node("127.0.0.1", 8332, "alice", "s3cret-node-pw"));

        let same = merge_preserved_password("t", node("127.0.0.1", 8332, "alice", ""));
        assert_eq!(same.node.unwrap().password, "s3cret-node-pw");

        for changed in [
            node("attacker.example", 8332, "alice", ""),
            node("127.0.0.1", 9999, "alice", ""),
            node("127.0.0.1", 8332, "mallory", ""),
        ] {
            assert_eq!(
                merge_preserved_password("t", changed)
                    .node
                    .unwrap()
                    .password,
                "",
                "the saved credential must not follow a different node"
            );
        }
    }

    #[test]
    fn a_saved_password_never_crosses_sessions() {
        store("a", node("127.0.0.1", 8332, "alice", "s3cret-node-pw"));
        let other = merge_preserved_password("b", node("127.0.0.1", 8332, "alice", ""));
        assert_eq!(other.node.unwrap().password, "");
    }

    #[test]
    fn onion_url_forces_tor_even_when_unchecked() {
        let dto = ElectrumBackendDto {
            url: "tcp://abcdef.onion:50001".to_string(),
            use_tor: false,
        };
        assert!(electrum_config(&dto, Some(9050)).socks5.is_some());
    }

    #[test]
    fn clearnet_url_is_direct_by_default() {
        let dto = ElectrumBackendDto {
            url: ChainBackendConfig::default().electrum.url,
            use_tor: false,
        };
        assert!(electrum_config(&dto, Some(9050)).socks5.is_none());
    }

    /// Only the proxied route is left to the crate, which paces itself at ten seconds there.
    /// A direct one would otherwise ping once a second, per watcher connection, forever.
    #[test]
    fn a_direct_route_paces_its_own_notification_ping() {
        let direct = ElectrumBackendDto {
            url: ChainBackendConfig::default().electrum.url,
            use_tor: false,
        };
        assert_eq!(
            electrum_config(&direct, Some(9050)).poll_interval_secs,
            Some(DIRECT_PING_SECS)
        );
        let onion = ElectrumBackendDto {
            url: "tcp://abcdef.onion:50001".to_string(),
            use_tor: false,
        };
        assert_eq!(electrum_config(&onion, Some(9050)).poll_interval_secs, None);
    }

    /// Electrum has no server-side wallet, so every wallet on a route shares one held
    /// connection; a Core client is bound to one named watch-only wallet, so it cannot.
    #[test]
    fn a_held_connection_is_shared_per_route_but_not_across_core_wallets() {
        let route = ChainBackendConfig::default();
        assert_eq!(
            pool_key(&route, "alice", None),
            pool_key(&route, "bob", None)
        );
        let core = node("127.0.0.1", 8332, "u", "p");
        assert_ne!(pool_key(&core, "alice", None), pool_key(&core, "bob", None));
    }

    /// A node whose password changed is a different route to every cache here. Otherwise a
    /// connection authenticated with the old credentials, and a verdict taken under them, would
    /// both answer for the new ones.
    #[test]
    fn changed_node_credentials_are_a_different_route() {
        let before = node("127.0.0.1", 8332, "alice", "old-password");
        let after = node("127.0.0.1", 8332, "alice", "new-password");
        assert_ne!(
            pool_key(&before, "w", None),
            pool_key(&after, "w", None),
            "the held connection must not be reused across a credential change"
        );
        assert_ne!(
            route_identity(&before, None),
            route_identity(&after, None),
            "nor may the cached verdict"
        );
        let renamed = node("127.0.0.1", 8332, "bob", "old-password");
        assert_ne!(route_identity(&before, None), route_identity(&renamed, None));
        // Electrum has no credentials of its own, so its identity is the route alone.
        let electrum = ChainBackendConfig::default();
        assert_eq!(route_identity(&electrum, None), fingerprint(&electrum, None));
    }

    /// Two routes alternating must not evict each other on every switch — that was the whole
    /// point of holding one — and the set must still be bounded.
    #[test]
    fn the_held_set_keeps_several_routes_and_drops_the_oldest() {
        // A Core client connects on first use, so one can be built here without a node behind
        // it. That makes this the real `hold`, not a copy of its arithmetic.
        let chain = || {
            AnyBlockchain::from_config(&BackendConfig::CoreRpc(CoreRpcConfig {
                url: "127.0.0.1:8332".into(),
                auth: Auth::UserPass("u".into(), "p".into()),
                wallet_name: "w".into(),
                zmq_addr: "tcp://127.0.0.1:28332".into(),
            }))
            .expect("a Core client is built without connecting")
        };
        let keys = |held: &Vec<(String, AnyBlockchain)>| -> Vec<String> {
            held.iter().map(|(k, _)| k.clone()).collect()
        };

        let mut held = Vec::new();
        for key in ["a", "b", "a", "b"] {
            hold(&mut held, key.to_string(), chain());
        }
        assert_eq!(
            keys(&held),
            ["b", "a"],
            "both routes stay held, once each, newest first"
        );

        held.clear();
        for key in ["one", "two", "three", "four", "five"] {
            hold(&mut held, key.to_string(), chain());
        }
        assert_eq!(held.len(), MAX_HELD, "bounded");
        assert_eq!(keys(&held)[0], "five", "most recently used first");
        assert!(
            !keys(&held).iter().any(|k| k == "one"),
            "the least recently used is the one dropped"
        );

        // Two callers on one route can each open a connection while the set is unlocked, and
        // keeping both would hold that route twice over at another route's expense.
        held.clear();
        hold(&mut held, "same".to_string(), chain());
        hold(&mut held, "same".to_string(), chain());
        assert_eq!(held.len(), 1, "one entry per route");
    }

    /// The set's lock must not span the handshake or the read: one route's server going quiet
    /// would otherwise stall every other route's reads for the same timeout. Under the earlier
    /// code this closure could not have run at all — the lock was held around it.
    #[test]
    fn a_read_runs_with_the_set_unlocked() {
        let core = node("127.0.0.1", 8332, "u", "p");
        let checked = std::cell::Cell::new(false);
        let outcome = with_chain(&core, "w", None, |_| {
            // Retried briefly: another test may hold the set for its own put, and this is about
            // `with_chain` not holding it, not about momentary contention.
            for _ in 0..50 {
                if SIDE_CHAIN.try_lock().is_ok() {
                    checked.set(true);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(())
        });
        assert!(outcome.is_ok(), "a lazy Core client needs no server to be built");
        assert!(checked.get(), "the set stayed locked for the whole read");
    }

    /// The saved route's verdict is shared by everything that asks at once — the shell on mount,
    /// a router preflight, the check before a sync. A probe of a caller-supplied config is never
    /// answered from it: the settings page asks because the user just changed something, and a
    /// stale yes would describe the server they replaced.
    #[test]
    fn a_fresh_verdict_is_shared_and_a_stale_one_is_not() {
        let status = BackendStatus {
            reachable: true,
            failure: None,
            error: None,
            chain: Some("signet".into()),
            blocks: Some(1),
            synced: true,
            subversion: None,
            verification_progress: Some(1.0),
            signet_challenge: None,
        };
        *PROBE_CACHE.lock().unwrap() =
            Some(("route-a".into(), std::time::Instant::now(), status.clone()));
        assert!(cached_probe("route-a").is_some_and(|s| s.reachable));
        assert!(cached_probe("route-b").is_none(), "another route is another answer");

        *PROBE_CACHE.lock().unwrap() = Some((
            "route-a".into(),
            std::time::Instant::now() - PROBE_CACHE_TTL - Duration::from_secs(1),
            status,
        ));
        assert!(cached_probe("route-a").is_none(), "past its window");
        *PROBE_CACHE.lock().unwrap() = None;
    }

    /// A connection opened inside the call and refused is the server being down: one attempt,
    /// and nothing left in the pool for the next caller to inherit. (The retry path — a reused
    /// socket an idle server dropped — needs a live server to exercise, and is covered by the
    /// reachability behaviour of `probe` in practice.)
    #[test]
    fn an_unreachable_route_fails_without_poisoning_the_pool() {
        let dead = ChainBackendConfig {
            kind: ChainBackendKind::Electrum,
            electrum: ElectrumBackendDto {
                // Port 1: refused immediately, so this never reaches the network.
                url: "tcp://127.0.0.1:1".to_string(),
                use_tor: false,
            },
            node: None,
        };
        let asked = std::cell::Cell::new(0);
        let outcome = with_chain(&dead, "", None, |_| {
            asked.set(asked.get() + 1);
            Ok(())
        });
        assert!(outcome.is_err(), "a refused connection is not a usable one");
        assert_eq!(asked.get(), 0, "the read never ran, so it must not be retried");
        // The set is process-wide and these tests run in parallel, so this is about this
        // route's entry, not about the set being empty.
        let key = pool_key(&dead, "", None);
        assert!(
            !SIDE_CHAIN
                .lock()
                .unwrap()
                .iter()
                .any(|(route, _)| route == &key),
            "a failed open must leave nothing held for that route"
        );
    }

    #[test]
    fn core_rpc_cannot_be_selected_without_a_node() {
        let config = ChainBackendConfig {
            kind: ChainBackendKind::CoreRpc,
            node: None,
            ..Default::default()
        };
        assert!(validate(&config).is_err());
    }
}
