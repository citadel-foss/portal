//! The maker's own wallet operations — split out from `commands::maker`
//! (which owns lifecycle) since it's the exact same shape as
//! `commands::taker_wallet`'s operations, selected from `state.makers` by ID.
//! The maker's wallet is the same
//! `openswap::wallet::Wallet` type the taker uses, so the DTOs
//! (`BalancesDto`, `UtxoEntry`, `NewAddress`) are
//! shared, not duplicated.

use std::sync::{Arc, RwLock};

use openswap::wallet::{AddressType, Wallet};

use crate::ops::chain_backend;
use crate::error::AppError;
use crate::state::AppState;
use crate::types::{
    AddressTypeDto, BalancesDto, FidelityBondDto, NewAddress, Outpoint, TxSummary, UtxoEntry,
};

/// Cloning the Arc (not the Wallet) keeps this independent of the maker
/// mutex — same reasoning as `commands::taker_wallet::get_wallet_handle`.
fn get_maker_wallet_handle(
    state: &Arc<AppState>,
    router_id: &str,
) -> Result<Arc<RwLock<Wallet>>, AppError> {
    let makers = state.makers.lock()?;
    let entry = makers
        .get(router_id)
        .ok_or_else(|| AppError::maker_not_found(router_id))?;
    entry
        .runtime
        .as_ref()
        .map(|runtime| runtime.server.wallet.clone())
        .ok_or_else(AppError::maker_not_initialized)
}

pub async fn get_maker_balances(
    state: &Arc<AppState>,
    router_id: String,
) -> Result<BalancesDto, AppError> {
    let wallet = get_maker_wallet_handle(state, &router_id)?;
    tokio::task::spawn_blocking(move || -> Result<BalancesDto, AppError> {
        let b = wallet.read()?.get_balances()?;
        Ok(BalancesDto {
            regular: b.regular.to_sat(),
            swap: b.swap.to_sat(),
            contract: b.contract.to_sat(),
            fidelity: b.fidelity.to_sat(),
            spendable: b.spendable.to_sat(),
        })
    })
    .await
    .map_err(AppError::internal)?
}

pub async fn list_maker_utxos(
    state: &Arc<AppState>,
    router_id: String,
) -> Result<Vec<UtxoEntry>, AppError> {
    let wallet = get_maker_wallet_handle(state, &router_id)?;
    let socks_port = state
        .makers
        .lock()?
        .get(&router_id)
        .map(|maker| maker.settings.socks_port);
    tokio::task::spawn_blocking(move || -> Result<Vec<UtxoEntry>, AppError> {
        let utxos = wallet.read()?.list_all_utxo_spend_info();
        Ok(utxos
            .into_iter()
            .map(|(entry, spend_info)| UtxoEntry {
                txid: entry.txid.to_string(),
                vout: entry.vout,
                amount_sats: entry.amount.to_sat(),
                confirmations: entry.confirmations,
                address: chain_backend::utxo_address(&entry, socks_port),
                spendable: entry.spendable,
                solvable: entry.solvable,
                spend_type: spend_info.to_string(),
            })
            .collect())
    })
    .await
    .map_err(AppError::internal)?
}

pub async fn get_maker_transactions(
    state: &Arc<AppState>,
    router_id: String,
    count: Option<usize>,
    skip: Option<usize>,
) -> Result<Vec<TxSummary>, AppError> {
    let wallet = get_maker_wallet_handle(state, &router_id)?;
    tokio::task::spawn_blocking(move || -> Result<Vec<TxSummary>, AppError> {
        let txs = wallet.read()?.get_transactions(count, skip)?;
        Ok(txs
            .into_iter()
            .map(|tx| TxSummary {
                txid: tx.info.txid.to_string(),
                category: format!("{:?}", tx.detail.category).to_lowercase(),
                amount_sats: tx.detail.amount.to_sat(),
                confirmations: tx.info.confirmations,
                address: tx.detail.address.map(|a| a.assume_checked().to_string()),
                time: tx.info.time,
                fee_sats: tx.detail.fee.map(|f| f.to_sat()),
                label: tx.detail.label,
            })
            .collect())
    })
    .await
    .map_err(AppError::internal)?
}

/// Unlike `commands::taker_wallet::get_new_address`, this always derives a fresh address rather
/// than re-offering the last unused one — no gap-limit-safe caching for the maker wallet yet.
pub async fn get_maker_new_address(
    state: &Arc<AppState>,
    router_id: String,
    address_type: AddressTypeDto,
) -> Result<NewAddress, AppError> {
    let wallet = get_maker_wallet_handle(state, &router_id)?;
    let (addr_type, label) = match address_type {
        AddressTypeDto::P2wpkh => (AddressType::P2WPKH, "p2wpkh"),
        AddressTypeDto::P2tr => (AddressType::P2TR, "p2tr"),
    };
    tokio::task::spawn_blocking(move || -> Result<NewAddress, AppError> {
        let address = wallet
            .write()?
            .get_next_external_address(addr_type)?
            .to_string();
        Ok(NewAddress {
            address,
            address_type: label.to_string(),
            // Always freshly derived here, so there is nothing to check.
            verified: true,
        })
    })
    .await
    .map_err(AppError::internal)?
}

pub async fn sync_maker_wallet(
    state: &Arc<AppState>,
    router_id: String,
) -> Result<(), AppError> {
    // Holds the server (not just the wallet) so stopping the maker aborts an
    // in-flight sync instead of leaving it to run out its backend retries.
    let server = {
        let makers = state.makers.lock()?;
        makers
            .get(&router_id)
            .ok_or_else(|| AppError::maker_not_found(&router_id))?
            .runtime
            .as_ref()
            .map(|runtime| runtime.server.clone())
            .ok_or_else(AppError::maker_not_initialized)?
    };
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        server.wallet.write()?.sync_and_save(&server.shutdown)?;
        Ok(())
    })
    .await
    .map_err(AppError::internal)?
}

/// `Wallet::get_fidelity_bonds()`/`calculate_bond_value` already expose everything structured
/// (see `FidelityBondDto`'s doc comment) — no need for the crate's own pre-formatted string dump.
pub async fn list_maker_fidelity_bonds(
    state: &Arc<AppState>,
    router_id: String,
) -> Result<Vec<FidelityBondDto>, AppError> {
    let wallet = get_maker_wallet_handle(state, &router_id)?;
    tokio::task::spawn_blocking(move || -> Result<Vec<FidelityBondDto>, AppError> {
        let wallet = wallet.read()?;
        let (tip_height, tip_time) = wallet.chain_tip()?;
        Ok(wallet
            .get_fidelity_bonds()
            .iter()
            .map(|bond| {
                let outpoint = bond.outpoint();
                let lock_time_height = bond.lock_time.to_consensus_u32();
                let is_spent = bond.is_spent();
                FidelityBondDto {
                    bond_index: bond.bond_index,
                    outpoint: Outpoint {
                        txid: outpoint.txid.to_string(),
                        vout: outpoint.vout,
                    },
                    amount_sats: bond.amount.to_sat(),
                    lock_time_height,
                    is_spent,
                    is_locked: !is_spent && (tip_height as u32) < lock_time_height,
                    bond_value_sats: (!is_spent)
                        .then(|| wallet.calculate_bond_value(bond, tip_height, tip_time).ok())
                        .flatten()
                        .map(|v| v.to_sat()),
                }
            })
            .collect())
    })
    .await
    .map_err(AppError::internal)?
}
