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
use openswap::wallet::{BackendConfig, Blockchain, CoreRpcConfig, Electrum, ElectrumConfig};

use crate::error::{AppError, ErrorCode};
use crate::types::{
    BackendStatus, ChainBackendConfig, ChainBackendKind, ChainBackendView, ElectrumBackendDto,
    ElectrumPresetDto, NodeBackendDto, NodeBackendViewDto,
};

/// One bounded attempt is enough for a probe. The backend's own defaults (a 120s proxied
/// read timeout plus `max_retries` reconnects) would stall the UI for minutes before verdict.
const PROBE_TIMEOUT_SECS: u8 = 15;

/// Written by versions that persisted the backend, including a node's RPC password.
const LEGACY_FILE_NAME: &str = "backend.json";

/// Each session's backend, seeded from the defaults on first read. Per session so one browser
/// passing the gate with a different backend never repoints another's.
static SESSIONS: Mutex<Option<HashMap<String, ChainBackendConfig>>> = Mutex::new(None);

/// Cached by complete endpoint/route fingerprint, never by process first-use.
/// The servers the gate offers, newest verified list. Every mainnet entry was probed against
/// the live chain before landing and agreed on the same tip; a preset that does not answer is
/// worse than no preset, so `fulcrum.sethforprivacy.com` was dropped for timing out.
///
/// Reference data, deliberately not configuration: nothing here is written to disk and the
/// choice is not remembered between launches. The signet entry is `DEFAULT_ELECTRUM_URL`
/// itself rather than a second copy of the same string.
const ELECTRUM_PRESETS: &[(&str, &str, &str)] = &[
    ("Portal signet", crate::types::DEFAULT_ELECTRUM_URL, "signet"),
    ("Blockstream", "ssl://electrum.blockstream.info:50002", "bitcoin"),
    ("Emzy", "ssl://electrum.emzy.de:50002", "bitcoin"),
    ("Bitaroo", "ssl://electrum.bitaroo.net:50002", "bitcoin"),
    ("DIY Nodes", "ssl://electrum.diynodes.com:50002", "bitcoin"),
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

fn electrum_config(dto: &ElectrumBackendDto, socks_port: Option<u16>) -> ElectrumConfig {
    let socks_port = live_socks_port(socks_port);
    ElectrumConfig {
        url: dto.url.clone(),
        socks5: electrum_needs_tor(dto)
            .then(|| socks_port.map(|port| format!("127.0.0.1:{port}")))
            .flatten(),
        timeout: None,
        poll_interval_secs: None,
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
            if !matches!(node.host.as_str(), "127.0.0.1" | "localhost" | "::1") {
                return Err(invalid(
                    "Remote Bitcoin Core RPC/ZMQ is plaintext. Use a trusted tunnel and configure its local 127.0.0.1 or ::1 endpoint.",
                ));
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
    let status = tokio::task::spawn_blocking(move || match config.kind {
        ChainBackendKind::Electrum => probe_electrum(&config.electrum, socks_port),
        ChainBackendKind::CoreRpc => config
            .node
            .as_ref()
            .map(probe_core_rpc)
            .unwrap_or_else(|| unreachable("no Bitcoin node is configured".to_string())),
    })
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
            password_configured: !node.password.is_empty(),
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
                if saved.host == node.host && saved.port == node.port && saved.username == node.username {
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
    Ok(())
}

pub async fn check_backend(
    session: &str,
    config: Option<ChainBackendConfig>,
    socks_port: Option<u16>,
) -> Result<BackendStatus, AppError> {
    let config = match config {
        Some(config) => merge_preserved_password(session, config),
        None => load(session),
    };
    // The same rules `set_chain_backend` applies. Probing reaches the network with a
    // credential attached, so it cannot be the lenient path — a caller-chosen host would
    // otherwise receive the saved RPC password, and the reachability of arbitrary addresses
    // would make this a scanner for whatever the server can see.
    validate(&config)?;
    tokio::task::spawn_blocking(move || match config.kind {
        ChainBackendKind::Electrum => probe_electrum(&config.electrum, socks_port),
        ChainBackendKind::CoreRpc => match &config.node {
            Some(node) => probe_core_rpc(node),
            None => unreachable("no Bitcoin node is configured".to_string()),
        },
    })
    .await
    .map_err(AppError::internal)
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

fn probe_electrum(dto: &ElectrumBackendDto, socks_port: Option<u16>) -> BackendStatus {
    let mut config = electrum_config(dto, socks_port);
    config.max_retries = 0;
    config.timeout = Some(PROBE_TIMEOUT_SECS);
    // Completes the Electrum handshake and checks the server's genesis hash, so this
    // rejects a reachable server on the wrong chain — a raw socket probe cannot.
    let client = match Electrum::new(&config) {
        Ok(c) => c,
        Err(e) => return unreachable(format!("{e:?}")),
    };
    match client.get_blockchain_info() {
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

fn probe_core_rpc(node: &NodeBackendDto) -> BackendStatus {
    let url = format!("http://{}:{}", node.host, node.port);
    // Built by hand rather than via `Client::new` so the probe inherits `PROBE_TIMEOUT_SECS`
    // instead of the transport's 15-minute default — this runs on the startup checklist,
    // where a filtered port would otherwise hang behind the OS connect timeout.
    let transport = match simple_http::Builder::new().url(&url).map(|b| {
        b.auth(node.username.clone(), Some(node.password.clone()))
            .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS as u64))
            .build()
    }) {
        Ok(t) => t,
        Err(e) => return unreachable(format!("{e:?}")),
    };
    let client = Client::from_jsonrpc(jsonrpc::Client::with_transport(transport));
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
    let mut electrum = electrum_config(&config.electrum, socks_port);
    electrum.max_retries = 0;
    electrum.timeout = Some(PROBE_TIMEOUT_SECS);
    let network = match Electrum::new(&electrum).and_then(|c| c.get_blockchain_info()) {
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

    /// Probing reaches the network with a credential attached, so it has to be held to the
    /// same loopback rule as adopting a backend. Without this a caller names any host and
    /// the saved RPC password is posted to it in the clear.
    #[test]
    fn probing_is_held_to_the_same_host_rule_as_adopting() {
        assert!(validate(&node("127.0.0.1", 8332, "u", "p")).is_ok());
        assert!(validate(&node("attacker.example", 80, "u", "p")).is_err());
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
                merge_preserved_password("t", changed).node.unwrap().password,
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
