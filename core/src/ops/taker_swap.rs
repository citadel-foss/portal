//! Swap execution: two-phase prepare/start, live progress, recovery.
//!
//! openswap keeps a swap's progress private, so live per-router progress comes from Portal's own
//! swap journal (`ops::taker_journal`), folded from the crate's log lines on the swap's thread.

use std::sync::{Arc, Mutex};

use std::str::FromStr;
use std::time::SystemTime;

use openswap::bitcoin::{Address, Amount, OutPoint, ScriptBuf, Txid, WitnessProgram, WitnessVersion};
use openswap::protocol::ProtocolVersion;
use openswap::taker::{SwapParams, SwapSummary};
use openswap::utill::{
    funding_fee_policy_sats, sweep_fee_policy_sats, MAX_TX_COUNT, MIN_RELAY_FEE_RATE,
};
use openswap::wallet::{Blockchain, RecoveryReport, UTXOSpendInfo};
use crate::events::AppEvent;
use crate::ops::taker_journal::{self, SwapEntry, SwapOutcome};

use crate::error::{AppError, ErrorCode};
use crate::security::operation::{SensitiveOperation, SensitiveOperationGuard};
use crate::state::{try_lock_taker, ActiveSwap, AppState, SwapLifecycle, TakerInstance};
use crate::types::{
    PaymentQuoteDto, ProtocolVersionDto, RecoveredContractDto, RecoveryContractDto,
    RecoveryHandoff, RecoveryStatus, RecoverySummary, RouterFeeInfoDto, SwapFundingEstimateDto,
    SwapPhaseDto, SwapPreparationDto, SwapProgressDto, SwapRequest, SwapSummaryDto,
    SwapTrackerDto,
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

        let input_count: usize = splits.iter().map(|split| split.utxos.len()).sum();
        let priced_at = |rate: f64| {
            splits
                .iter()
                .try_fold(0u64, |acc, split| {
                    funding_fee_policy_sats(split.utxos.len(), u32::MAX, rate)
                        .and_then(|fee| acc.checked_add(fee))
                })
                .ok_or_else(price_err)
        };
        // `utill::funding_tx_vsize` is `pub(crate)`; at one sat per vbyte its price is the size.
        let vbytes = priced_at(1.0)?;
        let fee_sats = priced_at(feerate)?;

        Ok(SwapFundingEstimateDto {
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
    let preparation = Arc::clone(&instance.preparation);
    let summary = tokio::task::spawn_blocking(move || -> Result<SwapSummary, AppError> {
        let _log = crate::logging::wallet_scope(log_dir);
        let mut guard = try_lock_taker(&taker)?;
        let taker = guard.as_mut().ok_or_else(AppError::not_initialized)?;
        // Only once this call holds the taker: a refused one must not wipe the progress of the
        // preparation that does.
        *preparation.lock()? = None;
        let watched = Arc::clone(&preparation);
        let watch = crate::logging::watch_thread_logs(move |_, line| {
            let Ok(mut preparation) = watched.lock() else {
                return;
            };
            if let Some(count) = line
                .strip_prefix("Selected ")
                .and_then(|rest| rest.split(' ').next())
                .and_then(|n| n.parse().ok())
            {
                *preparation = Some(SwapPreparationDto {
                    router_count: count,
                    negotiated_count: 0,
                });
            } else if let (Some(prep), Some(index)) = (
                preparation.as_mut(),
                line.strip_prefix("Maker ")
                    .filter(|_| line.ends_with(" accepted swap with tweakable point"))
                    .and_then(|rest| rest.split(' ').next())
                    .and_then(|n| n.parse::<usize>().ok()),
            ) {
                prep.negotiated_count = prep.negotiated_count.max(index + 1);
            }
        });
        let prepared = taker.prepare_swap(params);
        drop(watch);
        *preparation.lock()? = None;
        Ok(prepared?)
    })
    .await
    .map_err(AppError::internal)??;

    let dto = to_summary_dto(&summary);
    *instance.active_swap.lock()? = Some(ActiveSwap {
        swap_id: summary.swap_id,
        journal: Arc::new(Mutex::new(SwapEntry::start(&dto))),
        summary: dto.clone(),
        phase: SwapLifecycle::Prepared,
        backend_fingerprint: crate::ops::chain_backend::fingerprint(
            &instance.chain_backend,
            Some(instance.socks_port),
        ),
        started_at: None,
        error: None,
        funding_waits: Default::default(),
    });
    Ok(dto)
}

/// Backs out of a prepared swap. Nothing is funded before `start_swap`, and the crate replaces a
/// prepared swap that never started at the next `prepare_swap`.
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

/// Progress for a `prepare_swap` still in flight. Never touches the taker mutex — `prepare_swap`
/// is holding it — so this is safe to poll while preparation blocks.
pub fn get_swap_preparation(taker: &TakerInstance) -> Result<Option<SwapPreparationDto>, AppError> {
    Ok(taker.preparation.lock()?.clone())
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
    let (funding_waits, journal) = {
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
                // Before the slot turns Running: nothing would clear a Running slot whose swap
                // thread never started.
                taker_journal::save(
                    &instance.data_dir,
                    &instance.journal_key,
                    &*active.journal.lock()?,
                )?;
                active.phase = SwapLifecycle::Running;
                active.started_at = Some(SystemTime::now());
                (Arc::clone(&active.funding_waits), Arc::clone(&active.journal))
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
    };
    let taker = instance.taker.clone();
    // The thread outlives this request, so it owns handles rather than borrowing them.
    let swap_state = Arc::clone(state);
    let swap_instance = Arc::clone(instance);
    std::thread::spawn(move || {
        let data_dir = swap_instance.data_dir.clone();
        let _log = crate::logging::wallet_scope(data_dir.clone());
        let key = swap_instance.journal_key.clone();
        let watched = (Arc::clone(&journal), data_dir.clone(), key.clone());
        let watch = crate::logging::watch_thread_logs(move |target, line| {
            if let Some(txid) = crate::logging::funding_wait_txid(target, line) {
                if let Ok(mut waits) = funding_waits.lock() {
                    waits.observe(txid);
                }
            }
            let (journal, data_dir, key) = &watched;
            // Saved outside the lock: the tracker poll reads this entry while the file is written.
            let changed = journal.lock().ok().and_then(|mut entry| {
                entry.observe(line).then(|| {
                    entry.updated_at = taker_journal::now_secs();
                    entry.clone()
                })
            });
            if let Some(entry) = changed {
                let _ = taker_journal::save(data_dir, key, &entry);
            }
        });
        let (result, coins_committed) = {
            let mut guard = match taker.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            let Some(taker) = guard.as_mut() else {
                return; // taker dropped (app shutting down) mid-swap
            };
            let outgoing_coins = |taker: &openswap::taker::Taker| {
                taker.get_wallet().read().map_or(0, |w| w.get_outgoing_swapcoins_count())
            };
            let before = outgoing_coins(taker);
            let result = taker.start_swap(&swap_id);
            // A failed swap the crate aborted takes its coins back out of the wallet; one it
            // handed to recovery leaves them in. Recovery of older swaps only ever removes coins.
            (result, outgoing_coins(taker) > before)
        };
        // Before the journal lock below: anything logged while a watch is live re-enters it.
        drop(watch);
        let result = result.map(drop).map_err(AppError::from);
        let outcome = match journal.lock() {
            Ok(mut entry) => {
                entry.recovery_started |= coins_committed;
                entry.settle(result.clone().map_err(|e| e.message));
                if let Err(e) = taker_journal::save(&data_dir, &key, &entry) {
                    log::warn!("could not save the swap journal: {e:?}");
                }
                entry.outcome
            }
            Err(_) => SwapOutcome::Recovering,
        };

        let app_state = &swap_state;
        let wallet = swap_instance.data_dir.as_path();
        let mut active_guard = match swap_instance.active_swap.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        match result {
            // The crate has already saved its report into the wallet.
            Ok(()) => {
                if let Some(active) = active_guard.as_mut() {
                    active.phase = SwapLifecycle::Finished;
                }
                app_state
                    .events
                    .publish_for(wallet, AppEvent::SwapFinished(swap_id.clone()));
            }
            Err(app_err) => {
                // Once funds are on-chain the crate has already started recovering them, so this
                // is a handoff rather than a failure: the swap slot is released outright —
                // recovery has its own page and its own journal-backed status, and leaving it
                // parked here would keep the Swap page pinned to a swap that is over.
                if outcome == SwapOutcome::Recovering {
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

/// Live per-router detail, polled every couple of seconds while a swap runs.
///
/// `swap_id` is optional so the recovery page can ask for a swap the app is no longer running:
/// recovery releases the active-swap slot, and the route it was taking is still what the recovery
/// view draws.
pub async fn get_swap_tracker(
    taker: &TakerInstance,
    swap_id: Option<String>,
) -> Result<Option<SwapTrackerDto>, AppError> {
    let (swap_id, live) = {
        let active = taker.active_swap.lock()?;
        let active = active.as_ref();
        let id = match swap_id {
            Some(id) => id,
            None => match active {
                Some(a) => a.swap_id.clone(),
                None => return Ok(None),
            },
        };
        let live = active
            .filter(|a| a.swap_id == id)
            .map(|a| (Arc::clone(&a.journal), Arc::clone(&a.funding_waits)));
        (id, live)
    };
    if let Some((journal, funding_waits)) = live {
        let legs = funding_waits.lock()?.legs.clone();
        return Ok(Some(journal.lock()?.to_tracker_dto(legs)));
    }
    let (data_dir, key) = (taker.data_dir.clone(), taker.journal_key.clone());
    tokio::task::spawn_blocking(move || {
        taker_journal::load_for_view(&data_dir, &key)
            .remove(&swap_id)
            .map(|entry| entry.to_tracker_dto(Vec::new()))
    })
    .await
    .map_err(AppError::internal)
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

/// "cleaned_up" once recovery has nothing left of a swap, "not_started" until then.
fn recovery_phase(outcome: SwapOutcome) -> &'static str {
    if outcome == SwapOutcome::Recovered {
        "cleaned_up"
    } else {
        "not_started"
    }
}

/// Every swap handed to recovery, newest first.
///
/// The crate runs one recovery loop over all of them at once rather than one per swap, so this
/// lists the swaps *inside* that recovery — which is what a user who failed several swaps is
/// actually looking for — not several independent recoveries.
///
/// Contracts are deliberately absent: `list_live_contract_spend_info` reports the wallet's live
/// contracts as one pool, and the crate keeps the swap-to-swapcoin maps private, so there is no
/// honest way to split them per swap from out here. The detail view shows the pool.
pub async fn list_recoveries(
    taker: &TakerInstance,
) -> Result<Vec<RecoverySummary>, AppError> {
    let (data_dir, key) = (taker.data_dir.clone(), taker.journal_key.clone());
    let wallet = taker.wallet.clone();
    let live_swap = taker.running_swap_id();

    tokio::task::spawn_blocking(move || -> Result<Vec<RecoverySummary>, AppError> {
        let reports = wallet.read()?.swap_reports().recovery;
        let mut rows: Vec<RecoverySummary> = taker_journal::load_for_view(&data_dir, &key)
            .into_values()
            .filter_map(|entry| {
                let outcome = entry.effective_outcome(live_swap.as_deref());
                if !matches!(outcome, SwapOutcome::Recovering | SwapOutcome::Recovered) {
                    return None;
                }
                let mine = reports.iter().filter(|report| report.swap_id == entry.swap_id);
                let mut recovery_types: Vec<String> =
                    mine.clone().map(|report| report.recovery_type.clone()).collect();
                recovery_types.sort_unstable();
                recovery_types.dedup();
                let resolved_count = mine.map(|report| report.recovery_txids.len()).sum();
                Some(RecoverySummary {
                    phase: recovery_phase(outcome).to_string(),
                    router_count: entry.routers.len(),
                    send_amount_sats: entry.send_amount_sats,
                    resolved_count,
                    active: outcome == SwapOutcome::Recovering,
                    updated_at: entry.updated_at,
                    recovery_types,
                    failure_reason: entry.failure_reason,
                    swap_id: entry.swap_id,
                })
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
/// Built from the wallet's live contract UTXOs: they are what actually hold the money, for the
/// whole of the wait.
///
/// Never takes the taker mutex: a running swap holds that for hours, and this is exactly when the
/// user most needs to see whether an earlier swap's funds have come back.
pub async fn get_recovery_status(
    taker: &TakerInstance,
    swap_id: Option<String>,
) -> Result<RecoveryStatus, AppError> {
    let recovery_running = match taker.taker.try_lock() {
        Ok(guard) => guard.as_ref().map(|taker| !taker.is_recovery_complete()),
        Err(std::sync::TryLockError::WouldBlock) => None,
        Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned
            .into_inner()
            .as_ref()
            .map(|taker| !taker.is_recovery_complete()),
    };
    let wallet = taker.wallet.clone();
    let (data_dir, key) = (taker.data_dir.clone(), taker.journal_key.clone());
    // A healthy swap in flight holds its funds in contracts too, so a live contract UTXO is not
    // on its own evidence of recovery.
    let live_swap = taker.running_swap_id();
    let swap_running = live_swap.is_some();
    let (config, wallet_name, socks_port) =
        (taker.chain_backend.clone(), taker.wallet_name.clone(), taker.socks_port);

    tokio::task::spawn_blocking(move || -> Result<RecoveryStatus, AppError> {
        let (live, locked_sats, reports) = {
            let guard = wallet.read()?;
            let live = guard.list_live_contract_spend_info();
            let locked_sats = guard.get_balances()?.contract.to_sat();
            (live, locked_sats, guard.swap_reports().recovery)
        };
        // The crate keeps no per-swap recovery state, so a swap counts as recovered once the
        // worker has stopped with nothing left in contracts.
        if recovery_running == Some(false) && live.is_empty() && !swap_running {
            let settled = taker_journal::update_all(&data_dir, &key, |entry| {
                let recovering = entry.effective_outcome(None) == SwapOutcome::Recovering;
                if recovering {
                    entry.outcome = SwapOutcome::Recovered;
                    entry.updated_at = taker_journal::now_secs();
                }
                recovering
            });
            if let Err(e) = settled {
                log::warn!("could not mark finished recoveries in the journal: {}", e.message);
            }
        }
        let journal = taker_journal::load_for_view(&data_dir, &key);
        let outcome = |entry: &SwapEntry| entry.effective_outcome(live_swap.as_deref());
        let candidates: Vec<&SwapEntry> = journal
            .values()
            .filter(|entry| outcome(entry) == SwapOutcome::Recovering)
            .collect();
        let record = match &swap_id {
            // Looked up directly rather than among the candidates, so the history can open a
            // recovery that is already done.
            Some(wanted) => journal.get(wanted).filter(|entry| {
                matches!(outcome(entry), SwapOutcome::Recovering | SwapOutcome::Recovered)
            }),
            None => candidates.iter().max_by_key(|entry| entry.updated_at).copied(),
        };

        // The wallet's cached confirmations are only as fresh as its last sync, so the countdown
        // reads heights live. Do not open an Electrum connection for the common idle case, or for
        // contracts belonging only to a healthy swap.
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
            .map(|entry| refund_locktime_blocks(entry.routers.len()))
            .max();
        // Once every key is handed over, a swap's outgoing contract is the first router's
        // payment: the crate holds the refund back for that router's hashlock claim, so counting
        // it as this wallet's money being reclaimed would be wrong twice over. Only Taproot logs
        // those contracts' txids.
        let router_owed: std::collections::HashSet<Txid> = candidates
            .iter()
            .filter(|entry| swap_received(entry, &reports))
            .flat_map(|entry| entry.outgoing_contract_txids.iter())
            .filter_map(|txid| txid.parse().ok())
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
                // Contracts with no recovering swap behind them are only strandable once nothing
                // is running — that is the crashed-before-journaling case.
                active: !swap_running && !live.is_empty(),
                recovery_running,
                swap_id: None,
                phase: recovery_phase(SwapOutcome::Recovering).to_string(),
                failure_reason: None,
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

        let resolved: Vec<RecoveredContractDto> = reports
            .iter()
            .filter(|report| report.swap_id == record.swap_id)
            .flat_map(|report| {
                let leg = if report.recovery_type == "hashlock" { "incoming" } else { "outgoing" };
                report.recovery_txids.iter().map(move |txid| RecoveredContractDto {
                    txid: txid.clone(),
                    leg: leg.to_string(),
                })
            })
            .collect();
        let outcome = outcome(record);

        Ok(RecoveryStatus {
            active: outcome == SwapOutcome::Recovering,
            recovery_running,
            swap_id: Some(record.swap_id.clone()),
            phase: recovery_phase(outcome).to_string(),
            failure_reason: record.failure_reason.clone(),
            router_count: record.routers.len(),
            send_amount_sats: record.send_amount_sats,
            swap_received: swap_received(record, &reports),
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

/// The route paid this wallet: every key went round before the swap stopped, or recovery has
/// since claimed the incoming coins by hashlock. Either way the crate holds the outgoing refund
/// back for the first router.
fn swap_received(entry: &SwapEntry, reports: &[RecoveryReport]) -> bool {
    entry.reached() >= SwapPhaseDto::PrivkeysForwarded
        || reports
            .iter()
            .any(|r| r.swap_id == entry.swap_id && r.recovery_type == "hashlock")
}
