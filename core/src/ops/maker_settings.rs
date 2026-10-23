//! Persistent maker registrations. Runtime objects are deliberately absent:
//! makers are reconstructed from these settings and their wallet files only
//! when `start_maker` is explicitly called.

use std::sync::Arc;

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use openswap::maker::MakerServerConfig;
use openswap::utill::MIN_RELAY_FEE_RATE;

use crate::error::{AppError, ErrorCode};
use crate::state::AppState;
use crate::types::{MakerPhase, MakerPortCheckDto, MakerSettingsDto, SuggestedMakerPortsDto};

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredMakers {
    #[serde(default)]
    makers: HashMap<String, MakerSettingsDto>,
}

/// Compatibility shape for Maker Dashboard's registration store. Credentials
/// and backend fields are intentionally not represented, so they can never be
/// copied into this app's unencrypted registry.
#[derive(Debug, Default, serde::Deserialize)]
struct DashboardStoredMakers {
    #[serde(default)]
    makers: HashMap<String, DashboardMakerSettings>,
}

#[derive(Debug, serde::Deserialize)]
struct DashboardMakerSettings {
    data_directory: Option<String>,
    wallet_name: Option<String>,
    #[serde(default = "default_network_port")]
    network_port: u16,
    #[serde(default = "default_rpc_port")]
    rpc_port: u16,
    #[serde(default = "default_socks_port")]
    socks_port: u16,
    #[serde(default = "default_control_port")]
    control_port: u16,
    #[serde(default = "default_fidelity_amount")]
    fidelity_amount: u64,
    #[serde(default = "default_fidelity_timelock")]
    fidelity_timelock: u32,
    #[serde(default = "default_fidelity_feerate")]
    fidelity_feerate: f64,
    #[serde(default = "default_required_confirms")]
    required_confirms: u32,
    #[serde(default = "default_base_fee")]
    base_fee: u64,
    #[serde(default = "default_amount_relative_fee_pct")]
    amount_relative_fee_pct: f64,
    #[serde(default = "default_time_relative_fee_pct")]
    time_relative_fee_pct: f64,
}

fn default_network_port() -> u16 {
    6102
}
fn default_rpc_port() -> u16 {
    6103
}
fn default_socks_port() -> u16 {
    9050
}
fn default_control_port() -> u16 {
    9051
}
fn default_fidelity_amount() -> u64 {
    10_000
}
fn default_fidelity_timelock() -> u32 {
    15_000
}
pub(crate) fn default_fidelity_feerate() -> f64 {
    MIN_RELAY_FEE_RATE
}
fn default_required_confirms() -> u32 {
    1
}
fn default_base_fee() -> u64 {
    1_000
}
fn default_amount_relative_fee_pct() -> f64 {
    0.025
}
fn default_time_relative_fee_pct() -> f64 {
    0.001
}

static SETTINGS_IO: Mutex<()> = Mutex::new(());

fn settings_path() -> Result<PathBuf, AppError> {
    crate::storage::makers_registry()
}

pub(crate) fn maker_data_dir(settings: &MakerSettingsDto) -> Result<PathBuf, AppError> {
    match settings.data_dir.as_deref() {
        Some(data_dir) => Ok(PathBuf::from(data_dir)),
        None => crate::storage::maker_data_dir(&settings.router_id),
    }
}

/// Parsed configs keyed by file, reused while the file's bytes are unchanged. Every registry read
/// goes through here and the router pages poll the registry every few seconds; the crate logs a
/// line per parse. Keyed on content, not mtime: an edit that keeps the mtime would otherwise be
/// served stale and then written back over by `write_runtime_config`.
type RuntimeConfigCache = HashMap<PathBuf, (Vec<u8>, MakerServerConfig)>;
static RUNTIME_CONFIGS: Mutex<Option<RuntimeConfigCache>> = Mutex::new(None);

fn read_runtime_config(path: &Path) -> Result<MakerServerConfig, AppError> {
    let bytes = std::fs::read(path).map_err(AppError::internal)?;
    let mut cache = RUNTIME_CONFIGS.lock()?;
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some((cached, config)) = cache.get(path) {
        if *cached == bytes {
            return Ok(config.clone());
        }
    }
    let config = MakerServerConfig::new(Some(path)).map_err(AppError::from)?;
    cache.insert(path.to_path_buf(), (bytes, config.clone()));
    Ok(config)
}

fn apply_runtime_config(settings: &mut MakerSettingsDto) -> Result<(), AppError> {
    // A registration from before names existed, with no config.toml to supply one, would
    // otherwise fail the name check and never start again.
    if settings.name.is_empty() {
        settings.name = settings
            .router_id
            .chars()
            .take(crate::ops::maker::MAX_ROUTER_NAME_LEN)
            .collect();
    }
    let config_path = maker_data_dir(settings)?.join("config.toml");
    if !config_path.exists() {
        return Ok(());
    }
    let config = read_runtime_config(&config_path)?;
    if !config.name.is_empty() {
        settings.name = config.name;
    }
    settings.network_port = config.network_port;
    settings.rpc_port = config.rpc_port;
    settings.socks_port = config.socks_port;
    settings.control_port = config.control_port;
    settings.fidelity_amount = config.fidelity_amount;
    settings.fidelity_timelock = config.fidelity_timelock;
    settings.fidelity_feerate = config.fidelity_feerate;
    settings.required_confirms = config.required_confirms;
    settings.base_fee = config.base_fee;
    settings.amount_relative_fee_pct = config.amount_relative_fee_pct;
    settings.time_relative_fee_pct = config.time_relative_fee_pct;
    Ok(())
}

/// Write the editable runtime settings to the file the standalone maker daemon also uses.
/// `makers.json` remains the multi-maker registry for identity and wallet location only.
pub(crate) fn write_runtime_config(settings: &MakerSettingsDto) -> Result<(), AppError> {
    let config_path = maker_data_dir(settings)?.join("config.toml");
    // A file the crate now rejects (a bond under its minimum, say) is rewritten from scratch:
    // refusing would leave Settings unable to fix the very value that broke it.
    let mut config = if config_path.exists() {
        read_runtime_config(&config_path).unwrap_or_else(|error| {
            log::warn!(
                "rewriting {} the crate could not read: {}",
                config_path.display(),
                error.message
            );
            MakerServerConfig::default()
        })
    } else {
        MakerServerConfig::default()
    };
    apply_settings(&mut config, settings);
    config.write_to_file(&config_path)?;
    Ok(())
}

fn apply_settings(config: &mut MakerServerConfig, settings: &MakerSettingsDto) {
    config.name = settings.name.clone();
    config.network_port = settings.network_port;
    config.rpc_port = settings.rpc_port;
    config.socks_port = settings.socks_port;
    config.control_port = settings.control_port;
    config.fidelity_amount = settings.fidelity_amount;
    config.fidelity_timelock = settings.fidelity_timelock;
    config.fidelity_feerate = settings.fidelity_feerate;
    config.required_confirms = settings.required_confirms;
    config.base_fee = settings.base_fee;
    config.amount_relative_fee_pct = settings.amount_relative_fee_pct;
    config.time_relative_fee_pct = settings.time_relative_fee_pct;
}

/// What the crate's own config check says about these values, before anything is created. It
/// only runs while reading a `config.toml` (`MakerServerConfig::new`), and its limits, such as
/// the bond minimum, are not public; so the values go through a throwaway file. `None` means
/// the crate accepts them.
pub fn check_router_config(settings: MakerSettingsDto) -> Result<Option<String>, AppError> {
    let mut config = MakerServerConfig::default();
    apply_settings(&mut config, &settings);
    crate_verdict(&config)
}

fn crate_verdict(config: &MakerServerConfig) -> Result<Option<String>, AppError> {
    let path =
        std::env::temp_dir().join(format!("portal-router-check-{}.toml", uuid::Uuid::new_v4()));
    config.write_to_file(&path)?;
    let verdict = match MakerServerConfig::new(Some(&path)) {
        Ok(_) => Ok(None),
        Err(openswap::wallet::WalletError::Fidelity(e)) => Ok(Some(e.to_string())),
        Err(openswap::wallet::WalletError::General(message)) => Ok(Some(message)),
        Err(error) => Err(error.into()),
    };
    let _ = std::fs::remove_file(&path);
    verdict
}

/// The smallest bond amount the crate's config check accepts, so the form can say it up front.
/// The crate keeps that constant private and its error type unreachable from here, so the value
/// is found by asking the check itself rather than restated. Fixed for a build, hence cached.
pub(crate) fn min_fidelity_amount() -> Option<u64> {
    static MIN: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    if let Some(min) = MIN.get() {
        return Some(*min);
    }
    let accepts = |amount: u64| {
        let config = MakerServerConfig {
            fidelity_amount: amount,
            ..Default::default()
        };
        crate_verdict(&config).map(|verdict| verdict.is_none())
    };
    // A failed filesystem operation leaves the cache empty so the next request can retry.
    let (mut low, mut high) = (0, MakerServerConfig::default().fidelity_amount);
    if !accepts(high).ok()? {
        return None;
    }
    while high - low > 1 {
        let mid = low + (high - low) / 2;
        if accepts(mid).ok()? {
            high = mid;
        } else {
            low = mid;
        }
    }
    let _ = MIN.set(high);
    Some(high)
}

fn dashboard_settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("maker-dashboard").join("makers.json"))
}

fn load_file(path: &Path) -> Result<StoredMakers, AppError> {
    if !path.exists() {
        return Ok(StoredMakers::default());
    }
    let bytes = std::fs::read(path)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| AppError::internal(format!("failed to parse {}: {e}", path.display())))
}

fn save_file(path: &Path, stored: &StoredMakers) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let payload = serde_json::to_vec_pretty(stored).map_err(AppError::internal)?;
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(&payload)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

fn load_dashboard_registrations(
    path: &Path,
) -> Result<HashMap<String, MakerSettingsDto>, AppError> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let bytes = std::fs::read(path)?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::internal(format!("failed to parse {}: {e}", path.display())))?;
    // Password-encrypted dashboard stores require an explicit migration flow;
    // silently treating their envelope as maker settings would lose config.
    if value.get("v").is_some() && value.get("data").is_some() {
        return Ok(HashMap::new());
    }
    let stored: DashboardStoredMakers = serde_json::from_value(value)
        .map_err(|e| AppError::internal(format!("failed to parse {}: {e}", path.display())))?;
    Ok(stored
        .makers
        .into_iter()
        .map(|(router_id, settings)| {
            let wallet_name = settings.wallet_name.unwrap_or_else(|| router_id.clone());
            let dto = MakerSettingsDto {
                router_id: router_id.clone(),
                // Ids have no length cap; a name longer than wallets accept would stop it starting.
                name: router_id
                    .chars()
                    .take(crate::ops::maker::MAX_ROUTER_NAME_LEN)
                    .collect(),
                wallet_name,
                network_port: settings.network_port,
                rpc_port: settings.rpc_port,
                socks_port: settings.socks_port,
                control_port: settings.control_port,
                fidelity_amount: settings.fidelity_amount,
                fidelity_timelock: settings.fidelity_timelock,
                fidelity_feerate: settings.fidelity_feerate,
                required_confirms: settings.required_confirms,
                base_fee: settings.base_fee,
                amount_relative_fee_pct: settings.amount_relative_fee_pct,
                time_relative_fee_pct: settings.time_relative_fee_pct,
                data_dir: settings.data_directory,
                network: None,
            };
            (router_id, dto)
        })
        .collect())
}

pub(crate) fn load_all() -> Result<HashMap<String, MakerSettingsDto>, AppError> {
    let _guard = SETTINGS_IO.lock()?;
    let mut stored = load_file(&settings_path()?)?;
    // One router's file failing the crate's checks must not take every router's settings with
    // it; that router keeps its registered values, and its start reports the crate's error.
    for settings in stored.makers.values_mut() {
        if let Err(error) = apply_runtime_config(settings) {
            log::warn!(
                "router {}: config.toml not applied: {}",
                settings.router_id,
                error.message
            );
        }
    }
    Ok(stored.makers)
}

pub(crate) fn load(router_id: &str) -> Result<Option<MakerSettingsDto>, AppError> {
    Ok(load_all()?.remove(router_id))
}

pub(crate) fn save(settings: &MakerSettingsDto) -> Result<(), AppError> {
    let _guard = SETTINGS_IO.lock()?;
    let path = settings_path()?;
    let mut stored = load_file(&path)?;
    stored
        .makers
        .insert(settings.router_id.clone(), settings.clone());
    save_file(&path, &stored)
}

pub fn list_makers(state: &Arc<AppState>) -> Result<Vec<MakerSettingsDto>, AppError> {
    let mut all = load_all()?;
    // A restore is registered only after it finishes. Include its process-local entry so leaving
    // the form does not make the ongoing restore disappear from the dashboard.
    for (router_id, entry) in state.makers.lock()?.iter() {
        if matches!(entry.phase, MakerPhase::Restoring) {
            all.entry(router_id.clone())
                .or_insert_with(|| entry.settings.clone());
        }
    }
    let mut makers = all
        .into_values()
        .map(|mut settings| {
            let dir = match &settings.data_dir {
                Some(dir) => std::path::PathBuf::from(dir),
                None => crate::storage::maker_data_dir(&settings.router_id)?,
            };
            settings.network = crate::storage::wallet_network(&dir);
            Ok(settings)
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    makers.sort_by(|a, b| a.router_id.cmp(&b.router_id));
    Ok(makers)
}

pub fn get_saved_maker_settings(router_id: String) -> Result<Option<MakerSettingsDto>, AppError> {
    load(&router_id)
}

/// Maker Dashboard registrations this app has no entry for yet. Offering them for an explicit
/// import rather than adopting them on load is what keeps a deleted maker deleted: the registry
/// is the only record of what the user curated, and it cannot vouch for ids it no longer holds.
pub fn list_dashboard_imports() -> Result<Vec<MakerSettingsDto>, AppError> {
    let Some(dashboard_path) = dashboard_settings_path() else {
        return Ok(Vec::new());
    };
    let registered = load_file(&settings_path()?)?.makers;
    let mut available: Vec<_> = load_dashboard_registrations(&dashboard_path)?
        .into_iter()
        .filter(|(router_id, _)| !registered.contains_key(router_id))
        .map(|(_, dto)| dto)
        .collect();
    available.sort_by(|a, b| a.router_id.cmp(&b.router_id));
    Ok(available)
}

pub fn import_dashboard_makers(router_ids: Vec<String>) -> Result<Vec<MakerSettingsDto>, AppError> {
    let Some(dashboard_path) = dashboard_settings_path() else {
        return Ok(Vec::new());
    };
    let mut discovered = load_dashboard_registrations(&dashboard_path)?;
    let imported: Vec<_> = router_ids
        .iter()
        .filter_map(|router_id| discovered.remove(router_id))
        .collect();
    for settings in &imported {
        save(settings)?;
    }
    Ok(imported)
}

pub fn clear_maker_settings(state: &Arc<AppState>, router_id: String) -> Result<(), AppError> {
    if let Some(entry) = state.makers.lock()?.get(&router_id) {
        if !matches!(
            entry.phase,
            crate::types::MakerPhase::Stopped | crate::types::MakerPhase::Failed { .. }
        ) {
            return Err(AppError::maker_busy());
        }
    }
    let _guard = SETTINGS_IO.lock()?;
    let path = settings_path()?;
    let mut stored = load_file(&path)?;
    stored.makers.remove(&router_id);
    save_file(&path, &stored)?;
    state.makers.lock()?.remove(&router_id);
    crate::logging::unregister_maker(&router_id);
    Ok(())
}

fn is_port_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn find_available_port(start: u16, reserved: &[u16]) -> Option<u16> {
    (start..=u16::MAX).find(|port| !reserved.contains(port) && is_port_free(*port))
}

pub fn get_suggested_maker_ports() -> Result<SuggestedMakerPortsDto, AppError> {
    const DEFAULT_NETWORK_PORT: u16 = 6102;
    const DEFAULT_RPC_PORT: u16 = 6103;

    let registered = load_all()?;
    let mut reserved = tor_ports().to_vec();
    reserved.extend(
        registered
            .values()
            .flat_map(|m| [m.network_port, m.rpc_port]),
    );

    let network_port = find_available_port(DEFAULT_NETWORK_PORT, &reserved)
        .ok_or_else(|| AppError::new(ErrorCode::Internal, "no available network port found"))?;
    reserved.push(network_port);
    let rpc_port = find_available_port(DEFAULT_RPC_PORT, &reserved)
        .ok_or_else(|| AppError::new(ErrorCode::Internal, "no available rpc port found"))?;

    Ok(SuggestedMakerPortsDto {
        network_port,
        rpc_port,
    })
}

/// Why `port` cannot be a maker listener, or `None` if it can.
///
/// Deliberately not `setup::check_port`: that connects and reports success when something
/// is *already listening*, the inverse of what a port we intend to bind needs — wiring it
/// in here would approve exactly the ports that are taken.
/// Portal's own Tor, so a maker never suggests or accepts a port Tor already holds. Both
/// zero before Tor starts, which no maker port can collide with.
fn tor_ports() -> [u16; 2] {
    crate::tor::runtime().map_or([0, 0], |tor| [tor.socks_port, tor.control_port])
}

fn port_conflict(
    port: u16,
    socks_port: u16,
    control_port: u16,
    taken: &HashMap<u16, String>,
) -> Option<String> {
    if port == 0 {
        return Some("Not a valid port.".to_string());
    }
    if port == socks_port || port == control_port {
        return Some(format!("Port {port} is already used by Tor."));
    }
    if let Some(owner) = taken.get(&port) {
        return Some(format!("Port {port} is already used by router '{owner}'."));
    }
    if !is_port_free(port) {
        return Some(format!("Port {port} is already in use. Pick another."));
    }
    None
}

/// Validates a maker's two listener ports so the UI can warn before `init_maker` writes a
/// wallet — a failure there has to be unwound (`maker::abort_failed_creation`).
pub fn check_maker_ports(network_port: u16, rpc_port: u16) -> Result<MakerPortCheckDto, AppError> {
    let [socks_port, control_port] = tor_ports();
    let mut taken = HashMap::new();
    for (id, settings) in load_all()? {
        taken.insert(settings.network_port, id.clone());
        taken.insert(settings.rpc_port, id);
    }

    let mut result = MakerPortCheckDto {
        network_port: port_conflict(network_port, socks_port, control_port, &taken),
        rpc_port: port_conflict(rpc_port, socks_port, control_port, &taken),
    };
    // Both bind the same host, so an identical pair fails at start even though each port is
    // free on its own.
    if result.rpc_port.is_none() && network_port == rpc_port {
        result.rpc_port = Some("Network and RPC ports must differ.".to_string());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> MakerSettingsDto {
        MakerSettingsDto {
            router_id: "maker-one".to_string(),
            wallet_name: "wallet-one".to_string(),
            name: "maker-one".to_string(),
            network_port: 6102,
            rpc_port: 6103,
            socks_port: 9050,
            control_port: 9051,
            fidelity_amount: 10_000,
            fidelity_timelock: 15_000,
            fidelity_feerate: 2.0,
            required_confirms: 1,
            base_fee: 500,
            amount_relative_fee_pct: 0.0025,
            time_relative_fee_pct: 0.0001,
            data_dir: Some("/tmp/maker-one".to_string()),
            network: None,
        }
    }

    #[test]
    fn runtime_config_round_trip_is_the_editable_source_of_truth() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!(
            "portal-maker-config-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&data_dir).unwrap();

        let mut saved = settings();
        saved.data_dir = Some(data_dir.to_string_lossy().into_owned());
        saved.network_port = 6202;
        saved.rpc_port = 6203;
        saved.base_fee = 777;
        saved.fidelity_feerate = 3.5;
        saved.name = "satoshi's lounge".to_string();
        write_runtime_config(&saved).unwrap();

        let mut registry_copy = settings();
        registry_copy.data_dir = saved.data_dir.clone();
        apply_runtime_config(&mut registry_copy).unwrap();
        assert_eq!(registry_copy.network_port, 6202);
        assert_eq!(registry_copy.rpc_port, 6203);
        assert_eq!(registry_copy.base_fee, 777);
        assert_eq!(registry_copy.fidelity_feerate, 3.5);
        assert_eq!(registry_copy.name, "satoshi's lounge");

        std::fs::remove_dir_all(data_dir).unwrap();
    }

    /// The crate refuses a `config.toml` whose bond is under its minimum. Starting from it fails
    /// with that error, and Settings can still save the corrected amount over it.
    #[test]
    fn a_rejected_config_can_be_corrected_from_settings() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data_dir = std::env::temp_dir().join(format!(
            "portal-maker-small-bond-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&data_dir).unwrap();
        let config_path = data_dir.join("config.toml");

        let mut saved = settings();
        saved.data_dir = Some(data_dir.to_string_lossy().into_owned());
        saved.fidelity_amount = 1_000;
        write_runtime_config(&saved).unwrap();
        let rejected = MakerServerConfig::new(Some(&config_path));
        assert!(
            rejected.is_err(),
            "the crate refuses a bond under its minimum"
        );

        let mut registry_copy = settings();
        registry_copy.data_dir = saved.data_dir.clone();
        assert!(apply_runtime_config(&mut registry_copy).is_err());

        saved.fidelity_amount = 10_000;
        write_runtime_config(&saved).unwrap();
        let fixed = MakerServerConfig::new(Some(&config_path)).unwrap();
        assert_eq!(fixed.fidelity_amount, 10_000);

        std::fs::remove_dir_all(data_dir).unwrap();
    }

    #[test]
    fn a_registration_without_a_name_falls_back_to_its_id() {
        let mut legacy = settings();
        legacy.name = String::new();
        legacy.router_id = "r".repeat(40);
        legacy.data_dir = Some(
            std::env::temp_dir()
                .join("portal-no-such-router")
                .display()
                .to_string(),
        );
        apply_runtime_config(&mut legacy).unwrap();
        assert_eq!(
            legacy.name,
            "r".repeat(crate::ops::maker::MAX_ROUTER_NAME_LEN)
        );
    }

    #[test]
    fn stored_makers_default_when_makers_field_is_missing() {
        let decoded: StoredMakers = serde_json::from_str("{}").unwrap();
        assert!(decoded.makers.is_empty());
    }

    /// The guard fields an earlier silent dashboard import needed. A registry still carrying
    /// them has to load, since deleting them is what stops that import resurrecting makers.
    #[test]
    fn legacy_guard_fields_are_ignored() {
        let decoded: StoredMakers =
            serde_json::from_str(r#"{"makers":{},"dashboardMigrated":true,"deleted":["Zoro"]}"#)
                .unwrap();
        assert!(decoded.makers.is_empty());
    }

    /// An id the registry does not hold is offered for import, never adopted. Three makers
    /// deleted in the UI previously reappeared because an absent registry read as a pending
    /// migration, and the tombstones meant to prevent it lived in that same absent file.
    #[test]
    fn only_unregistered_ids_are_offered_for_import() {
        let registered = HashMap::from([("Zoro".to_string(), settings())]);
        let discovered = HashMap::from([
            ("Zoro".to_string(), settings()),
            ("Luffy".to_string(), settings()),
        ]);

        let offered: Vec<_> = discovered
            .into_iter()
            .filter(|(router_id, _)| !registered.contains_key(router_id))
            .map(|(router_id, _)| router_id)
            .collect();
        assert_eq!(offered, vec!["Luffy"]);
    }

    #[test]
    fn port_conflict_names_tor_and_other_makers() {
        let mut taken = HashMap::new();
        taken.insert(6102, "maker-one".to_string());

        assert!(port_conflict(9050, 9050, 9051, &taken)
            .expect("socks port must conflict")
            .contains("Tor"));
        assert!(port_conflict(9051, 9050, 9051, &taken)
            .expect("control port must conflict")
            .contains("Tor"));
        assert!(port_conflict(6102, 9050, 9051, &taken)
            .expect("registered maker port must conflict")
            .contains("maker-one"));
        assert!(port_conflict(0, 9050, 9051, &taken).is_some());
    }

    #[test]
    fn port_conflict_reports_an_occupied_port() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        // Held open for the duration of the check, so the bind probe must fail.
        assert!(port_conflict(port, 9050, 9051, &HashMap::new()).is_some());
        drop(listener);
        assert!(port_conflict(port, 9050, 9051, &HashMap::new()).is_none());
    }

    #[test]
    fn identical_network_and_rpc_ports_are_rejected() {
        let free = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = free.local_addr().unwrap().port();
        drop(free);
        let result = check_maker_ports(port, port).unwrap();
        assert!(result.network_port.is_none());
        assert!(result
            .rpc_port
            .expect("duplicate must be caught")
            .contains("differ"));
    }

    #[test]
    fn imports_plaintext_dashboard_registrations_without_credentials() {
        let value = serde_json::json!({
            "makers": {
                "Zoro": {
                    "data_directory": "/tmp/Zoro",
                    "wallet_name": "Zoro",
                    "password": "wallet-secret",
                    "rpc_password": "rpc-secret",
                    "tor_auth": "tor-secret",
                    "network_port": 6104,
                    "rpc_port": 6105,
                    "socks_port": 9050,
                    "control_port": 9051,
                    "min_swap_amount": 20_000,
                    "fidelity_amount": 30_000,
                    "fidelity_timelock": 15_000,
                    "required_confirms": 2,
                    "base_fee": 900,
                    "amount_relative_fee_pct": 0.02,
                    "time_relative_fee_pct": 0.001
                }
            }
        });
        let path = std::env::temp_dir().join(format!(
            "portal-dashboard-import-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let makers = load_dashboard_registrations(&path).unwrap();
        let _ = std::fs::remove_file(path);

        let zoro = makers.get("Zoro").unwrap();
        assert_eq!(zoro.wallet_name, "Zoro");
        assert_eq!(zoro.network_port, 6104);
        assert_eq!(zoro.fidelity_amount, 30_000);
    }

    #[test]
    fn ignores_encrypted_dashboard_store_until_explicit_migration() {
        let path = std::env::temp_dir().join(format!(
            "portal-dashboard-encrypted-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, br#"{"v":1,"data":"encrypted"}"#).unwrap();
        let makers = load_dashboard_registrations(&path).unwrap();
        let _ = std::fs::remove_file(path);
        assert!(makers.is_empty());
    }
}
