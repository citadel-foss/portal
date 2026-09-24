//! The web route table: one explicit entry per exposed operation.
//!
//! Generated against `contracts/operations.json` and core's real signatures, then checked by
//! test. Deliberately not a reflective dispatcher — each entry deserializes its own concrete
//! request type and rejects unknown fields, so a body cannot select behavior the inventory
//! does not list.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use portal_core::error::{AppError, ErrorCode};
use portal_core::ops;
use portal_core::state::AppState;
use portal_core::types::*;
use serde_json::Value;
use uuid::Uuid;

type Run = fn(Arc<AppState>, Value) -> Pin<Box<dyn Future<Output = Result<Value, AppError>> + Send>>;

pub struct Operation {
    pub name: &'static str,
    /// Whether a CSRF token is required. Only pure reads are exempt: a probe reaches the
    /// network on the server's behalf, which is worth protecting from a cross-site trigger.
    pub mutates: bool,
    pub run: Run,
}

const fn op(name: &'static str, mutates: bool, run: Run) -> Operation {
    Operation { name, mutates, run }
}

/// Strips the server path from a result before it crosses to the browser.
///
/// Desktop shows it because the user picked the folder; on the web there is exactly one data
/// root and nothing in the UI displays it — it only keys a per-wallet sync timestamp, which
/// stays unique without it. Emptied rather than dropped because the field is not optional,
/// and the frontend already coerces an absent one to the same thing.
fn without_server_path(mut result: InitResult) -> InitResult {
    result.data_dir = String::new();
    result
}

fn parse<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, AppError> {
    let args = if args.is_null() { Value::Object(Default::default()) } else { args };
    serde_json::from_value(args)
        .map_err(|e| AppError::new(ErrorCode::InvalidInput, format!("invalid request body: {e}")))
}

fn encode<T: serde::Serialize>(value: &T) -> Result<Value, AppError> {
    serde_json::to_value(value).map_err(AppError::internal)
}

pub fn lookup(name: &str) -> Option<&'static Operation> {
    OPERATIONS.iter().find(|op| op.name == name)
}

pub static OPERATIONS: &[Operation] = &[
    op("check_backend", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            config: Option<ChainBackendConfig>,
            socks_port: Option<u16>,
            }
            let body: Args = parse(args)?;
            encode(&ops::chain_backend::check_backend(body.config, body.socks_port).await?)
        })
    }),
    op("check_maker_ports", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            network_port: u16,
            rpc_port: u16,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker_settings::check_maker_ports(body.network_port, body.rpc_port)?)
        })
    }),
    op("estimate_fees", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::taker_wallet::estimate_fees().await?)
        })
    }),
    op("estimate_swap_funding", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            amount_sats: u64,
            protocol: ProtocolVersionDto,
            outpoints: Option<Vec<Outpoint>>,
            tx_count: Option<u32>,
            }
            let body: Args = parse(args)?;
            encode(&ops::taker_swap::estimate_swap_funding(&rt, body.amount_sats, body.protocol, body.outpoints, body.tx_count).await?)
        })
    }),
    op("get_balances", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::taker_wallet::get_balances(&rt).await?)
        })
    }),
    op("get_btc_price", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::taker_wallet::get_btc_price().await?)
        })
    }),
    op("get_chain_backend", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::chain_backend::get_chain_backend())
        })
    }),
    op("get_incoming_swap_utxo", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            swap_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::taker_reports::get_incoming_swap_utxo(&rt, body.swap_id).await?)
        })
    }),
    op("get_logs", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            lines: Option<usize>,
            }
            let body: Args = parse(args)?;
            encode(&ops::logs::get_logs(&rt, body.lines).await?)
        })
    }),
    op("get_maker_balances", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker_wallet::get_maker_balances(&rt, body.router_id).await?)
        })
    }),
    op("get_maker_info", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker::get_maker_info(&rt, body.router_id)?)
        })
    }),
    op("get_maker_logs", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            lines: Option<usize>,
            }
            let body: Args = parse(args)?;
            encode(&ops::logs::get_maker_logs(&rt, body.router_id, body.lines).await?)
        })
    }),
    op("get_maker_status", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker::get_maker_status(&rt, body.router_id)?)
        })
    }),
    op("get_maker_swap_report", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            swap_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker_reports::get_maker_swap_report(&rt, body.router_id, body.swap_id).await?)
        })
    }),
    op("get_maker_transactions", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            count: Option<usize>,
            skip: Option<usize>,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker_wallet::get_maker_transactions(&rt, body.router_id, body.count, body.skip).await?)
        })
    }),
    op("get_offers", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::market::get_offers(&rt)?)
        })
    }),
    op("get_recovery_status", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            swap_id: Option<String>,
            }
            let body: Args = parse(args)?;
            encode(&ops::taker_swap::get_recovery_status(&rt, body.swap_id).await?)
        })
    }),
    op("get_saved_maker_settings", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker_settings::get_saved_maker_settings(body.router_id)?)
        })
    }),
    op("get_router_defaults", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move { encode(&ops::maker::router_defaults()) })
    }),
    op("get_suggested_maker_ports", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::maker_settings::get_suggested_maker_ports()?)
        })
    }),
    op("get_swap_preparation", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            since: u64,
            }
            let body: Args = parse(args)?;
            encode(&ops::taker_swap::get_swap_preparation(&rt, body.since).await?)
        })
    }),
    op("get_swap_progress", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::taker_swap::get_swap_progress(&rt)?)
        })
    }),
    op("get_swap_report", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            swap_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::taker_reports::get_swap_report(&rt, body.swap_id).await?)
        })
    }),
    op("get_swap_tracker", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            swap_id: Option<String>,
            }
            let body: Args = parse(args)?;
            encode(&ops::taker_swap::get_swap_tracker(&rt, body.swap_id).await?)
        })
    }),
    op("get_transactions", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            count: Option<usize>,
            skip: Option<usize>,
            }
            let body: Args = parse(args)?;
            encode(&ops::taker_wallet::get_transactions(&rt, body.count, body.skip).await?)
        })
    }),
    op("get_session_state", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            // Desktop shows the path because the user chose it; a browser has no business
            // learning where the server keeps its files, and nothing in the web UI reads it.
            let mut state = ops::taker_wallet::get_session_state(&rt);
            state.data_dir = None;
            encode(&state)
        })
    }),
    op("get_wallet_info", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::taker_wallet::get_wallet_info(&rt)?)
        })
    }),
    op("list_maker_fidelity_bonds", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker_wallet::list_maker_fidelity_bonds(&rt, body.router_id).await?)
        })
    }),
    op("list_maker_swap_reports", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker_reports::list_maker_swap_reports(&rt, body.router_id).await?)
        })
    }),
    op("list_maker_utxos", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker_wallet::list_maker_utxos(&rt, body.router_id).await?)
        })
    }),
    op("list_makers", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::maker_settings::list_makers()?)
        })
    }),
    op("list_recoveries", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::taker_swap::list_recoveries(&rt).await?)
        })
    }),
    op("list_swap_reports", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::taker_reports::list_swap_reports(&rt).await?)
        })
    }),
    op("list_utxos", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::taker_wallet::list_utxos(&rt).await?)
        })
    }),
    op("list_wallets", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            /// Accepted so the shared frontend can keep one call shape, and then discarded:
            /// a browser naming a directory would enumerate `<anywhere>/wallets` on the host.
            /// The contract calls this out as "redact paths".
            #[allow(dead_code)]
            data_dir: Option<String>,
            }
            let _: Args = parse(args)?;
            encode(&ops::taker_wallet::list_wallets(None)?)
        })
    }),
    op("poll_maker", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            address: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::market::poll_maker(&rt, body.address).await?)
        })
    }),
    op("validate_address", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            address: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::taker_wallet::validate_address(body.address))
        })
    }),
    op("verify_deniability", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            swap_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::taker_reports::verify_deniability(&rt, body.swap_id).await?)
        })
    }),
    op("verify_last_address", false, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            address_type: AddressTypeDto,
            }
            let body: Args = parse(args)?;
            encode(&ops::taker_wallet::verify_last_address(&rt, body.address_type).await?)
        })
    }),
    op("verify_maker_deniability", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            swap_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker_reports::verify_maker_deniability(&rt, body.router_id, body.swap_id).await?)
        })
    }),
    op("check_tor", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::setup::check_tor().await?)
        })
    }),
    op("restart_tor_bootstrap", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::setup::restart_tor_bootstrap().await?)
        })
    }),
    op("set_chain_backend", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            config: ChainBackendConfig,
            }
            let body: Args = parse(args)?;
            encode(&ops::chain_backend::set_chain_backend(body.config)?)
        })
    }),
    op("sync_maker_wallet", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker_wallet::sync_maker_wallet(&rt, body.router_id).await?)
        })
    }),
    op("shutdown_taker", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move { encode(&ops::taker_wallet::shutdown_async(&rt).await?) })
    }),
    op("sync_offerbook", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::market::sync_offerbook(&rt).await?)
        })
    }),
    op("sync_wallet", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::taker_wallet::sync_wallet(&rt).await?)
        })
    }),
];

/// Operations that change persistent state or move money. These never run inline: they are
/// admitted to the journal first, so a lost response can be reconciled by the key the client
/// already holds rather than resubmitted blind.
pub static DURABLE: &[Operation] = &[
    op("clear_maker_settings", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker_settings::clear_maker_settings(&rt, body.router_id)?)
        })
    }),
    op("get_maker_new_address", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            address_type: AddressTypeDto,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker_wallet::get_maker_new_address(&rt, body.router_id, body.address_type).await?)
        })
    }),
    op("get_new_address", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            address_type: AddressTypeDto,
            }
            let body: Args = parse(args)?;
            encode(&ops::taker_wallet::get_new_address(&rt, body.address_type).await?)
        })
    }),
    op("init_maker", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            config: MakerInitConfig,
            }
            let mut body: Args = parse(args)?;
            // Nested one level down, and just as much a client-supplied server path: a
            // directory that does not exist yet is created, so this would write a wallet and
            // a Tor data dir wherever the caller pointed.
            body.config.data_dir = None;
            encode(&ops::maker::init_maker(&rt, body.config).await?)
        })
    }),
    op("init_taker", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            config: InitConfig,
            }
            let mut body: Args = parse(args)?;
            // Also repoints the debug log at the given directory, so it must be ours.
            body.config.data_dir = None;
            encode(&without_server_path(ops::taker_wallet::init_taker(&rt, body.config).await?))
        })
    }),
    op("prepare_swap", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            request: SwapRequest,
            }
            let body: Args = parse(args)?;
            encode(&ops::taker_swap::prepare_swap(&rt, body.request).await?)
        })
    }),
    op("recover_swap", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            encode(&ops::taker_swap::recover_swap(&rt).await?)
        })
    }),
    op("remove_maker", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            address: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::market::remove_maker(&rt, body.address).await?)
        })
    }),
    op("restore_wallet", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            /// Accepted and discarded — "no supplied server path", per this operation's
            /// contract entry. The restore always lands in the server-resolved root.
            #[allow(dead_code)]
            data_dir: Option<String>,
            wallet_name: String,
            socks_port: Option<u16>,
            selection_id: Uuid,
            password: Option<String>,
            }
            let body: Args = parse(args)?;
            encode(&ops::taker_wallet::restore_wallet(&rt, None, body.wallet_name, body.socks_port, body.selection_id, body.password).await?)
        })
    }),
    op("send_maker_to_address", true, |rt, args| {
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                router_id: String,
                address: String,
                amount_sats: u64,
                fee_rate: Option<f64>,
                outpoints: Option<Vec<portal_core::types::Outpoint>>,
            }
            let body: Args = parse(args)?;
            encode(
                &ops::maker_wallet::send_maker_to_address(
                    &rt,
                    body.router_id,
                    body.address,
                    body.amount_sats,
                    body.fee_rate,
                    body.outpoints,
                )
                .await?,
            )
        })
    }),
    op("send_to_address", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            address: String,
            amount_sats: u64,
            fee_rate: Option<f64>,
            outpoints: Option<Vec<Outpoint>>,
            }
            let body: Args = parse(args)?;
            encode(&ops::taker_wallet::send_to_address(&rt, body.address, body.amount_sats, body.fee_rate, body.outpoints).await?)
        })
    }),
    op("start_maker", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            wallet_password: Option<String>,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker::start_maker(&rt, body.router_id, body.wallet_password).await?)
        })
    }),
    op("start_swap", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            swap_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::taker_swap::start_swap(&rt, body.swap_id).await?)
        })
    }),
    op("stop_maker", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker::stop_maker(&rt, body.router_id).await?)
        })
    }),
    op("update_maker_settings", true, |rt, args| {
        let _ = (&rt, &args);
        Box::pin(async move {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
            router_id: String,
            settings: MakerSettingsDto,
            }
            let body: Args = parse(args)?;
            encode(&ops::maker::update_maker_settings(&rt, body.router_id, body.settings)?)
        })
    }),
];

pub fn lookup_durable(name: &str) -> Option<&'static Operation> {
    DURABLE.iter().find(|op| op.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The inventory is the contract; the route table must be exactly its web subset for the
    /// classes this host has audited. A command added to one and not the other fails here.
    #[test]
    fn routes_match_the_audited_subset_of_the_inventory() {
        let contracts = include_str!("../../contracts/operations.json");
        let inventory: serde_json::Value = serde_json::from_str(contracts).unwrap();
        let expected: Vec<&str> = inventory["operations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| o["web"].as_bool().unwrap())
            .filter(|o| matches!(o["executionClass"].as_str(), Some("read" | "probe" | "transient")))
            // Served by the specialized backup endpoint, not by a command route.
            .filter(|o| o["webAdaptation"] != "specialized" || o["name"] != "backup_wallet")
            .map(|o| o["name"].as_str().unwrap())
            .collect();

        let mut routed: Vec<&str> = OPERATIONS.iter().map(|o| o.name).collect();
        routed.sort_unstable();
        let mut expected = expected;
        expected.sort_unstable();
        assert_eq!(routed, expected);
    }

    #[test]
    fn a_name_outside_the_table_never_resolves() {
        assert!(lookup("quit_app").is_none());
        assert!(lookup("choose_restore_backup").is_none());
        assert!(lookup("../../etc/passwd").is_none());
        // Durable operations live in their own table behind admission, never inline.
        assert!(lookup("send_to_address").is_none());
        assert!(lookup_durable("send_to_address").is_some());
        assert!(lookup_durable("get_balances").is_none());
    }

    /// Reads are exempt from CSRF; anything that reaches the network on the server's behalf
    /// is not.
    #[test]
    fn only_pure_reads_skip_csrf() {
        assert!(!lookup("get_balances").unwrap().mutates);
        assert!(lookup("check_backend").unwrap().mutates);
        assert!(lookup("get_btc_price").unwrap().mutates);
    }
}
