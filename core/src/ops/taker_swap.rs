//! Swap execution: two-phase prepare/start, coarse in-memory progress, recovery.
//!
//! Live per-maker progress (`get_swap_tracker`) reads `openswap::taker::swap_tracker::SwapTracker`
//! directly — a public crate API (`SwapTracker`/`SwapRecord`/`MakerProgress` are all `pub`,
//! `Serialize`/`Deserialize`), the same `<data_dir>/swap_tracker.cbor` file the old Electron app
//! polled straight off disk.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use std::str::FromStr;
use std::time::SystemTime;

use openswap::bitcoin::{Address, Amount, OutPoint, ScriptBuf, Txid, WitnessProgram, WitnessVersion};
use openswap::protocol::ProtocolVersion;
use openswap::taker::swap_tracker::{
    ContractResolution, ExchangeProgress, LegacyExchangeProgress, MakerProgress,
    RecoveryPhase, SwapPhase, SwapRecord, SwapTracker, TaprootExchangeProgress,
};
use openswap::taker::{SwapParams, SwapSummary};
use openswap::utill::{
    funding_fee_policy_sats, sweep_fee_policy_sats, MAX_TX_COUNT, MIN_RELAY_FEE_RATE,
};
use openswap::wallet::{Blockchain, UTXOSpendInfo};
use crate::events::AppEvent;
use crate::ops::taker_reports::{recovery_reports, tracker_records};

use crate::error::{AppError, ErrorCode};
use crate::security::operation::{SensitiveOperation, SensitiveOperationGuard};
use crate::state::{try_lock_taker, ActiveSwap, AppState, SwapLifecycle, TakerInstance};
use crate::types::{
    ProtocolVersionDto, RecoveredContractDto, RecoveryContractDto, RecoveryStatus, RecoverySummary,
    RecoveryHandoff,
    PaymentQuoteDto, SwapPreparationDto, RouterFeeInfoDto, RouterMilestoneDto,
    RouterProgressDto, RouterStageDto, SwapFundingEstimateDto, SwapProgressDto, SwapRequest,
    SwapSummaryDto, SwapTrackerDto,
};

fn protocol_label(p: ProtocolVersion) -> &'static str {
    match p {
        ProtocolVersion::Legacy => "legacy",
        ProtocolVersion::Taproot => "taproot",
    }
}

fn to_summary_dto(s: &SwapSummary) -> SwapSummaryDto {
    let router_fees: u64 = s.makers.iter().map(|m| m.estimated_fee_sats).sum();
    SwapSummaryDto {
        swap_id: s.swap_id.clone(),
        protocol: protocol_label(s.protocol).to_string(),
        send_amount_sats: s.send_amount.to_sat(),
        routers: s
            .makers
            .iter()
            .map(|m| RouterFeeInfoDto {
                address: m.address.clone(),
                protocol: protocol_label(m.protocol).to_string(),
                base_fee: m.base_fee,
                amount_relative_fee_pct: m.amount_relative_fee_pct,
                time_relative_fee_pct: m.time_relative_fee_pct,
                locktime: m.locktime,
                estimated_fee_sats: m.estimated_fee_sats,
            })
            .collect(),
        total_estimated_fee_sats: s.total_estimated_fee.to_sat(),
        estimated_receive_amount_sats: s.estimated_receive_amount.to_sat(),
        router_fee_sats: router_fees,
        // Whatever the ceiling holds beyond the routers' own service fees is miner cost:
        // our funding, each hop's forwarding, and the claim sweep.
        mining_fee_sats: s.total_estimated_fee.to_sat().saturating_sub(router_fees),
        payment: s.payment.as_ref().map(|p| PaymentQuoteDto {
            address: p.address.to_string(),
            amount_sats: p.amount.to_sat(),
        }),
    }
}

fn tracker_phase_label(phase: SwapPhase) -> &'static str {
    match phase {
        SwapPhase::MakersDiscovered => "routers_discovered",
        SwapPhase::Negotiated => "negotiated",
        SwapPhase::FundingCreated => "funding_created",
        SwapPhase::FundsBroadcast => "funds_broadcast",
        SwapPhase::ContractsExchanged => "contracts_exchanged",
        SwapPhase::Finalizing => "finalizing",
        SwapPhase::PrivkeysForwarded => "privkeys_forwarded",
        SwapPhase::Completed => "completed",
        SwapPhase::Failed => "failed",
    }
}

fn legacy_milestones(p: &LegacyExchangeProgress) -> Vec<(&'static str, bool)> {
    vec![
        ("connected", p.connected),
        ("sender_sigs_requested", p.sender_sigs_requested),
        ("sender_sigs_received", p.sender_sigs_received),
        ("prev_funding_broadcast", p.prev_funding_broadcast),
        ("prev_funding_confirmed", p.prev_funding_confirmed),
        ("proof_of_funding_sent", p.proof_of_funding_sent),
        ("maker_contracts_received", p.maker_contracts_received),
        ("next_maker_sigs_obtained", p.next_maker_sigs_obtained),
        ("prev_maker_sigs_obtained", p.prev_maker_sigs_obtained),
        ("combined_sigs_sent", p.combined_sigs_sent),
        ("maker_funding_confirmed", p.maker_funding_confirmed),
        ("watchonly_created", p.watchonly_created),
    ]
}

fn taproot_milestones(p: &TaprootExchangeProgress) -> Vec<(&'static str, bool)> {
    vec![
        ("connected", p.connected),
        ("contract_data_sent", p.contract_data_sent),
        ("maker_contract_received", p.maker_contract_received),
        ("swapcoins_created", p.swapcoins_created),
        ("maker_funding_confirmed", p.maker_funding_confirmed),
    ]
}

/// Collapse a maker's flags onto the stage the UI narrates.
///
/// Both protocols park the taker on a confirmation wait immediately after one specific flag, so
/// that flag — not a step count — is what says "this hop is waiting on the chain now".
fn router_stage(m: &MakerProgress) -> RouterStageDto {
    if m.finalization.privkey_forwarded {
        return RouterStageDto::Settled;
    }
    if m.finalization.privkey_received {
        return RouterStageDto::KeyReceived;
    }
    let (confirmed, awaiting_confirmation, connected) = match &m.exchange {
        ExchangeProgress::Taproot(p) => {
            (p.maker_funding_confirmed, p.contract_data_sent, p.connected)
        }
        ExchangeProgress::Legacy(p) => (
            p.maker_funding_confirmed,
            // Two waits per legacy hop: the previous hop's funding before the proof of funding
            // goes out, and this maker's own funding once it has the combined signatures.
            p.combined_sigs_sent || (p.prev_funding_broadcast && !p.prev_funding_confirmed),
            p.connected,
        ),
    };
    if confirmed {
        RouterStageDto::Routed
    } else if awaiting_confirmation {
        RouterStageDto::Confirming
    } else if connected {
        RouterStageDto::Handshaking
    } else if m.negotiated {
        RouterStageDto::Negotiated
    } else {
        RouterStageDto::Waiting
    }
}

fn to_router_progress_dto(m: &MakerProgress) -> RouterProgressDto {
    let mut flags = vec![("negotiated", m.negotiated)];
    flags.extend(match &m.exchange {
        ExchangeProgress::Legacy(p) => legacy_milestones(p),
        ExchangeProgress::Taproot(p) => taproot_milestones(p),
    });
    flags.push(("privkey_received", m.finalization.privkey_received));
    flags.push(("privkey_forwarded", m.finalization.privkey_forwarded));
    RouterProgressDto {
        address: m.address.clone(),
        stage: router_stage(m),
        milestones: flags
            .into_iter()
            .map(|(key, done)| RouterMilestoneDto { key, done })
            .collect(),
    }
}

fn to_tracker_dto(r: &SwapRecord) -> SwapTrackerDto {
    let mut routers: Vec<RouterProgressDto> = r.makers.iter().map(to_router_progress_dto).collect();

    // Taproot sets all five of a maker's exchange flags in one write, *after* the wait for that
    // maker's contract to confirm — so the maker whose turn it is reads as untouched for the
    // whole hop, which is the longest stretch of the swap. `FundsBroadcast` is exactly the phase
    // the per-maker exchange loop runs in, so inside it the first unstarted maker is in flight,
    // and confirmation is what it is overwhelmingly waiting on.
    if r.phase == SwapPhase::FundsBroadcast {
        if let Some(front) = routers
            .iter()
            .position(|x| x.stage <= RouterStageDto::Negotiated)
        {
            if routers[..front]
                .iter()
                .all(|x| x.stage >= RouterStageDto::Routed)
            {
                routers[front].stage = RouterStageDto::Confirming;
            }
        }
    }

    SwapTrackerDto {
        phase: tracker_phase_label(r.phase).to_string(),
        send_amount_sats: r.send_amount_sat,
        router_count: r.maker_count,
        failure_reason: r.failure_reason.clone(),
        routers,
        outgoing_contract_txids: r.outgoing_contract_txids.iter().map(Txid::to_string).collect(),
        incoming_contract_txids: r.incoming_contract_txids.iter().map(Txid::to_string).collect(),
        watchonly_contract_txids: r
            .watchonly_contract_txids
            .iter()
            .map(Txid::to_string)
            .collect(),
        outgoing_confirmed: false,
        protocol: protocol_label(r.protocol).to_string(),
        funding_wait_txids: Vec::new(),
        payment_address: r.payment_address.clone(),
        payment_amount_sats: r.payment_amount_sat,
    }
}

/// The crate rejects a rate under the relay floor only at prepare time, after maker discovery;
/// the ceiling stops a mistyped rate from spending a swap's coins on mining fees.
fn validate_feerate(rate: u64) -> Result<u64, AppError> {
    if rate < MIN_RELAY_FEE_RATE as u64 || rate as f64 > crate::ops::taker_wallet::MAX_FEE_RATE {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!(
                "feeRate must be between {} and {} sat/vB; anything higher is almost certainly a typo",
                MIN_RELAY_FEE_RATE as u64,
                crate::ops::taker_wallet::MAX_FEE_RATE
            ),
        ));
    }
    Ok(rate)
}

/// Quote every on-chain cost the taker bears directly: our own funding transactions, the
/// per-hop mining fee each maker deducts from the routed amount, and the sweep that claims
/// the incoming contracts at the end.
///
/// Every route figure is the ceiling `prepare_swap` will quote — the crate prices a hop at the
/// full `max_input_budget` on every one of `tx_count` splits — so the settled cost can only
/// come in under it. The fee rate, split count and input budget all come from `SwapParams`
/// rather than being restated here, since `prepare_swap` builds it the same way.
pub async fn estimate_swap_funding(
    taker: &TakerInstance,
    amount_sats: u64,
    protocol: ProtocolVersionDto,
    outpoints: Option<Vec<crate::types::Outpoint>>,
    tx_count: Option<u32>,
    fee_rate: u64,
) -> Result<SwapFundingEstimateDto, AppError> {
    let wallet = taker.wallet.clone();
    let protocol = match protocol {
        ProtocolVersionDto::Legacy => ProtocolVersion::Legacy,
        ProtocolVersionDto::Taproot => ProtocolVersion::Taproot,
    };
    // Only the fee defaults are read off this; the hop count never reaches a quote.
    let mut params = SwapParams::new(protocol, Amount::from_sat(amount_sats), 2)
        .with_feerate(validate_feerate(fee_rate)?);
    if let Some(count) = tx_count {
        params = params.with_tx_count(validate_tx_count(count)?);
    }
    let outpoints = outpoints
        .map(|items| {
            items
                .into_iter()
                .map(|item| {
                    let txid = Txid::from_str(&item.txid)
                        .map_err(|e| AppError::new(ErrorCode::InvalidInput, e.to_string()))?;
                    Ok(OutPoint::new(txid, item.vout))
                })
                .collect::<Result<Vec<_>, AppError>>()
        })
        .transpose()?;

    tokio::task::spawn_blocking(move || -> Result<SwapFundingEstimateDto, AppError> {
        let feerate = params.feerate as f64;
        let price_err = || AppError::internal("fee policy price overflow");
        // Each split is billed at the full input budget, and each of the hop's contracts costs
        // the maker one cooperative claim: the same two prices `prepare_swap` sums per hop.
        let split_funding_sats = funding_fee_policy_sats(
            params.max_input_budget as usize,
            params.max_input_budget,
            feerate,
        )
        .ok_or_else(price_err)?;
        let sweep_per_contract_sats =
            sweep_fee_policy_sats(params.protocol, feerate).ok_or_else(price_err)?;
        let route_mining_fee_per_router_sats = params.tx_count as u64
            * split_funding_sats
                .checked_add(sweep_per_contract_sats)
                .ok_or_else(price_err)?;

        // The very plan `prepare_swap` will replay: our own hop pays its fee on top of the
        // amount, so it runs with no input budget and no over-budget guard.
        let splits = wallet.read()?.plan_funding(
            Amount::from_sat(amount_sats),
            params.tx_count,
            feerate,
            u32::MAX,
            None,
            outpoints,
            None,
            params.protocol,
        )?;

        // `utill::funding_tx_vsize` is `pub(crate)`: overhead 11 + payment output 43 + P2TR
        // change 43, plus 68 per input. It is the shape `funding_fee_policy_sats` prices, so
        // the two have to stay in step.
        const FUNDING_TX_BASE_VBYTES: u64 = 97;
        const FUNDING_TX_INPUT_VBYTES: u64 = 68;
        let input_count: usize = splits.iter().map(|split| split.utxos.len()).sum();
        let vbytes = splits.len() as u64 * FUNDING_TX_BASE_VBYTES
            + input_count as u64 * FUNDING_TX_INPUT_VBYTES;
        let fee_sats = splits
            .iter()
            .try_fold(0u64, |acc, split| {
                funding_fee_policy_sats(split.utxos.len(), u32::MAX, feerate)
                    .and_then(|fee| acc.checked_add(fee))
            })
            .ok_or_else(price_err)?;

        Ok(SwapFundingEstimateDto {
            input_count,
            vbytes,
            fee_sats,
            outgoing_utxo_count: input_count,
            // The most the last leg can carry: each hop re-plans against its own pool and may
            // commit to fewer, and every later hop inherits that smaller count.
            incoming_utxo_count: params.tx_count as usize,
            route_mining_fee_per_router_sats,
            sweep_fee_sats: params.tx_count as u64 * sweep_per_contract_sats,
            // The wallet sweeps every incoming contract to a fresh P2TR address; only the
            // script type matters to the dust limit, so any 32-byte program prices it.
            receive_dust_sats: ScriptBuf::new_witness_program(
                &WitnessProgram::new(WitnessVersion::V1, &[0; 32]).map_err(AppError::internal)?,
            )
            .minimal_non_dust()
            .to_sat(),
        })
    })
    .await
    .map_err(AppError::internal)?
}

/// The crate rejects an out-of-range split count at prepare time, which is after the user has
/// waited through maker discovery; refusing it here keeps the message specific and immediate.
fn validate_tx_count(count: u32) -> Result<u32, AppError> {
    if count == 0 || count > MAX_TX_COUNT {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("splitCount must be between 1 and {MAX_TX_COUNT}"),
        ));
    }
    Ok(count)
}

/// Phase 1: maker discovery + negotiation, no funds committed. Summary is
/// for a confirmation screen before calling start_swap.
pub async fn prepare_swap(
    instance: &TakerInstance,
    request: SwapRequest,
) -> Result<SwapSummaryDto, AppError> {
    if let Some(active) = instance.active_swap.lock()?.as_ref() {
        if active.phase == SwapLifecycle::Running {
            return Err(AppError::swap_in_progress());
        }
    }
    if request.router_count == 0 {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "routerCount must be at least 1",
        ));
    }

    let protocol = match request.protocol {
        ProtocolVersionDto::Legacy => ProtocolVersion::Legacy,
        ProtocolVersionDto::Taproot => ProtocolVersion::Taproot,
    };
    let mut params = SwapParams::new(
        protocol,
        Amount::from_sat(request.amount_sats),
        request.router_count,
    )
    .with_feerate(validate_feerate(request.fee_rate)?);
    if let Some(outpoints) = request.outpoints {
        let converted = outpoints
            .into_iter()
            .map(|o| -> Result<OutPoint, AppError> {
                let txid = Txid::from_str(&o.txid)
                    .map_err(|e| AppError::new(ErrorCode::InvalidInput, e.to_string()))?;
                Ok(OutPoint::new(txid, o.vout))
            })
            .collect::<Result<Vec<_>, _>>()?;
        params = params.with_utxos(converted);
    }
    if let Some(preferred) = request.preferred_routers {
        params = params.with_preferred_makers(preferred);
    }
    if let Some(count) = request.tx_count {
        params = params.with_tx_count(validate_tx_count(count)?);
    }
    if let Some(address) = request.payment_address {
        // Parsed here, checked against the wallet's own network inside `prepare_swap`.
        let parsed = Address::from_str(address.trim())
            .map_err(|e| AppError::new(ErrorCode::InvalidInput, e.to_string()))?;
        params = params.with_payment_address(parsed);
    }

    let taker = instance.taker.clone();
    let log_dir = instance.data_dir.clone();
    let summary = tokio::task::spawn_blocking(move || -> Result<SwapSummary, AppError> {
        let _log = crate::logging::wallet_scope(log_dir);
        let mut guard = try_lock_taker(&taker)?;
        let taker = guard.as_mut().ok_or_else(AppError::not_initialized)?;
        Ok(taker.prepare_swap(params)?)
    })
    .await
    .map_err(AppError::internal)??;

    let dto = to_summary_dto(&summary);
    *instance.active_swap.lock()? = Some(ActiveSwap {
        swap_id: summary.swap_id,
        summary: dto.clone(),
        phase: SwapLifecycle::Prepared,
        backend_fingerprint: crate::ops::chain_backend::fingerprint(
            &instance.chain_backend,
            Some(instance.socks_port),
        ),
        started_at: None,
        error: None,
        outgoing: Default::default(),
        funding_waits: Default::default(),
    });
    Ok(dto)
}

/// Backs out of a prepared swap. Nothing is funded before `start_swap`; the crate keeps its
/// Negotiated record until the next `Taker::init` removes it, and the reports hide it until then.
pub fn cancel_swap(instance: &TakerInstance, swap_id: String) -> Result<(), AppError> {
    let mut guard = instance.active_swap.lock()?;
    if guard
        .as_ref()
        .is_some_and(|a| a.swap_id == swap_id && a.phase == SwapLifecycle::Prepared)
    {
        *guard = None;
    }
    Ok(())
}

/// Progress for a `prepare_swap` still in flight.
///
/// Never touches the taker mutex — `prepare_swap` is holding it — so this is safe to poll
/// while preparation blocks. `since` is the second the caller started preparing, which is what
/// separates this preparation's record from an older incomplete swap's.
pub async fn get_swap_preparation(
    taker: &TakerInstance,
    since: u64,
) -> Result<Option<SwapPreparationDto>, AppError> {
    let data_dir = taker.data_dir.clone();
    tokio::task::spawn_blocking(move || -> Result<Option<SwapPreparationDto>, AppError> {
        let tracker = SwapTracker::load_or_create(&data_dir)?;
        Ok(tracker
            .incomplete_swaps()
            .into_iter()
            .filter(|r| r.updated_at >= since && r.phase <= SwapPhase::Negotiated)
            .max_by_key(|r| r.updated_at)
            .map(|r| SwapPreparationDto {
                phase: tracker_phase_label(r.phase).to_string(),
                router_count: r.maker_count,
                negotiated_count: r.makers.iter().filter(|m| m.negotiated).count(),
            }))
    })
    .await
    .map_err(AppError::internal)?
}

/// Phase 2: commits funds, can run for hours — dedicated thread, not
/// spawn_blocking. Result via swap://finished / swap://failed events.
pub async fn start_swap(
    state: &Arc<AppState>,
    instance: &Arc<TakerInstance>,
    swap_id: String,
) -> Result<(), AppError> {
    let _operation = SensitiveOperationGuard::acquire(
        &state.sensitive_operation_active,
        SensitiveOperation::StartSwap,
    )?;
    // Checked before the preflight below, which is a network round trip: a stale or missing
    // swap id should fail immediately rather than after one.
    {
        let guard = instance.active_swap.lock()?;
        match guard.as_ref() {
            Some(active)
                if active.swap_id == swap_id && active.phase == SwapLifecycle::Prepared => {}
            Some(active) if active.phase == SwapLifecycle::Running => {
                return Err(AppError::swap_in_progress())
            }
            _ => {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "no prepared swap with this id — call prepare_swap first",
                ))
            }
        }
    }
    let preflight_fingerprint = crate::ops::chain_backend::preflight_active(instance).await?;
    let expected_fingerprint = instance
        .active_swap
        .lock()?
        .as_ref()
        .map(|active| active.backend_fingerprint.clone())
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "prepared swap disappeared"))?;
    if preflight_fingerprint != expected_fingerprint {
        return Err(AppError::new(
            ErrorCode::BackendRouteChanged,
            "active backend route changed after swap preparation",
        ));
    }
    {
        let mut guard = instance.active_swap.lock()?;
        match guard.as_mut() {
            Some(active)
                if active.swap_id == swap_id && active.phase == SwapLifecycle::Prepared =>
            {
                if active.backend_fingerprint != preflight_fingerprint {
                    return Err(AppError::new(
                        ErrorCode::BackendRouteChanged,
                        "backend route changed while awaiting swap approval",
                    ));
                }
                active.phase = SwapLifecycle::Running;
                active.started_at = Some(SystemTime::now());
            }
            Some(active) if active.phase == SwapLifecycle::Running => {
                return Err(AppError::swap_in_progress())
            }
            _ => {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "no prepared swap with this id — call prepare_swap first",
                ))
            }
        }
    }

    let funding_waits = instance
        .active_swap
        .lock()?
        .as_ref()
        .map(|active| Arc::clone(&active.funding_waits))
        .unwrap_or_default();
    let taker = instance.taker.clone();
    // The thread outlives this request, so it owns handles rather than borrowing them.
    let swap_state = Arc::clone(state);
    let swap_instance = Arc::clone(instance);
    std::thread::spawn(move || {
        let _log = crate::logging::wallet_scope(swap_instance.data_dir.clone());
        let _waits = crate::logging::watch_funding_waits(funding_waits);
        let result = {
            let mut guard = match taker.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            match guard.as_mut() {
                Some(taker) => taker.start_swap(&swap_id),
                None => return, // taker dropped (app shutting down) mid-swap
            }
        };

        let app_state = &swap_state;
        let wallet = swap_instance.data_dir.as_path();
        let mut active_guard = match swap_instance.active_swap.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        match result {
            // Crate already persists the report to <wallet>_swap_report.json.
            Ok(_report) => {
                if let Some(active) = active_guard.as_mut() {
                    active.phase = SwapLifecycle::Finished;
                }
                app_state
                    .events
                    .publish_for(wallet, AppEvent::SwapFinished(swap_id.clone()));
            }
            Err(e) => {
                let app_err = AppError::from(e);
                // Past FundsBroadcast the crate has already spawned its recovery loop, so this is
                // a handoff rather than a failure: the swap slot is released outright — recovery
                // has its own page and its own disk-backed status, and leaving it parked here
                // would keep the Swap page pinned to a swap that is over.
                if recovery_started(wallet, &swap_id) {
                    *active_guard = None;
                    app_state
                        .events
                        .publish_for(wallet, AppEvent::SwapRecovering(RecoveryHandoff {
                            swap_id: swap_id.clone(),
                            reason: app_err.message.clone(),
                        }));
                } else {
                    if let Some(active) = active_guard.as_mut() {
                        active.phase = SwapLifecycle::Failed;
                        active.error = Some(app_err.message.clone());
                    }
                    app_state
                        .events
                        .publish_for(wallet, AppEvent::SwapFailed(app_err));
                }
            }
        }
        drop(active_guard);
        // Settled either way, and recovery does not keep a wallet open: if every session left
        // while this ran, the wallet goes now, and its recovery resumes at the next unlock.
        crate::ops::taker_wallet::release_if_unused(app_state, wallet);
    });

    Ok(())
}

pub fn get_swap_progress(
    taker: &TakerInstance,
) -> Result<Option<SwapProgressDto>, AppError> {
    let guard = taker.active_swap.lock()?;
    // Only Running is worth reconciling after a remount — a terminal phase is stale by definition
    // and would otherwise resurrect the last outcome indefinitely. Recovery is deliberately not
    // here: it has its own page and its own disk-backed status, and reporting it as swap progress
    // would drag a remounted Swap page back into a progress view it has already been released from.
    Ok(guard
        .as_ref()
        .filter(|a| matches!(a.phase, SwapLifecycle::Running))
        .map(|active| SwapProgressDto {
            swap_id: active.swap_id.clone(),
            summary: active.summary.clone(),
            started_at: active
                .started_at
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
        }))
}

/// Live per-maker detail, straight off `<data_dir>/swap_tracker.cbor` — intended to be polled
/// every couple seconds while a swap is running, same cadence as the old Electron app's disk-read
/// poll.
///
/// `swap_id` is optional so the recovery page can ask for a swap the app is no longer running:
/// recovery releases the active-swap slot, and the route it was taking is still what the recovery
/// view draws.
pub async fn get_swap_tracker(
    taker: &TakerInstance,
    swap_id: Option<String>,
) -> Result<Option<SwapTrackerDto>, AppError> {
    let (swap_id, outgoing, funding_waits) = {
        let active = taker.active_swap.lock()?;
        let active = active.as_ref();
        let id = match swap_id {
            Some(id) => id,
            None => match active {
                Some(a) => a.swap_id.clone(),
                None => return Ok(None),
            },
        };
        let active = active.filter(|a| a.swap_id == id);
        let outgoing = active.map(|a| Arc::clone(&a.outgoing));
        let funding_waits = active
            .and_then(|a| a.funding_waits.lock().ok().map(|waits| waits.legs.clone()))
            .unwrap_or_default();
        (id, outgoing, funding_waits)
    };
    let data_dir = taker.data_dir.clone();
    let chain = (taker.chain_backend.clone(), taker.wallet_name.clone(), taker.socks_port);

    tokio::task::spawn_blocking(move || -> Result<Option<SwapTrackerDto>, AppError> {
        let tracker = SwapTracker::load_or_create(&data_dir)?;
        let Some(record) = tracker.get_record(&swap_id) else {
            return Ok(None);
        };
        let mut dto = to_tracker_dto(record);
        dto.funding_wait_txids = funding_waits;
        let Some(outgoing) = outgoing else {
            return Ok(Some(dto));
        };
        dto.outgoing_confirmed = outgoing.confirmed.load(Ordering::Relaxed);
        // Taproot's outgoing contract txs are our funding txs, so the chain backend has their
        // confirmations. Legacy's are pre-signed and never broadcast; it reports this itself
        // through `prev_funding_confirmed`. The answer lands on a later poll: a refreshed page
        // must not wait out an Electrum round trip before it can draw the route at all.
        if !dto.outgoing_confirmed
            && record.phase == SwapPhase::FundsBroadcast
            && !record.outgoing_contract_txids.is_empty()
            && !outgoing.checking.swap(true, Ordering::AcqRel)
        {
            let txids = record.outgoing_contract_txids.clone();
            std::thread::spawn(move || {
                // Its own connection rather than the wallet's: the swap holds the wallet for most
                // of the wait, and a `try_read` on it was refused for whole hops at a time.
                let (config, wallet_name, socks_port) = chain;
                let confirmed = crate::ops::chain_backend::with_chain(
                    &config,
                    &wallet_name,
                    Some(socks_port),
                    |chain| {
                        // A failed read reaches `with_chain`, which recycles the connection;
                        // only a txid the backend places in no block answers "not yet".
                        for txid in &txids {
                            if chain.tx_block_height(txid)?.is_none() {
                                return Ok(false);
                            }
                        }
                        Ok(true)
                    },
                )
                .unwrap_or(false);
                if confirmed {
                    outgoing.confirmed.store(true, Ordering::Relaxed);
                }
                outgoing.checking.store(false, Ordering::Release);
            });
        }
        Ok(Some(dto))
    })
    .await
    .map_err(AppError::internal)?
}

/// Starts recovery for contracts the automatic path never picked up — a loop that failed to spawn,
/// or a crash before `recover_active_swap` ran.
///
/// Refuses while a loop is already running. `recover_active_swap` replaces the taker's
/// `recovery_loop`, and dropping the old one joins its thread mid-pass, which would hold the taker
/// mutex for the length of a chain round trip for no gain — the running loop already retries every
/// minute.
pub async fn recover_swap(instance: &TakerInstance) -> Result<(), AppError> {
    let taker = instance.taker.clone();
    let log_dir = instance.data_dir.clone();
    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
        let _log = crate::logging::wallet_scope(log_dir);
        let mut guard = try_lock_taker(&taker)?;
        let taker = guard.as_mut().ok_or_else(AppError::not_initialized)?;
        if !taker.is_recovery_complete() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "recovery is already running",
            ));
        }
        Ok(taker.recover_active_swap()?)
    })
    .await
    .map_err(AppError::internal)?
}

/// The taker's own refund delay, in blocks: `REFUND_LOCKTIME_BASE + REFUND_LOCKTIME_STEP *
/// maker_count` from `openswap::taker::api` (20 and 20 at the time of writing — both `pub(crate)`
/// there, so they are mirrored rather than imported, and both change under the crate's
/// `integration-test` feature). The taker's is one step beyond the first maker's on purpose: its
/// refund must be the last to mature so every maker can act first.
fn refund_locktime_blocks(router_count: usize) -> u32 {
    20 + 20 * router_count as u32
}

/// Whether the crate put this swap into recovery rather than aborting it cleanly.
///
/// The tracker record is the authority: the crate removes it entirely for a failure before
/// `FundsBroadcast` (nothing on-chain, nothing to recover) and marks it `Failed` once it has
/// spawned a recovery loop.
fn recovery_started(data_dir: &std::path::Path, swap_id: &str) -> bool {
    let Ok(tracker) = SwapTracker::load_or_create(data_dir) else {
        return false;
    };
    tracker
        .get_record(swap_id)
        .is_some_and(|r| r.phase >= SwapPhase::FundsBroadcast)
}

/// Every swap whose funds recovery has not finished reclaiming, newest first.
///
/// The crate runs one recovery loop over all of them at once rather than one per swap, so this
/// lists the swaps *inside* that recovery — which is what a user who failed several swaps is
/// actually looking for — not several independent recoveries.
///
/// Contracts are deliberately absent: `list_live_contract_spend_info` reports the wallet's live
/// contracts as one pool, and the crate keeps the swap-to-swapcoin maps `pub(crate)`, so there is
/// no honest way to split them per swap from out here. The detail view shows the pool.
pub async fn list_recoveries(
    taker: &TakerInstance,
) -> Result<Vec<RecoverySummary>, AppError> {
    let data_dir = taker.data_dir.clone();
    let wallet_name = taker.wallet_name.clone();

    tokio::task::spawn_blocking(move || -> Result<Vec<RecoverySummary>, AppError> {
        // Every record, so a finished recovery stays listed: `incomplete_swaps` drops it. The
        // crate's own list is added back so a tracker this read can't decode still shows the
        // recoveries that are running.
        let mut records: HashMap<String, SwapRecord> = tracker_records(&data_dir)
            .into_iter()
            .map(|r| (r.swap_id.clone(), r))
            .collect();
        let tracker = SwapTracker::load_or_create(&data_dir)?;
        for r in tracker.incomplete_swaps() {
            records.entry(r.swap_id.clone()).or_insert_with(|| r.clone());
        }
        let reports = recovery_reports(&data_dir, &wallet_name);
        let mut rows: Vec<RecoverySummary> = records
            .into_values()
            .filter(|r| r.phase == SwapPhase::Failed || r.recovery.phase != RecoveryPhase::NotStarted)
            .map(|r| {
                let mine = reports.iter().filter(|report| report.swap_id == r.swap_id);
                let mut recovery_types: Vec<String> =
                    mine.clone().map(|report| report.recovery_type.clone()).collect();
                recovery_types.sort_unstable();
                recovery_types.dedup();
                RecoverySummary {
                    phase: recovery_phase_label(r.recovery.phase).to_string(),
                    failed_at_phase: r.failed_at_phase.map(|p| tracker_phase_label(p).to_string()),
                    router_count: r.maker_count,
                    send_amount_sats: r.send_amount_sat,
                    resolved_count: r.recovery.incoming.len() + r.recovery.outgoing.len(),
                    active: r.recovery.phase < RecoveryPhase::CleanedUp,
                    updated_at: r.updated_at,
                    recovery_types,
                    recovery_txids: mine.flat_map(|report| report.recovery_txids.clone()).collect(),
                    failure_reason: r.failure_reason,
                    swap_id: r.swap_id,
                }
            })
            .collect();
        rows.sort_unstable_by_key(|r| std::cmp::Reverse(r.updated_at));
        Ok(rows)
    })
    .await
    .map_err(AppError::internal)?
}

/// Recovery state.
///
/// Built from the wallet's live contract UTXOs first and the tracker second, because that is the
/// only order that describes a recovery in progress: the tracker records an outcome per contract
/// *after* it is claimed, so during the wait — which is the whole of it — its outcome vectors are
/// empty. The UTXOs are what actually hold the money.
///
/// Never takes the taker mutex: a running swap holds that for hours, and this is exactly when the
/// user most needs to see whether an earlier swap's funds have come back.
pub async fn get_recovery_status(
    taker: &TakerInstance,
    swap_id: Option<String>,
) -> Result<RecoveryStatus, AppError> {
    // `RecoveryPhase::NotStarted` remains set while the loop scans, waits for confirmations and
    // broadcasts claims. Inspect the worker separately without waiting behind an active swap.
    let recovery_running = match taker.taker.try_lock() {
        Ok(guard) => guard.as_ref().map(|taker| !taker.is_recovery_complete()),
        Err(std::sync::TryLockError::WouldBlock) => None,
        Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned
            .into_inner()
            .as_ref()
            .map(|taker| !taker.is_recovery_complete()),
    };
    let wallet = taker.wallet.clone();
    let data_dir = taker.data_dir.clone();
    // A healthy swap in flight holds its funds in contracts too, so a live contract UTXO is not
    // on its own evidence of recovery.
    let swap_running = taker.swap_running();
    let (config, wallet_name, socks_port) =
        (taker.chain_backend.clone(), taker.wallet_name.clone(), taker.socks_port);

    tokio::task::spawn_blocking(move || -> Result<RecoveryStatus, AppError> {
        let (live, locked_sats) = {
            let guard = wallet.read()?;
            let live = guard.list_live_contract_spend_info();
            let locked_sats = guard.get_balances()?.contract.to_sat();
            (live, locked_sats)
        };
        let tracker = SwapTracker::load_or_create(&data_dir)?;
        // `incomplete_swaps` already excludes anything cleaned up. With no `swap_id` the newest
        // failed record is the one to report against — the crate recovers all outstanding
        // contracts together rather than per swap, so any of them describes the same recovery.
        let candidates: Vec<_> = tracker
            .incomplete_swaps()
            .into_iter()
            .filter(|r| {
                r.phase == SwapPhase::Failed || r.recovery.phase != RecoveryPhase::NotStarted
            })
            .collect();
        let record = match &swap_id {
            // Looked up directly rather than among the candidates: a recovery that has finished
            // is dropped from `incomplete_swaps`, and reading one back by id is exactly how the
            // history opens a recovery that is already done. Still has to have been a recovery —
            // an ordinary completed swap is not one.
            Some(wanted) => tracker
                .get_record(wanted)
                .filter(|r| {
                    r.phase == SwapPhase::Failed || r.recovery.phase != RecoveryPhase::NotStarted
                })
                .cloned(),
            None => candidates.iter().max_by_key(|r| r.updated_at).copied().cloned(),
        };

        // The crate records a UTXO's confirmations when it first caches it and never updates
        // them, so recovery needs a live height read for its countdown. Do not open an Electrum
        // connection for the common idle case, or for contracts belonging only to a healthy swap.
        let needs_chain = !live.is_empty() && (!candidates.is_empty() || !swap_running);
        // One visit to the connection for the tip and every contract's height together, rather
        // than one per contract: they are all answered off the same socket.
        let heights = needs_chain
            .then(|| {
                crate::ops::chain_backend::with_chain(
                    &config,
                    &wallet_name,
                    Some(socks_port),
                    |chain| {
                        let tip = chain.get_block_count()?;
                        // Propagated, not swallowed: a read that failed is not a txid absent
                        // from every block, and reporting it as one would show a contract as
                        // unconfirmed with its whole timelock still to run. A failure here
                        // leaves `heights` unset, and every contract keeps its cached count.
                        let seen = live
                            .iter()
                            .map(|(utxo, _)| {
                                chain.tx_block_height(&utxo.txid).map(|at| (utxo.txid, at))
                            })
                            .collect::<Result<std::collections::HashMap<Txid, Option<u64>>, _>>()?;
                        Ok((tip, seen))
                    },
                )
                .ok()
            })
            .flatten();

        // The taker's own refund delay, taken across *every* unfinished swap rather than the
        // selected one: `pending` below is the wallet's whole contract pool, which cannot be
        // attributed per swap, so a single swap's delay applied to all of it would count a
        // longer-locked contract down to zero early. The longest is the only safe bound — it
        // can overstate the wait when swaps have different router counts, never understate it.
        // Without a record there is no router count at all, so no countdown is offered.
        let offset = candidates
            .iter()
            .map(|r| refund_locktime_blocks(r.maker_count))
            .max();
        // Once a swap's incoming coins are claimed, its outgoing contract is the first router's
        // payment: the crate holds the refund back for that router's hashlock claim, so counting
        // it as this wallet's money being reclaimed would be wrong twice over.
        let router_owed: std::collections::HashSet<Txid> = candidates
            .iter()
            .filter(|r| swap_received(r))
            .flat_map(|r| r.outgoing_contract_txids.iter().copied())
            .collect();

        let mut pending: Vec<RecoveryContractDto> = live
            .iter()
            .map(|(utxo, info)| {
                let timelocked = matches!(info, UTXOSpendInfo::TimelockContract { .. });
                let confirmations = match &heights {
                    // A txid the backend does not place in a block is in the mempool, which is
                    // exactly the case the wallet's cached count gets wrong.
                    Some((tip, seen)) => match seen.get(&utxo.txid) {
                        Some(Some(height)) => (tip + 1).saturating_sub(*height) as u32,
                        Some(None) => 0,
                        None => utxo.confirmations,
                    },
                    None => utxo.confirmations,
                };
                let router_owed = timelocked && router_owed.contains(&utxo.txid);
                let lock_blocks = offset.filter(|_| timelocked && !router_owed);
                RecoveryContractDto {
                    outpoint: crate::types::Outpoint {
                        txid: utxo.txid.to_string(),
                        vout: utxo.vout,
                    },
                    amount_sats: utxo.amount.to_sat(),
                    claim_path: if timelocked { "timelock" } else { "hashlock" }.to_string(),
                    confirmations,
                    router_owed,
                    blocks_remaining: lock_blocks.map(|lock| lock.saturating_sub(confirmations)),
                    lock_blocks,
                }
            })
            .collect();
        // Longest wait last, so the countdown headline and the list agree on what is holding
        // things up.
        pending.sort_by_key(|c| c.blocks_remaining.unwrap_or(0));

        let blocks_remaining = pending
            .iter()
            .filter_map(|c| c.blocks_remaining)
            .max()
            .filter(|blocks| *blocks > 0);
        let router_owed_sats: u64 =
            pending.iter().filter(|c| c.router_owed).map(|c| c.amount_sats).sum();
        let locked_sats = locked_sats.saturating_sub(router_owed_sats);

        let Some(record) = record else {
            return Ok(RecoveryStatus {
                // Contracts with no failed record behind them are only strandable once nothing
                // is running — that is the crashed-before-persisting case.
                active: !swap_running && !live.is_empty(),
                recovery_running,
                swap_id: None,
                phase: recovery_phase_label(RecoveryPhase::NotStarted).to_string(),
                failure_reason: None,
                failed_at_phase: None,
                router_count: 0,
                send_amount_sats: 0,
                swap_received: false,
                pending,
                resolved: Vec::new(),
                blocks_remaining,
                locked_sats,
                router_owed_sats,
                updated_at: None,
            });
        };

        let resolved: Vec<RecoveredContractDto> = record
            .recovery
            .incoming
            .iter()
            .map(|o| ("incoming", o))
            .chain(record.recovery.outgoing.iter().map(|o| ("outgoing", o)))
            .map(|(leg, o)| RecoveredContractDto {
                contract_txid: o.contract_txid.to_string(),
                leg: leg.to_string(),
                resolution: resolution_label(&o.resolution).to_string(),
                spending_txid: o.spending_txid.map(|t| t.to_string()),
            })
            .collect();

        // `RecoveryPhase::CleanedUp` is the authority on being finished, not an empty contract
        // list: an incoming contract whose preimage was never stamped is invisible to
        // `list_live_contract_spend_info`, so a zero count can precede the real end of recovery.
        // Deliberately not gated on a running swap: a failed record is direct evidence, and an
        // earlier swap's recovery has to stay visible while a new swap runs.
        let active = record.recovery.phase < RecoveryPhase::CleanedUp;

        Ok(RecoveryStatus {
            active,
            recovery_running,
            swap_id: Some(record.swap_id.clone()),
            phase: recovery_phase_label(record.recovery.phase).to_string(),
            failure_reason: record.failure_reason.clone(),
            failed_at_phase: record.failed_at_phase.map(|p| tracker_phase_label(p).to_string()),
            router_count: record.maker_count,
            send_amount_sats: record.send_amount_sat,
            swap_received: swap_received(&record),
            pending,
            resolved,
            blocks_remaining,
            locked_sats,
            router_owed_sats,
            updated_at: Some(record.updated_at),
        })
    })
    .await
    .map_err(AppError::internal)?
}

/// Mirrors the crate's own test for holding a refund back (`incoming_claimed`, `pub(crate)`
/// there): the tracker records each incoming claim as soon as its sweep lands.
fn swap_received(record: &SwapRecord) -> bool {
    record.recovery.incoming.iter().any(|o| {
        matches!(o.resolution, ContractResolution::Hashlock | ContractResolution::KeyPath)
    })
}

fn resolution_label(r: &ContractResolution) -> &'static str {
    match r {
        ContractResolution::Hashlock => "hashlock",
        ContractResolution::Timelock => "timelock",
        ContractResolution::KeyPath => "key_path",
        ContractResolution::Discarded => "discarded",
        ContractResolution::Unresolved => "unresolved",
    }
}

fn recovery_phase_label(p: RecoveryPhase) -> &'static str {
    match p {
        RecoveryPhase::NotStarted => "not_started",
        RecoveryPhase::PreimageStamped => "preimage_stamped",
        RecoveryPhase::SwapcoinsPersisted => "swapcoins_persisted",
        RecoveryPhase::IncomingRecovered => "incoming_recovered",
        RecoveryPhase::OutgoingRecovered => "outgoing_recovered",
        RecoveryPhase::CleanedUp => "cleaned_up",
    }
}
