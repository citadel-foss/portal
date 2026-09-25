fn main() {
    // One lockfile at the workspace root now, not beside this package.
    let lock_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri always has a workspace root above it")
        .join("Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock_path.display());
    let lock = std::fs::read_to_string(&lock_path).expect("Cargo.lock is required");
    let openswap_block = lock
        .split("[[package]]")
        .find(|block| {
            block
                .lines()
                .any(|line| line.trim() == "name = \"openswap\"")
        })
        .expect("openswap must be pinned in Cargo.lock");
    let revision = openswap_block
        .lines()
        .find_map(|line| line.trim().strip_prefix("source = \"")?.strip_suffix('"'))
        .and_then(|source| source.rsplit_once('#').map(|(_, revision)| revision))
        .filter(|revision| revision.len() == 40)
        .expect("openswap Cargo.lock source must end in a full git revision");
    println!("cargo:rustc-env=PORTAL_OPENSWAP_REV={revision}");
    // Every command in `lib.rs`'s `generate_handler!`, in the same order. A command absent
    // here gets no generated permission, so `capabilities/default.json` cannot grant it and
    // every call is rejected at the IPC boundary — see the sync test in `lib.rs`.
    const COMMANDS: &[&str] = &[
        "check_tor",
        "restart_tor_bootstrap",
        "get_chain_backend",
        "get_electrum_presets",
        "set_chain_backend",
        "check_backend",
        "list_wallets",
        "init_taker",
        "shutdown_taker",
        "get_paths",
        "get_session_state",
        "get_wallet_info",
        "choose_restore_backup",
        "restore_wallet",
        "backup_wallet",
        "get_balances",
        "validate_address",
        "get_new_address",
        "verify_last_address",
        "get_transactions",
        "list_utxos",
        "send_to_address",
        "sync_wallet",
        "estimate_fees",
        "get_btc_price",
        "get_offers",
        "sync_offerbook",
        "poll_maker",
        "remove_maker",
        "prepare_swap",
        "estimate_swap_funding",
        "start_swap",
        "get_swap_progress",
        "get_swap_tracker",
        "get_swap_preparation",
        "recover_swap",
        "get_recovery_status",
        "list_recoveries",
        "list_swap_reports",
        "get_swap_report",
        "get_incoming_swap_utxo",
        "verify_deniability",
        "get_logs",
        "init_maker",
        "update_maker_settings",
        "start_maker",
        "stop_maker",
        "get_maker_status",
        "get_maker_info",
        "list_maker_swap_reports",
        "get_maker_swap_report",
        "verify_maker_deniability",
        "get_maker_balances",
        "list_maker_utxos",
        "get_maker_new_address",
        "get_maker_transactions",
        "send_maker_to_address",
        "sync_maker_wallet",
        "list_maker_fidelity_bonds",
        "list_makers",
        "get_saved_maker_settings",
        "list_dashboard_imports",
        "import_dashboard_makers",
        "clear_maker_settings",
        "get_router_defaults",
        "get_suggested_maker_ports",
        "check_maker_ports",
        "get_maker_logs",
        "quit_app",
    ];
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    tauri_build::try_build(attributes).expect("failed to build Tauri command manifest")
}
