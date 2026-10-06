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

    let root = lock_path
        .parent()
        .expect("Cargo.lock has a parent directory");
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|stdout| stdout.trim().to_owned())
    };
    // A source tarball has no repository; the build still has to succeed.
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    if let Some(git_dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={git_dir}/HEAD");
        println!("cargo:rerun-if-changed={git_dir}/refs/heads");
    }
    println!("cargo:rustc-env=PORTAL_COMMIT={commit}");

    // SOURCE_DATE_EPOCH keeps reproducible builds byte-identical.
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let epoch = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock is after 1970")
                .as_secs() as i64
        });
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm), to avoid a date crate.
    let z = epoch.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    println!("cargo:rustc-env=PORTAL_BUILD_DATE={year:04}-{month:02}-{day:02}");
    // Every command in `lib.rs`'s `generate_handler!`, in the same order. A command absent
    // here gets no generated permission, so `capabilities/default.json` cannot grant it and
    // every call is rejected at the IPC boundary — see the sync test in `lib.rs`.
    const COMMANDS: &[&str] = &[
        "auth_session",
        "auth_claim",
        "auth_login",
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
        "get_transactions",
        "list_addresses",
        "list_utxos",
        "send_to_address",
        "sync_wallet",
        "estimate_fees",
        "estimate_send_fee",
        "get_btc_price",
        "get_offers",
        "sync_offerbook",
        "poll_maker",
        "remove_maker",
        "prepare_swap",
        "estimate_swap_funding",
        "start_swap",
        "cancel_swap",
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
        "get_restore_logs",
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
        "list_maker_addresses",
        "list_maker_utxos",
        "get_maker_new_address",
        "get_maker_transactions",
        "send_maker_to_address",
        "backup_maker_wallet",
        "sync_maker_wallet",
        "list_maker_fidelity_bonds",
        "list_makers",
        "get_saved_maker_settings",
        "list_dashboard_imports",
        "import_dashboard_makers",
        "check_router_config",
        "clear_maker_settings",
        "get_router_defaults",
        "get_suggested_maker_ports",
        "check_maker_ports",
        "list_taker_blocklist",
        "import_taker_blocklist",
        "remove_taker_blocklist",
        "list_maker_blocklist",
        "import_maker_blocklist",
        "remove_maker_blocklist",
        "get_maker_logs",
        "quit_app",
    ];
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    tauri_build::try_build(attributes).expect("failed to build Tauri command manifest")
}
