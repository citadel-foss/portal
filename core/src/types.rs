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
    pub password_configured: bool,
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
    pub min_swap_amount: u64,
    pub fidelity_amount: u64,
    pub fidelity_timelock: u32,
    pub required_confirms: u32,
    pub base_fee: u64,
    pub amount_relative_fee_pct: f64,
    pub time_relative_fee_pct: f64,
}

/// Result of probing a chain backend. Electrum answers the height/chain questions
/// from its tip subscription, so both backends fill the same shape; `subversion`
/// is the one field only Core can report.
#[derive(Debug, serde::Serialize)]
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
    /// Core's version string, e.g. "/Satoshi:27.0.0/".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subversion: Option<String>,
    /// [0..1] estimate of chain verification progress.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_progress: Option<f64>,
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
    /// Result of the independent SOCKS5 greeting, even if the control port later fails.
    pub socks_reachable: bool,
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
    pub running_makers: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionTypeDto {
    Tor,
    Clearnet,
}

#[derive(Debug, serde::Deserialize)]
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
    /// False for a re-offered cached address whose payment status has not been checked yet.
    /// A freshly derived address is unused by construction, so it is always true.
    pub verified: bool,
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
    pub bond_index: u32,
    pub outpoint: Outpoint,
    pub amount_sats: u64,
    /// Absolute block height the bond unlocks at.
    pub lock_time_height: u32,
    pub is_spent: bool,
    /// Not yet unlocked and not already redeemed.
    pub is_locked: bool,
    /// Coinswap's theoretical fidelity-value formula — only computable for a confirmed,
    /// unspent bond, hence optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bond_value_sats: Option<u64>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeEstimate {
    pub high: f64,
    pub mid: f64,
    pub low: f64,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceEstimate {
    pub usd: f64,
    /// True when the live price service failed and Portal returned the last
    /// successfully saved quote instead.
    pub cached: bool,
    /// Unix timestamp in seconds for the live quote or persisted fallback.
    pub fetched_at: u64,
}

// ---------------------------------------------------------------------------
// Market / offerbook
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferDto {
    pub base_fee: u64,
    pub amount_relative_fee_pct: f64,
    pub time_relative_fee_pct: f64,
    pub required_confirms: u32,
    pub minimum_locktime: u16,
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
    /// "legacy" | "taproot" | null (protocol unknown until an offer is fetched)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
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
    pub last_sync_ts: u64,
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
    /// The next three are totals across the planned funding split: the crate funds our own
    /// hop from up to `tx_count` transactions, not one.
    pub input_count: usize,
    pub vbytes: u64,
    pub fee_sats: u64,
    /// Wallet UTXOs the funding transactions consume.
    pub outgoing_utxo_count: usize,
    /// Contracts the last hop can pay us back through — the sweep spends each one separately,
    /// so one UTXO each. A ceiling, not a count: `tx_count` is the most any hop may forward,
    /// and a router short of liquidity commits to fewer, which then carries down the route.
    pub incoming_utxo_count: usize,
    /// The swap feerate every transaction in the route is priced at. Reported because it is
    /// the `SwapParams` default and nothing here overrides it, so a swap cannot be sped up or
    /// slowed down by paying more.
    pub fee_rate_sats_per_vb: f64,
    /// Ceiling on what one router deducts for miner fees: its funding splits at the full input
    /// budget plus one cooperative claim per contract. The settled cost can only come in lower.
    pub route_mining_fee_per_router_sats: u64,
    /// Fee to claim the incoming contracts at the end; not a full swap fee total.
    pub sweep_fee_sats: u64,
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
    /// Reserved on the final hop to settle the receiver's output.
    pub settlement_budget_sats: u64,
}

/// Coarse in-memory lifecycle snapshot (survives across commands via `AppState.active_swap`).
/// For live per-router detail, see `SwapTrackerDto` / `get_swap_tracker`, which reads the crate's
/// own `swap_tracker.cbor` — the same file the old Electron app polled directly off disk.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapProgressDto {
    pub swap_id: String,
    pub started_at: Option<u64>,
    /// Replayed so a remount can restore what only `prepare_swap` ever returned.
    pub summary: SwapSummaryDto,
}

/// One of the crate's own per-maker boolean flags, carried by its field name so the UI can name
/// the protocol step instead of rendering a fraction.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterMilestoneDto {
    pub key: &'static str,
    pub done: bool,
}

/// Where one router is in the protocol.
///
/// Deliberately coarser than the crate's flags: the taker only flushes the tracker at a handful
/// of points per hop (once per maker under taproot), so these are the transitions actually
/// observable from disk. `Ord` follows the protocol order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
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
    /// The raw flags behind `stage`, in the order the crate sets them: the protocol's exchange
    /// flags (5 taproot / 12 legacy) plus `negotiated` and the two shared finalization flags.
    pub milestones: Vec<RouterMilestoneDto>,
}

/// Live per-router detail read straight from `openswap::taker::swap_tracker::SwapTracker`
/// (a public crate API — see `commands::taker_swap`'s module doc).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapTrackerDto {
    /// "routers_discovered" | "negotiated" | "funding_created" | "funds_broadcast" |
    /// "contracts_exchanged" | "finalizing" | "privkeys_forwarded" | "completed" | "failed"
    pub phase: String,
    // send_amount_sats/router_count let the frontend rebuild its progress screen after remounting
    // mid-swap (e.g. navigating away and back) without a cached SwapSummary — prepareSwap only
    // ever returns one, and re-running it isn't possible for an already-running swap.
    pub send_amount_sats: u64,
    pub router_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    pub routers: Vec<RouterProgressDto>,
    /// Contract transactions recorded so far on each kind of leg. A hop is funded by up to
    /// `tx_count` splits rather than one transaction, so these are the strands a leg actually
    /// carries, and they fill in as the swap records each txid.
    ///
    /// `watchonly` covers every leg between two routers as one flat list — the crate keeps no
    /// per-hop grouping — so it only attributes to a leg when it divides evenly across them.
    pub outgoing_contract_txids: Vec<String>,
    pub incoming_contract_txids: Vec<String>,
    pub watchonly_contract_txids: Vec<String>,
    /// PaySwap receiver, echoed from the tracker so a remounted page still knows where the
    /// coins are going without the prepared quote.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_amount_sats: Option<u64>,
}

/// How far `prepare_swap` has got, for a progress readout while it blocks.
///
/// Read off `swap_tracker.cbor` rather than reported by the command: `prepare_swap` is one
/// opaque call that holds the taker for its whole duration, but it persists the record as it
/// goes, so the file is the only place its progress is visible from.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapPreparationDto {
    /// "routers_discovered" | "negotiated"
    pub phase: String,
    pub router_count: usize,
    /// Routers that have agreed terms so far.
    pub negotiated_count: usize,
}

/// A contract still holding funds, from the wallet's own live UTXO set.
///
/// This — not the tracker — is what a recovery in progress actually looks like. The tracker's
/// `recovery.incoming`/`recovery.outgoing` vectors stay **empty** until a sweep succeeds: nothing
/// ever writes an `Unresolved` placeholder (`taker/background_services.rs`, `update_tracker_outcomes`
/// only pushes on resolution). Driving the UI off them showed an empty page for the entire wait.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryContractDto {
    pub outpoint: Outpoint,
    pub amount_sats: u64,
    /// "hashlock" — spendable as soon as it confirms, because this wallet holds the preimage.
    /// "timelock" — this wallet's own funding, reclaimable only once the refund delay matures.
    pub claim_path: String,
    pub confirmations: u32,
    /// Blocks still to wait. Timelock contracts only; `None` means nothing left to wait for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks_remaining: Option<u32>,
    /// The refund delay in full. Sent alongside the remainder so the UI can say how far
    /// through the wait this is rather than only how much is left — "~60 blocks" with no
    /// denominator reads the same on the first block as on the last.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_blocks: Option<u32>,
}

/// A contract the recovery loop has already spent back, as the tracker recorded it.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveredContractDto {
    pub contract_txid: String,
    /// "hashlock" | "timelock" | "key_path" | "discarded" | "unresolved"
    pub resolution: String,
    /// The transaction that claimed it back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spending_txid: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_at_phase: Option<String>,
    pub router_count: usize,
    pub send_amount_sats: u64,
    /// How many of this swap's contracts recovery has already claimed back.
    pub resolved_count: usize,
    pub active: bool,
    pub updated_at: u64,
}

/// Read entirely from `swap_tracker.cbor` plus the cached wallet handle — never through the taker
/// mutex, so it still answers while a swap holds the taker for hours.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryStatus {
    /// Any contract still unresolved, from disk — so it survives a reload.
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap_id: Option<String>,
    /// The crate's `RecoveryPhase`. In practice only three of the six are ever written:
    /// "not_started" until something resolves, then "incoming_recovered"/"outgoing_recovered",
    /// then "cleaned_up". The rest exist but are set only in the crate's own tests.
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// Where the swap got to before it stopped, which decides whether there is an incoming leg
    /// at all: a swap that failed on its first hop never created one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_at_phase: Option<String>,
    pub router_count: usize,
    pub send_amount_sats: u64,
    /// Contracts still holding funds.
    pub pending: Vec<RecoveryContractDto>,
    /// Contracts already claimed back.
    pub resolved: Vec<RecoveredContractDto>,
    /// The longest wait left across every pending timelock contract.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks_remaining: Option<u32>,
    pub locked_sats: u64,
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
    /// From the report file: "success" | "recovery_hashlock" | "recovery_timelock" | "failed".
    /// Synthesised from the tracker when no report was written: "interrupted" (the process died
    /// mid-swap and nothing has reclaimed the funds yet), "recovered" (it did), or "unfinished".
    pub status: String,
    /// False when this row came from the tracker because no report file entry exists — the swap
    /// is real, but its numbers are only the ones the tracker kept.
    pub reported: bool,
    pub start_timestamp: u64,
    /// `None` for a tracker-derived row. The tracker's `updated_at` is not an end time — the
    /// crate re-stamps it every launch while re-marking an already-failed swap — so reporting
    /// it as one made a dead swap climb back to the top of a newest-first list on each start.
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
    pub router_address: String,
    pub base_fee_sats: f64,
    pub amount_relative_fee_sats: f64,
    pub time_relative_fee_sats: f64,
    pub total_fee_sats: f64,
}

/// The coin a completed swap actually paid the user, recovered from chain rather than from
/// the report file — see `taker_reports::get_incoming_swap_utxo`.
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
    /// The contract UTXO this swap funded, not just its transaction: the deniability proof
    /// records the exact outpoint, and a Taproot contract output is not necessarily vout 0.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outgoing_contract_outpoint: Option<Outpoint>,
    /// The contract UTXO the route paid back, from the same proof.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incoming_contract_outpoint: Option<Outpoint>,
    /// Wallet coins spent to fund the outgoing contract.
    pub outgoing_utxos: Vec<ReportUtxo>,
    /// Wallet coins the incoming swapcoin sweep created. Empty when the swap paid a
    /// third-party receiver, whose settlement outputs this wallet never owns.
    pub incoming_utxos: Vec<ReportUtxo>,
    pub funding_txids: Vec<Vec<String>>,
    pub routers_count: usize,
    pub router_addresses: Vec<String>,
    pub router_fee_info: Vec<ReportRouterFee>,
    /// Raw pass-through of the crate's `DeniabilityProof` (already `Serialize`) rather than
    /// hand-mirrored types — the frontend renders whatever shape comes through generically.
    pub deniability_proof: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Maker
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MakerInitConfig {
    /// Stable registration ID. Wallet name remains independently configurable.
    pub router_id: String,
    pub wallet_name: String,
    #[serde(default)]
    pub wallet_password: Option<String>,
    pub network_port: u16,
    pub rpc_port: u16,
    pub socks_port: u16,
    pub control_port: u16,
    pub min_swap_amount: u64,
    pub fidelity_amount: u64,
    pub fidelity_timelock: u32,
    pub required_confirms: u32,
    pub base_fee: u64,
    pub amount_relative_fee_pct: f64,
    pub time_relative_fee_pct: f64,
    #[serde(default)]
    pub data_dir: Option<String>,
}

/// Persisted registration settings. Wallet and Tor control passwords are
/// process-only and omitted when this DTO is serialized to disk.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MakerSettingsDto {
    #[serde(alias = "makerId")]
    pub router_id: String,
    pub wallet_name: String,
    pub network_port: u16,
    pub rpc_port: u16,
    pub socks_port: u16,
    pub control_port: u16,
    pub min_swap_amount: u64,
    pub fidelity_amount: u64,
    pub fidelity_timelock: u32,
    pub required_confirms: u32,
    pub base_fee: u64,
    pub amount_relative_fee_pct: f64,
    pub time_relative_fee_pct: f64,
    #[serde(default)]
    pub data_dir: Option<String>,
}

impl MakerSettingsDto {
    pub fn from_init(c: &MakerInitConfig, data_dir: &std::path::Path) -> Self {
        Self {
            router_id: c.router_id.clone(),
            wallet_name: c.wallet_name.clone(),
            network_port: c.network_port,
            rpc_port: c.rpc_port,
            socks_port: c.socks_port,
            control_port: c.control_port,
            min_swap_amount: c.min_swap_amount,
            fidelity_amount: c.fidelity_amount,
            fidelity_timelock: c.fidelity_timelock,
            required_confirms: c.required_confirms,
            base_fee: c.base_fee,
            amount_relative_fee_pct: c.amount_relative_fee_pct,
            time_relative_fee_pct: c.time_relative_fee_pct,
            data_dir: Some(data_dir.display().to_string()),
        }
    }

    pub fn into_init(self, wallet_password: Option<String>) -> MakerInitConfig {
        MakerInitConfig {
            router_id: self.router_id,
            wallet_name: self.wallet_name,
            wallet_password,
            network_port: self.network_port,
            rpc_port: self.rpc_port,
            socks_port: self.socks_port,
            control_port: self.control_port,
            min_swap_amount: self.min_swap_amount,
            fidelity_amount: self.fidelity_amount,
            fidelity_timelock: self.fidelity_timelock,
            required_confirms: self.required_confirms,
            base_fee: self.base_fee,
            amount_relative_fee_pct: self.amount_relative_fee_pct,
            time_relative_fee_pct: self.time_relative_fee_pct,
            data_dir: self.data_dir,
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
    Initializing,
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed {
        message: String,
    },
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incoming_contract_outpoint: Option<Outpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outgoing_contract_outpoint: Option<Outpoint>,
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
pub(crate) const DEFAULT_ELECTRUM_URL: &str = "ssl://electrum.citadelfoss.xyz:50002";
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
