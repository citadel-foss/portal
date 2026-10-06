//! Market / offerbook commands. Sync goes through the cached OfferSyncClient
//! so it never contends with a running swap.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use openswap::taker::offers::{MakerOfferCandidate, MakerProtocol, MakerState, OfferBook};

use crate::error::{AppError, ErrorCode};
use crate::state::{try_lock_taker, TakerInstance};
use crate::types::{MakerDto, OfferBookView, OfferDto};

/// A router's self-chosen name with the invisible formatting characters taken out. The crate
/// only refuses control characters, which leaves direction overrides and zero-width characters
/// free to make one router's name render as another's. Emptied by that, it reads as unnamed and
/// the UI shows the address instead.
fn visible_name(raw: &str) -> String {
    raw.chars()
        .filter(|c| {
            !matches!(
                c,
                '\u{00AD}'
                    | '\u{061C}'
                    | '\u{180E}'
                    | '\u{200B}'..='\u{200F}'
                    | '\u{202A}'..='\u{202E}'
                    | '\u{2060}'..='\u{206F}'
                    | '\u{FEFF}'
                    | '\u{FFF9}'..='\u{FFFB}'
            )
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn to_maker_dto(m: MakerOfferCandidate) -> MakerDto {
    let state = match m.state {
        MakerState::Good => "good",
        MakerState::Banned(_) => "bad",
        MakerState::Unavailable(_) => "unresponsive",
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
            name: visible_name(&o.name),
            base_fee: o.base_fee,
            amount_relative_fee_pct: o.amount_relative_fee_pct,
            time_relative_fee_pct: o.time_relative_fee_pct,
            required_confirms: o.required_confirms,
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
///
/// Falls back to the offerbook file when a running swap holds the taker: the sync service
/// writes it after every poll, so the market stays readable for the hours a swap owns the
/// mutex instead of going blank for the whole of it.
pub fn get_offers(state: &TakerInstance) -> Result<OfferBookView, AppError> {
    let makers = match try_lock_taker(&state.taker) {
        Ok(guard) => guard
            .as_ref()
            .ok_or_else(AppError::not_initialized)?
            .fetch_offers()?
            .all_makers(),
        // Parsed directly rather than through `OfferBookHandle::load_or_create`, which logs a line
        // and rewrites the file on every call — once per poll for the length of a swap, racing
        // the sync service that owns the file.
        Err(busy) if busy.code == ErrorCode::SwapInProgress => {
            match std::fs::read(state.data_dir.join("offerbook.json")) {
                Ok(bytes) => serde_json::from_slice::<OfferBook>(&bytes)
                    .map_err(AppError::internal)?
                    .all_makers(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(AppError::internal(e)),
            }
        }
        Err(other) => return Err(other),
    };

    let mut good = Vec::new();
    let mut bad = Vec::new();
    let mut unresponsive = Vec::new();
    for maker in makers {
        match maker.state {
            MakerState::Good => good.push(to_maker_dto(maker)),
            MakerState::Banned(_) => bad.push(to_maker_dto(maker)),
            MakerState::Unavailable(_) => unresponsive.push(to_maker_dto(maker)),
        }
    }

    Ok(OfferBookView {
        good,
        bad,
        unresponsive,
        syncing: state.is_offerbook_syncing.load(Ordering::Relaxed),
    })
}

/// Maker discovery over Nostr + Tor; can take 30-60s+. `syncing` in get_offers is our own
/// bookkeeping (the crate doesn't expose it on the public client) and only reflects syncs
/// triggered here.
pub async fn sync_offerbook(state: &TakerInstance) -> Result<(), AppError> {
    let client = state.offer_sync.clone();

    state.is_offerbook_syncing.store(true, Ordering::Relaxed);
    let result = tokio::task::spawn_blocking(move || client.sync_and_wait())
        .await
        .map_err(AppError::internal)
        .and_then(|r| r.map_err(AppError::from));
    state.is_offerbook_syncing.store(false, Ordering::Relaxed);
    result
}

/// The crate's sync thread takes polls one at a time and none during a sync, which can run for
/// minutes, and the client waits on it with no deadline. Past this the caller is told to retry;
/// the poll itself still runs when the thread gets to it and records its result.
const POLL_DEADLINE: Duration = Duration::from_secs(120);

/// The crate polls one router at a time, so polls past this many only wait behind each other.
const MAX_QUEUED_POLLS: usize = 4;

pub async fn poll_maker(
    state: &TakerInstance,
    address: String,
) -> Result<MakerDto, AppError> {
    // Through the offer-sync client, as `Taker::poll_maker` itself does, but without the taker
    // mutex: a running swap holds that for hours.
    let address = openswap::taker::offers::MakerAddress::try_from(address)
        .map_err(|e| AppError::new(ErrorCode::InvalidInput, format!("Invalid router address: {e}")))?;
    let key = address.to_string();
    {
        let mut polls = state.polls_in_flight.lock()?;
        if polls.contains(&key) {
            return Err(AppError::new(
                ErrorCode::RouterBusy,
                "This router's last poll is still waiting for the market sync. Try again in a moment.",
            ));
        }
        if polls.len() >= MAX_QUEUED_POLLS {
            return Err(AppError::new(
                ErrorCode::RouterBusy,
                "Several polls are already waiting for the market sync. Try again in a moment.",
            ));
        }
        polls.insert(key.clone());
    }
    let client = state.offer_sync.clone();
    let in_flight = state.polls_in_flight.clone();
    let poll = tokio::task::spawn_blocking(move || -> Result<MakerDto, AppError> {
        // Released when the crate answers, not when the caller stops waiting, and on a panic too,
        // or the router would refuse polls for the rest of the session.
        struct Release(Arc<Mutex<HashSet<String>>>, String);
        impl Drop for Release {
            fn drop(&mut self) {
                if let Ok(mut polls) = self.0.lock() {
                    polls.remove(&self.1);
                }
            }
        }
        let _release = Release(in_flight, key);
        Ok(to_maker_dto(client.poll_maker(address)?))
    });
    match tokio::time::timeout(POLL_DEADLINE, poll).await {
        Ok(joined) => joined.map_err(AppError::internal)?,
        Err(_) => Err(AppError::new(
            ErrorCode::RouterBusy,
            "The market is still syncing, so this router could not be polled yet. Try again in a moment.",
        )),
    }
}

pub async fn remove_maker(
    state: &TakerInstance,
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

#[cfg(test)]
mod tests {
    use super::visible_name;

    #[test]
    fn invisible_characters_cannot_disguise_a_name() {
        assert_eq!(visible_name("Galaxy\u{202E}yortseD"), "GalaxyyortseD");
        assert_eq!(visible_name("Asteroid\u{200B} Destroyer"), "Asteroid Destroyer");
        assert_eq!(visible_name("\u{2066}\u{FEFF}"), "");
        assert_eq!(visible_name("satoshi's lounge"), "satoshi's lounge");
    }
}
