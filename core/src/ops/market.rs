//! Market / offerbook commands. Sync goes through the cached OfferSyncClient
//! so it never contends with a running swap.

use std::sync::Arc;

use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use openswap::taker::offers::{MakerOfferCandidate, MakerProtocol, MakerState};

use crate::error::AppError;
use crate::state::{try_lock_taker, AppState};
use crate::types::{MakerDto, OfferBookView, OfferDto};

fn to_maker_dto(m: MakerOfferCandidate) -> MakerDto {
    let state = match m.state {
        MakerState::Good => "good",
        MakerState::Bad => "bad",
        MakerState::Unresponsive { .. } => "unresponsive",
    };
    let protocol = m.protocol.map(|p| match p {
        MakerProtocol::Legacy => "legacy".to_string(),
        MakerProtocol::Taproot => "taproot".to_string(),
        MakerProtocol::Unified => "unified".to_string(),
    });
    let offer = m.offer.map(|o| {
        let bond = &o.fidelity.bond;
        let outpoint = bond.outpoint();
        OfferDto {
            base_fee: o.base_fee,
            amount_relative_fee_pct: o.amount_relative_fee_pct,
            time_relative_fee_pct: o.time_relative_fee_pct,
            required_confirms: o.required_confirms,
            minimum_locktime: o.minimum_locktime,
            max_size: o.max_size,
            min_size: o.min_size,
            bond_amount_sats: bond.amount.to_sat(),
            bond_locktime_height: bond.lock_time.to_consensus_u32(),
            bond_txid: outpoint.txid.to_string(),
            bond_vout: outpoint.vout,
            bond_is_spent: bond.is_spent(),
        }
    });
    MakerDto {
        address: m.address.to_string(),
        protocol,
        offer,
        state: state.to_string(),
    }
}

/// Cached snapshot — no network I/O.
pub fn get_offers(state: &Arc<AppState>) -> Result<OfferBookView, AppError> {
    let taker_guard = try_lock_taker(&state.taker)?;
    let taker = taker_guard.as_ref().ok_or_else(AppError::not_initialized)?;
    let book = taker.fetch_offers()?;

    let mut good = Vec::new();
    let mut bad = Vec::new();
    let mut unresponsive = Vec::new();
    for maker in book.all_makers() {
        match maker.state {
            MakerState::Good => good.push(to_maker_dto(maker)),
            MakerState::Bad => bad.push(to_maker_dto(maker)),
            MakerState::Unresponsive { .. } => unresponsive.push(to_maker_dto(maker)),
        }
    }

    Ok(OfferBookView {
        good,
        bad,
        unresponsive,
        syncing: state.is_offerbook_syncing.load(Ordering::Relaxed),
        last_sync_ts: state.last_offerbook_sync_ts.load(Ordering::Relaxed),
    })
}

/// Maker discovery over Nostr + Tor; can take 30-60s+. `syncing`/`last_sync_ts`
/// in get_offers are our own bookkeeping (the crate doesn't expose them on
/// the public client) and only reflect syncs triggered here.
pub async fn sync_offerbook(state: &Arc<AppState>) -> Result<(), AppError> {
    let client = state
        .offer_sync
        .read()?
        .clone()
        .ok_or_else(AppError::not_initialized)?;

    state.is_offerbook_syncing.store(true, Ordering::Relaxed);
    let result = tokio::task::spawn_blocking(move || client.sync_and_wait())
        .await
        .map_err(AppError::internal)
        .and_then(|r| r.map_err(AppError::from));
    state.is_offerbook_syncing.store(false, Ordering::Relaxed);

    if result.is_ok() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        state.last_offerbook_sync_ts.store(now, Ordering::Relaxed);
    }
    result
}

pub async fn poll_maker(
    state: &Arc<AppState>,
    address: String,
) -> Result<MakerDto, AppError> {
    let taker = state.taker.clone();
    tokio::task::spawn_blocking(move || -> Result<MakerDto, AppError> {
        let guard = try_lock_taker(&taker)?;
        let taker = guard.as_ref().ok_or_else(AppError::not_initialized)?;
        Ok(to_maker_dto(taker.poll_maker(address)?))
    })
    .await
    .map_err(AppError::internal)?
}

pub async fn remove_maker(
    state: &Arc<AppState>,
    address: String,
) -> Result<bool, AppError> {
    let taker = state.taker.clone();
    tokio::task::spawn_blocking(move || -> Result<bool, AppError> {
        let guard = try_lock_taker(&taker)?;
        let taker = guard.as_ref().ok_or_else(AppError::not_initialized)?;
        Ok(taker.remove_maker(address)?)
    })
    .await
    .map_err(AppError::internal)?
}
