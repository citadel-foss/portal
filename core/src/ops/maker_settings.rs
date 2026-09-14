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
use openswap::utill::get_maker_dir;

use crate::error::{AppError, ErrorCode};
use crate::state::AppState;
use crate::types::{MakerPortCheckDto, MakerSettingsDto, SuggestedMakerPortsDto};

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
    #[serde(default = "default_min_swap_amount")]
    min_swap_amount: u64,
    #[serde(default = "default_fidelity_amount")]
    fidelity_amount: u64,
    #[serde(default = "default_fidelity_timelock")]
    fidelity_timelock: u32,
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
fn default_min_swap_amount() -> u64 {
    10_000
}
fn default_fidelity_amount() -> u64 {
    10_000
}
fn default_fidelity_timelock() -> u32 {
    15_000
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
    Ok(get_maker_dir()?.join("makers.json"))
}

fn maker_data_dir(settings: &MakerSettingsDto) -> Result<PathBuf, AppError> {
    if let Some(data_dir) = settings.data_dir.as_deref() {
        return Ok(PathBuf::from(data_dir));
    }
    let legacy = get_maker_dir()?;
    Ok(legacy
        .parent()
        .map(|base| base.join(&settings.router_id))
        .unwrap_or_else(|| legacy.join(&settings.router_id)))
}

fn apply_runtime_config(settings: &mut MakerSettingsDto) -> Result<(), AppError> {
    let config_path = maker_data_dir(settings)?.join("config.toml");
    if !config_path.exists() {
        return Ok(());
    }
    let config = MakerServerConfig::new(Some(&config_path)).map_err(AppError::from)?;
    settings.network_port = config.network_port;
    settings.rpc_port = config.rpc_port;
    settings.socks_port = config.socks_port;
    settings.control_port = config.control_port;
    settings.min_swap_amount = config.min_swap_amount;
    settings.fidelity_amount = config.fidelity_amount;
    settings.fidelity_timelock = config.fidelity_timelock;
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
    let mut config = if config_path.exists() {
        MakerServerConfig::new(Some(&config_path)).map_err(AppError::from)?
    } else {
        MakerServerConfig::default()
    };
    config.network_port = settings.network_port;
    config.rpc_port = settings.rpc_port;
    config.socks_port = settings.socks_port;
    config.control_port = settings.control_port;
    config.min_swap_amount = settings.min_swap_amount;
    config.fidelity_amount = settings.fidelity_amount;
    config.fidelity_timelock = settings.fidelity_timelock;
    config.required_confirms = settings.required_confirms;
    config.base_fee = settings.base_fee;
    config.amount_relative_fee_pct = settings.amount_relative_fee_pct;
    config.time_relative_fee_pct = settings.time_relative_fee_pct;
    config.write_to_file(&config_path)?;
    Ok(())
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
                wallet_name,
                network_port: settings.network_port,
                rpc_port: settings.rpc_port,
                socks_port: settings.socks_port,
                control_port: settings.control_port,
                min_swap_amount: settings.min_swap_amount,
                fidelity_amount: settings.fidelity_amount,
                fidelity_timelock: settings.fidelity_timelock,
                required_confirms: settings.required_confirms,
                base_fee: settings.base_fee,
                amount_relative_fee_pct: settings.amount_relative_fee_pct,
                time_relative_fee_pct: settings.time_relative_fee_pct,
                data_dir: settings.data_directory,
            };
            (router_id, dto)
        })
        .collect())
}

pub(crate) fn load_all() -> Result<HashMap<String, MakerSettingsDto>, AppError> {
    let _guard = SETTINGS_IO.lock()?;
    let mut stored = load_file(&settings_path()?)?;
    for settings in stored.makers.values_mut() {
        apply_runtime_config(settings)?;
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

pub fn list_makers() -> Result<Vec<MakerSettingsDto>, AppError> {
    let mut makers: Vec<_> = load_all()?.into_values().collect();
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

pub fn clear_maker_settings(
    state: &Arc<AppState>,
    router_id: String,
) -> Result<(), AppError> {
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
            network_port: 6102,
            rpc_port: 6103,
            socks_port: 9050,
            control_port: 9051,
            min_swap_amount: 10_000,
            fidelity_amount: 10_000,
            fidelity_timelock: 15_000,
            required_confirms: 1,
            base_fee: 500,
            amount_relative_fee_pct: 0.0025,
            time_relative_fee_pct: 0.0001,
            data_dir: Some("/tmp/maker-one".to_string()),
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
        saved.min_swap_amount = 42_000;
        saved.base_fee = 777;
        write_runtime_config(&saved).unwrap();

        let mut registry_copy = settings();
        registry_copy.data_dir = saved.data_dir.clone();
        apply_runtime_config(&mut registry_copy).unwrap();
        assert_eq!(registry_copy.network_port, 6202);
        assert_eq!(registry_copy.rpc_port, 6203);
        assert_eq!(registry_copy.min_swap_amount, 42_000);
        assert_eq!(registry_copy.base_fee, 777);

        std::fs::remove_dir_all(data_dir).unwrap();
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
        assert_eq!(zoro.min_swap_amount, 20_000);
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
