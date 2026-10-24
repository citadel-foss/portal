//! Serde DTOs crossing the IPC boundary. Mirrored by `src/api/types.ts`.
//! Conventions: camelCase field names, amounts in sats as u64.

/// Which chain data source the wallet is built against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChainBackendKind {
    Electrum,
    CoreRpc,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElectrumBackendDto {
    /// Full endpoint including scheme, e.g. `tcp://host:50001` or `ssl://host:50002`.
    pub url: String,
    /// Route through Tor's SOCKS proxy. An `.onion` URL forces this on regardless —
    /// the crate rejects one with no proxy configured.
    #[serde(default)]
    pub use_tor: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeBackendDto {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub zmq_port: u16,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainBackendConfig {
    pub kind: ChainBackendKind,
    pub electrum: ElectrumBackendDto,
    /// `None` until the user adds their own node; adding one also flips `kind`.
    #[serde(default)]
    pub node: Option<NodeBackendDto>,
}

/// Saved Core RPC settings with the secret replaced by a configured/not-configured flag.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeBackendViewDto {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub zmq_port: u16,
}

/// Secret-safe chain backend configuration returned to the settings UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainBackendView {
    pub kind: ChainBackendKind,
    pub electrum: ElectrumBackendDto,
    pub node: Option<NodeBackendViewDto>,
}

/// The economics a new router starts with, read straight off the protocol crate's own
/// `MakerServerConfig::default()`.
///
/// Served rather than restated in the frontend so there is exactly one source of truth: the
/// app had drifted to ten times core's figures on every one of these, which a UI constant can
/// do silently and a crate bump will never correct.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterDefaultsDto {
    pub fidelity_amount: u64,
    /// The crate's minimum bond amount, when its config check could be asked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_fidelity_amount: Option<u64>,
    pub fidelity_timelock: u32,
    pub fidelity_feerate: f64,
    pub required_confirms: u32,
    pub base_fee: u64,
    pub amount_relative_fee_pct: f64,
    pub time_relative_fee_pct: f64,
}

/// Result of probing a chain backend. Electrum answers the height/chain questions
/// from its tip subscription, so both backends fill the same shape.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    pub reachable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Why it failed, structured. The message alone cannot be branched on, so without this
    /// the gate could not tell a rejected password from a node that is simply down.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<crate::error::ErrorCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks: Option<u64>,
    /// true when headers == blocks and IBD is over
    pub synced: bool,
    /// Hex of the signet challenge script, on signet only. Every signet shares one genesis
    /// hash and differs solely in this script, so it is the only thing that names *which*
    /// signet. Core reports it; Electrum synthesizes its chain info from the header tip and
    /// cannot, which is why callers must tolerate `None` on a signet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signet_challenge: Option<String>,
}

/// bootstrapProgress is informational only — init doesn't gate on it.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorStatus {
    /// True only when both the SOCKS greeting and control-port handshake succeeded.
    pub reachable: bool,
    pub authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bootstrap_progress: Option<u8>,
    /// Tor's own one-line description of the phase it is in, e.g. "Loading relay descriptors".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bootstrap_summary: Option<String>,
    /// Portal failing to reach or authenticate against Tor — not Tor failing to connect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Loopback ports Portal's own Tor was started on; freshly chosen each run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socks_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_port: Option<u16>,
}

/// Work a quit would interrupt rather than finish.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuitBlockers {
    pub swap_running: bool,
    /// Recovery only advances while the app is running, so quitting stalls it until next launch.
    pub recovery_running: bool,
    pub running_routers: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionTypeDto {
    Tor,
    Clearnet,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitConfig {
    pub wallet_name: String,
    #[serde(default)]
    pub wallet_password: Option<String>,
    pub connection_type: ConnectionTypeDto,
    #[serde(default)]
    pub data_dir: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitResult {
    pub wallet_name: String,
    /// The root the wallet was listed under, not its own data dir: the frontend keys its caches
    /// by the pair of this and the name.
    pub data_dir: String,
    /// The wallet was already open for another session and this one joined it rather than
    /// starting a second Taker.
    pub joined: bool,
    /// Something the user should know about how they joined, e.g. that the running wallet
    /// uses a different server than the one they picked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Whether a wallet is open, asked on every page load.
///
/// Separate from `WalletInfo` because "nothing is open" is the ordinary answer here, not a
/// failure: a reload replaces the page but not the process, so the frontend has to ask what
/// the process is already holding. Reporting that as an error would make a normal startup log
/// a failed request in the browser console, every single time.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStateDto {
    pub initialized: bool,
    /// The connection gate was cleared, whether or not a wallet is open — a router session has
    /// none, and a reload must not send it back to the gate.
    pub connected: bool,
    pub wallet_name: Option<String>,
    pub data_dir: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletInfo {
    pub wallet_name: String,
    pub wallet_path: String,
    pub data_dir: String,
}

/// Opaque one-shot restore selection returned by the Rust-owned file dialog.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreSelectionView {
    pub selection_id: uuid::Uuid,
    pub display_name: String,
}

// ---------------------------------------------------------------------------
// Wallet operations
// ---------------------------------------------------------------------------

/// Mirrors openswap's `Balances` (all amounts in sats).
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalancesDto {
    pub regular: u64,
    pub swap: u64,
    pub contract: u64,
    pub fidelity: u64,
    pub spendable: u64,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AddressTypeDto {
    P2wpkh,
    P2tr,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAddress {
    pub address: String,
    pub address_type: String,
    /// Full path from the master key, e.g. `m/86'/1'/0'/0/7`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivation_path: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddressValidation {
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Condensed from `bitcoind::bitcoincore_rpc::json::ListTransactionResult`.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxSummary {
    pub txid: String,
    pub category: String,
    /// Signed: negative for outgoing, positive for incoming.
    pub amount_sats: i64,
    pub confirmations: i32,
    pub address: Option<String>,
    pub time: u64,
    pub fee_sats: Option<i64>,
    /// Core's wallet label for the receiving output, e.g. "watchonly_swapcoin".
    pub label: Option<String>,
    /// Unix seconds Portal first saw this transaction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_seen: Option<u64>,
    /// Full HD path of `address` when Portal knows it. Taker listings only, for now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivation_path: Option<String>,
}

/// One of the wallet's own HD addresses: handed out for receiving, or holding a coin now.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletAddressDto {
    pub address: String,
    /// `p2tr` or `p2wpkh`.
    pub address_type: String,
    pub change: bool,
    pub derivation_path: String,
    pub balance_sats: u64,
    /// Some of the balance has no confirmation yet.
    pub unconfirmed: bool,
}

/// One UTXO plus its openswap-specific spend-type classification.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UtxoEntry {
    pub txid: String,
    pub vout: u32,
    pub amount_sats: u64,
    pub confirmations: u32,
    pub address: Option<String>,
    /// Full path from the master key for a seed coin; swap and contract coins have none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivation_path: Option<String>,
    pub spendable: bool,
    pub solvable: bool,
    /// Human category from openswap's `UTXOSpendInfo` Display impl, e.g.
    /// "regular", "incoming swap", "outgoing swap", "timelock contract",
    /// "hashlock contract", "fidelity bond", "swept".
    pub spend_type: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outpoint {
    pub txid: String,
    pub vout: u32,
}

/// One coin named in a swap report. Mirrors the crate's `ReportUtxo`, which records where the
/// money sat rather than which transaction moved it — there is no outpoint to carry.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportUtxo {
    pub address: String,
    pub value_sats: u64,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    pub txid: String,
}

/// Structured equivalent of the crate's `Wallet::display_fidelity_bonds` string dump —
/// `Wallet::get_fidelity_bonds()`/`calculate_bond_value` already expose everything needed
/// directly, no openswap-side change required (see `.claude/MAKER_INTEGRATION.md` §4).
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FidelityBondDto {
    pub outpoint: Outpoint,
    pub amount_sats: u64,
    pub is_spent: bool,
    /// Not yet unlocked and not already redeemed.
    pub is_locked: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeEstimate {
    /// `None` where the chain server has no estimate for that target.
    pub fast: Option<f64>,
    pub medium: Option<f64>,
    pub slow: Option<f64>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceEstimate {
    pub usd: f64,
    /// True when the live price service failed and Portal returned the last
    /// successfully saved quote instead.
    pub cached: bool,
}

// ---------------------------------------------------------------------------
// Market / offerbook
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferDto {
    /// Self-chosen label, not an identity: any router can claim any name. Empty for offers
    /// saved before the crate carried names, until they are fetched again.
    pub name: String,
    pub base_fee: u64,
    pub amount_relative_fee_pct: f64,
    pub time_relative_fee_pct: f64,
    pub required_confirms: u32,
    pub max_size: u64,
    pub min_size: u64,
    pub bond_amount_sats: u64,
    /// Absolute block height the bond unlocks at.
    pub bond_locktime_height: u32,
    pub bond_txid: String,
    pub bond_vout: u32,
    pub bond_is_spent: bool,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MakerDto {
    pub address: String,
    /// What the router said it supports in its handshake: "legacy", "taproot", "ln-swap-in",
    /// "ln-swap-out". Only names this build knows; empty until an offer is fetched.
    pub features: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offer: Option<OfferDto>,
    /// "good" | "bad" | "unresponsive"
    pub state: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferBookView {
    pub good: Vec<MakerDto>,
    pub bad: Vec<MakerDto>,
    pub unresponsive: Vec<MakerDto>,
    pub syncing: bool,
}

// ---------------------------------------------------------------------------
// Swap
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolVersionDto {
    Legacy,
    Taproot,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapRequest {
    pub protocol: ProtocolVersionDto,
    pub amount_sats: u64,
    /// Falls back to the two-router route floor when a caller omits it.
    #[serde(default = "default_router_count")]
    pub router_count: usize,
    #[serde(default)]
    pub outpoints: Option<Vec<Outpoint>>,
    #[serde(default)]
    pub preferred_routers: Option<Vec<String>>,
    /// Funding transactions per hop. `None` keeps the crate's own default.
    #[serde(default)]
    pub tx_count: Option<u32>,
    /// Sat/vB for every transaction in the swap; the crate takes whole rates only.
    pub fee_rate: u64,
    /// PaySwap receiver. When set, `amount_sats` is what the receiver gets, not what leaves
    /// the wallet — the crate solves the gross route amount backward from it.
    #[serde(default)]
    pub payment_address: Option<String>,
}

fn default_router_count() -> usize {
    2
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapFundingEstimateDto {
    /// The next two are totals across the planned funding split: the crate funds our own
    /// hop from up to `tx_count` transactions, not one.
    pub vbytes: u64,
    pub fee_sats: u64,
    /// Wallet UTXOs the funding transactions consume.
    pub outgoing_utxo_count: usize,
    /// Contracts the last hop can pay us back through — the sweep spends each one separately,
    /// so one UTXO each. A ceiling, not a count: `tx_count` is the most any hop may forward,
    /// and a router short of liquidity commits to fewer, which then carries down the route.
    pub incoming_utxo_count: usize,
    /// Ceiling on what one router deducts for miner fees: its funding splits at the full input
    /// budget plus one cooperative claim per contract. The settled cost can only come in lower.
    pub route_mining_fee_per_router_sats: u64,
    /// Fee to claim the incoming contracts at the end; not a full swap fee total.
    pub sweep_fee_sats: u64,
    /// The dust limit of the P2TR output each incoming contract is swept to.
    pub receive_dust_sats: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterFeeInfoDto {
    pub address: String,
    pub protocol: String,
    pub base_fee: u64,
    pub amount_relative_fee_pct: f64,
    pub time_relative_fee_pct: f64,
    pub locktime: u16,
    pub estimated_fee_sats: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapSummaryDto {
    pub swap_id: String,
    pub protocol: String,
    pub send_amount_sats: u64,
    pub routers: Vec<RouterFeeInfoDto>,
    /// Ceiling: router fees, every hop's funding and sweep reimbursement at its negotiated
    /// maximum, and the taker's own funding tx. The settled cost can only come in under it.
    pub total_estimated_fee_sats: u64,
    /// What the taker gets back if every cost hits its ceiling, so a floor, not a forecast.
    /// Zero on a PaySwap: the receiver is paid and nothing comes back.
    pub estimated_receive_amount_sats: u64,
    /// `total_estimated_fee_sats` split the way the summary panel and the report already
    /// split it, so the live circuit cannot disagree with either about what a mining fee is.
    pub router_fee_sats: u64,
    pub mining_fee_sats: u64,
    /// Present only when this swap pays a third-party receiver.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment: Option<PaymentQuoteDto>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentQuoteDto {
    pub address: String,
    /// Exact amount the receiver gets.
    pub amount_sats: u64,
}

/// Coarse in-memory lifecycle snapshot (survives across commands via `AppState.active_swap`).
/// For live per-router detail, see `SwapTrackerDto` / `get_swap_tracker`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapProgressDto {
    pub swap_id: String,
    pub started_at: Option<u64>,
    /// Replayed so a remount can restore what only `prepare_swap` ever returned.
    pub summary: SwapSummaryDto,
}

/// Where a swap is, in protocol order. `Failed` sorts last so a failed swap's own `failed_at_phase`
/// is what records how far it got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwapPhaseDto {
    RoutersDiscovered,
    Negotiated,
    FundingCreated,
    FundsBroadcast,
    ContractsExchanged,
    Finalizing,
    PrivkeysForwarded,
    Completed,
    Failed,
}

/// Where one router is in the protocol. `Ord` follows the protocol order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterStageDto {
    /// Route position not reached yet.
    Waiting,
    /// Terms agreed, nothing in flight.
    Negotiated,
    /// Connected over Tor, exchanging contract data.
    Handshaking,
    /// This hop's contract is on-chain, waiting for confirmations.
    Confirming,
    /// Contract confirmed — the funds have reached this hop.
    Routed,
    KeyReceived,
    /// Private key passed to the next hop; this router is done.
    Settled,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterProgressDto {
    pub address: String,
    pub stage: RouterStageDto,
}

/// Live per-router detail, from Portal's own swap journal (`ops::taker_journal`).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapTrackerDto {
    pub phase: SwapPhaseDto,
    // send_amount_sats/router_count let the frontend rebuild its progress screen after remounting
    // mid-swap (e.g. navigating away and back) without a cached SwapSummary — prepareSwap only
    // ever returns one, and re-running it isn't possible for an already-running swap.
    pub send_amount_sats: u64,
    pub router_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    pub routers: Vec<RouterProgressDto>,
    /// Our own Taproot contract transactions, one per split, as they are broadcast.
    pub outgoing_contract_txids: Vec<String>,
    /// Our own funding has confirmed, which no router stage says: the first router's wait
    /// would otherwise swallow ours.
    pub outgoing_confirmed: bool,
    /// "legacy" | "taproot"
    pub protocol: String,
    /// What each leg's funding wait polled, in route order, while this process runs the swap:
    /// the leg's real on-chain txids.
    pub funding_wait_txids: Vec<Vec<String>>,
    /// PaySwap receiver, echoed so a remounted page still knows where the coins are going
    /// without the prepared quote.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_amount_sats: Option<u64>,
}

/// How far `prepare_swap` has got, for a progress readout while it blocks. Absent until the
/// routers are picked.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapPreparationDto {
    pub router_count: usize,
    /// Routers that have agreed terms so far.
    pub negotiated_count: usize,
}

/// A contract still holding funds, from the wallet's own live UTXO set.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryContractDto {
    pub outpoint: Outpoint,
    pub amount_sats: u64,
    /// "hashlock" — spendable as soon as it confirms, because this wallet holds the preimage.
    /// "timelock" — this wallet's own funding, reclaimable only once the refund delay matures.
    pub claim_path: String,
    pub confirmations: u32,
    /// The first router's payment for a swap this wallet already received, rather than funds of
    /// its own: the router claims it, and it has no refund wait worth counting.
    pub router_owed: bool,
    /// Blocks still to wait. Timelock contracts only; `None` means nothing left to wait for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks_remaining: Option<u32>,
    /// The refund delay in full. Sent alongside the remainder so the UI can say how far
    /// through the wait this is rather than only how much is left — "~60 blocks" with no
    /// denominator reads the same on the first block as on the last.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_blocks: Option<u32>,
}

/// A transaction the recovery loop broadcast to claim funds back, from the crate's recovery
/// reports.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveredContractDto {
    pub txid: String,
    /// "incoming" for a hashlock claim of the route's payment to us, "outgoing" for a timelock
    /// refund of our own funding.
    pub leg: String,
}

/// One swap inside the recovery, for the list that stands in front of the detail view. Carries
/// no contracts: the wallet reports its live contracts as one pool with no way to attribute them
/// per swap from outside the crate.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverySummary {
    pub swap_id: String,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    pub router_count: usize,
    pub send_amount_sats: u64,
    /// How many recovery transactions have claimed this swap's funds back.
    pub resolved_count: usize,
    pub active: bool,
    pub updated_at: u64,
    /// How the funds came back, from the crate's recovery reports: `hashlock`, `timelock`.
    pub recovery_types: Vec<String>,
}

/// Read from the swap journal plus the cached wallet handle — never through the taker mutex, so it
/// still answers while a swap holds the taker for hours.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryStatus {
    pub active: bool,
    /// Whether OpenSwap's background recovery worker currently exists and is unfinished.
    /// `None` means the taker was busy and could not be inspected without blocking this read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_running: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap_id: Option<String>,
    /// "not_started" while funds are still out, "cleaned_up" once recovery has nothing left.
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    pub router_count: usize,
    pub send_amount_sats: u64,
    /// Every key was handed over before the swap stopped, so the route paid this wallet and what
    /// is left is the router's payment.
    pub swap_received: bool,
    /// Contracts still holding funds.
    pub pending: Vec<RecoveryContractDto>,
    /// Recovery transactions already broadcast.
    pub resolved: Vec<RecoveredContractDto>,
    /// The longest wait left across every pending timelock contract.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks_remaining: Option<u32>,
    /// This wallet's own funds still in contracts.
    pub locked_sats: u64,
    /// Router payments still in contracts, which are not this wallet's to spend.
    pub router_owed_sats: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapReportSummary {
    pub swap_id: String,
    /// From the crate's report: "success" | "recovery_hashlock" | "recovery_timelock" | "failed".
    /// From the swap journal when the crate wrote none: "interrupted" (recovery still has funds
    /// out) or "recovered".
    pub status: String,
    /// False when this row came from the swap journal — the swap is real, but its numbers are
    /// only the ones the journal kept.
    pub reported: bool,
    pub start_timestamp: u64,
    /// `None` for a journal row: the swap never finished, so it has no end time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_timestamp: Option<u64>,
    pub outgoing_amount_sats: u64,
    /// What actually landed back in the wallet, measured on-chain — not an estimate. See
    /// `received_sats` in `ops::taker_reports` for the one case that still has to be derived.
    pub received_amount_sats: u64,
    pub fee_paid_sats: u64,
    pub routers_count: usize,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportRouterFee {
    pub router_index: usize,
    pub base_fee_sats: f64,
    pub amount_relative_fee_sats: f64,
    pub time_relative_fee_sats: f64,
    pub total_fee_sats: f64,
}

/// The coin a completed swap actually paid the user, recovered from chain rather than from
/// the report — see `taker_reports::get_incoming_swap_utxo`.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapUtxoDto {
    pub txid: String,
    pub vout: u32,
    pub amount_sats: u64,
    pub address: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapReportDetail {
    pub swap_id: String,
    pub status: String,
    pub network: String,
    pub swap_duration_seconds: f64,
    pub start_timestamp: u64,
    pub end_timestamp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub outgoing_amount_sats: u64,
    /// See the same field's doc on `SwapReportSummary`.
    pub received_amount_sats: u64,
    pub fee_paid_sats: u64,
    pub mining_fee_sats: u64,
    pub fee_percentage: f64,
    pub total_router_fees_sats: u64,
    /// Wallet coins spent to fund the outgoing contract.
    pub outgoing_utxos: Vec<ReportUtxo>,
    /// Wallet coins the incoming swapcoin sweep created. Empty when the swap paid a
    /// third-party receiver, whose settlement outputs this wallet never owns.
    pub incoming_utxos: Vec<ReportUtxo>,
    pub funding_txids: Vec<Vec<String>>,
    pub routers_count: usize,
    pub router_addresses: Vec<String>,
    pub router_fee_info: Vec<ReportRouterFee>,
    /// Paid a third-party receiver rather than back to this wallet.
    pub pay_swap: bool,
    /// Raw pass-through of the crate's `DeniabilityProof` (already `Serialize`) rather than
    /// hand-mirrored types — the frontend renders whatever shape comes through generically.
    pub deniability_proof: Option<serde_json::Value>,
    /// The crate's report, as JSON.
    pub raw: String,
}

// ---------------------------------------------------------------------------
// Maker
// ---------------------------------------------------------------------------

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MakerInitConfig {
    /// Stable registration ID. Wallet name remains independently configurable.
    pub router_id: String,
    pub wallet_name: String,
    #[serde(default)]
    pub wallet_password: Option<String>,
    /// Published in the router's offer, where wallets show it.
    pub name: String,
    pub network_port: u16,
    pub rpc_port: u16,
    pub socks_port: u16,
    pub control_port: u16,
    pub fidelity_amount: u64,
    pub fidelity_timelock: u32,
    pub fidelity_feerate: f64,
    pub required_confirms: u32,
    pub base_fee: u64,
    pub amount_relative_fee_pct: f64,
    pub time_relative_fee_pct: f64,
    #[serde(default)]
    pub data_dir: Option<String>,
    /// A backup file the host registered: the router's wallet is restored from it, encrypted
    /// with `wallet_password`, instead of created. Never part of the persisted settings.
    #[serde(default)]
    pub restore_selection: Option<uuid::Uuid>,
    /// Restore the router's wallet from this phrase instead of creating a new one. Never copied
    /// into the persisted settings, and named so the web journal scrubs it from requests.
    #[serde(default)]
    pub mnemonic: Option<String>,
}

/// A wallet's or router's recovery phrase, decrypted for one response. Wiped on drop, and not
/// `Debug` or `Clone`, so no copy outlives the response or reaches a log.
pub struct RecoveryPhraseDto {
    pub words: zeroize::Zeroizing<String>,
}

impl serde::Serialize for RecoveryPhraseDto {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut dto = serializer.serialize_struct("RecoveryPhraseDto", 1)?;
        dto.serialize_field("words", self.words.as_str())?;
        dto.end()
    }
}

/// Persisted registration settings. Wallet and Tor control passwords are
/// process-only and omitted when this DTO is serialized to disk.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MakerSettingsDto {
    #[serde(alias = "makerId")]
    pub router_id: String,
    pub wallet_name: String,
    // Registrations saved before names existed, and maker-dashboard's own file; `config.toml`
    // overrides it on load anyway.
    #[serde(default)]
    pub name: String,
    pub network_port: u16,
    pub rpc_port: u16,
    pub socks_port: u16,
    pub control_port: u16,
    pub fidelity_amount: u64,
    pub fidelity_timelock: u32,
    // Registrations saved before this field existed; `config.toml` overrides it on load anyway.
    #[serde(default = "crate::ops::maker_settings::default_fidelity_feerate")]
    pub fidelity_feerate: f64,
    pub required_confirms: u32,
    pub base_fee: u64,
    pub amount_relative_fee_pct: f64,
    pub time_relative_fee_pct: f64,
    #[serde(default)]
    pub data_dir: Option<String>,
    /// Filled in only when listing, from the record beside the router's data; never read from
    /// a caller or written to the registry.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
}

impl MakerSettingsDto {
    pub fn from_init(c: &MakerInitConfig, data_dir: &std::path::Path) -> Self {
        Self {
            router_id: c.router_id.clone(),
            wallet_name: c.wallet_name.clone(),
            name: c.name.clone(),
            network_port: c.network_port,
            rpc_port: c.rpc_port,
            socks_port: c.socks_port,
            control_port: c.control_port,
            fidelity_amount: c.fidelity_amount,
            fidelity_timelock: c.fidelity_timelock,
            fidelity_feerate: c.fidelity_feerate,
            required_confirms: c.required_confirms,
            base_fee: c.base_fee,
            amount_relative_fee_pct: c.amount_relative_fee_pct,
            time_relative_fee_pct: c.time_relative_fee_pct,
            data_dir: Some(data_dir.display().to_string()),
            network: None,
        }
    }

    pub fn into_init(self, wallet_password: Option<String>) -> MakerInitConfig {
        MakerInitConfig {
            router_id: self.router_id,
            wallet_name: self.wallet_name,
            wallet_password,
            name: self.name,
            network_port: self.network_port,
            rpc_port: self.rpc_port,
            socks_port: self.socks_port,
            control_port: self.control_port,
            fidelity_amount: self.fidelity_amount,
            fidelity_timelock: self.fidelity_timelock,
            fidelity_feerate: self.fidelity_feerate,
            required_confirms: self.required_confirms,
            base_fee: self.base_fee,
            amount_relative_fee_pct: self.amount_relative_fee_pct,
            time_relative_fee_pct: self.time_relative_fee_pct,
            data_dir: self.data_dir,
            restore_selection: None,
            mnemonic: None,
        }
    }
}

/// Coarse lifecycle for one entry in `AppState.makers`, pushed through a
/// maker-ID-tagged `maker://phase-changed` event.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "phase")]
pub enum MakerPhase {
    #[default]
    NotConfigured,
    Restoring,
    Initializing,
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed {
        message: String,
    },
}

/// The mining fee a send would pay, before it is broadcast.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendFeeEstimate {
    pub fee_sats: u64,
    pub inputs: usize,
}

/// A wallet in the picker. `network` is `None` until Portal can tell, and `"test"` when all it
/// knows is that the wallet is on some test network.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletListing {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
}

/// Where this host keeps wallet data. The frontend must not derive these itself: the default
/// comes from the crate and the active one may have been chosen by the user.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathsDto {
    pub data_dir: String,
    pub wallets_dir: String,
}

/// Emitted when the crate moves a failed swap into recovery rather than aborting it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryHandoff {
    pub swap_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MakerPhaseEvent {
    pub router_id: String,
    pub phase: MakerPhase,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MakerStatusDto {
    pub router_id: String,
    pub phase: MakerPhase,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tor_address: Option<String>,
    pub network_port: u16,
    /// None means the wallet file could not be inspected without opening it.
    pub wallet_encrypted: Option<bool>,
    /// Whether the running wallet holds an unspent fidelity bond, i.e. setup is behind it. None
    /// while stopped, or while a sync holds the wallet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_bond: Option<bool>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedMakerPortsDto {
    pub network_port: u16,
    pub rpc_port: u16,
}

/// Per-port verdict for a maker's listeners. `conflict` is `None` when the port is
/// usable; otherwise it names what is holding it, ready to show verbatim.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MakerPortCheckDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_port: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpc_port: Option<String>,
}

/// The maker's own perspective on one swap — one leg, not the whole
/// multi-hop route a `SwapReportSummary`/`SwapReportDetail` (taker-side)
/// describes, so this is a separate, simpler shape rather than reusing
/// those. Mirrors `openswap::wallet::MakerReport`.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MakerSwapReportSummary {
    pub swap_id: String,
    pub status: String,
    pub start_timestamp: u64,
    pub end_timestamp: u64,
    pub incoming_amount_sats: u64,
    pub outgoing_amount_sats: u64,
    pub fee_earned_sats: u64,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MakerSwapReportDetail {
    pub swap_id: String,
    pub status: String,
    pub network: String,
    pub swap_duration_seconds: f64,
    pub start_timestamp: u64,
    pub end_timestamp: u64,
    pub incoming_amount_sats: u64,
    pub outgoing_amount_sats: u64,
    pub fee_earned_sats: u64,
    /// Wallet coins the incoming swapcoin sweep created.
    pub incoming_utxos: Vec<ReportUtxo>,
    /// Wallet coins spent to fund the outgoing contract.
    pub outgoing_utxos: Vec<ReportUtxo>,
    pub timelock: u32,
    /// Raw pass-through of the crate's `DeniabilityProof` — see the same
    /// field's doc comment on `SwapReportDetail`.
    pub deniability_proof: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub line: String,
}

/// One server the picker offers. Reference data, not configuration: the list is fixed at
/// compile time and the choice is never persisted, same as everything else on the gate.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElectrumPresetDto {
    /// Short name for the picker — the URL is shown beside it, not in place of it.
    pub label: String,
    pub url: String,
    /// "bitcoin" or "signet". The picker colours by this: one of them spends real money.
    pub network: String,
}

/// Seeded on every launch and held in memory only: an edit is deliberately forgotten so a
/// node's RPC password is never at rest.
pub(crate) const DEFAULT_ELECTRUM_URL: &str = "ssl://electrum.openswap.live:50002";
const DEFAULT_NODE_HOST: &str = "127.0.0.1";
const DEFAULT_NODE_RPC_PORT: u16 = 38332;
const DEFAULT_NODE_ZMQ_PORT: u16 = 28332;
const DEFAULT_NODE_USERNAME: &str = "user";
const DEFAULT_NODE_PASSWORD: &str = "password";

impl Default for ChainBackendConfig {
    fn default() -> Self {
        Self {
            kind: ChainBackendKind::Electrum,
            electrum: ElectrumBackendDto {
                url: DEFAULT_ELECTRUM_URL.to_string(),
                use_tor: false,
            },
            // Prefilled rather than `None` so the gate can show the standard node fields
            // without the UI having to carry its own copy of the defaults.
            node: Some(NodeBackendDto {
                host: DEFAULT_NODE_HOST.to_string(),
                port: DEFAULT_NODE_RPC_PORT,
                username: DEFAULT_NODE_USERNAME.to_string(),
                password: DEFAULT_NODE_PASSWORD.to_string(),
                zmq_port: DEFAULT_NODE_ZMQ_PORT,
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Address blocklist
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlocklistEntryDto {
    pub address: String,
    pub label: Option<String>,
}

/// A CSV row that was not imported, by its 1-based line number.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlocklistRejectDto {
    pub line: usize,
    pub address: String,
    pub reason: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlocklistImportDto {
    pub added: usize,
    pub updated: usize,
    pub rejected: Vec<BlocklistRejectDto>,
}
