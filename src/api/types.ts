// Mirrors src-tauri/src/types.rs (camelCase, sats as numbers).

export type ChainBackendKind = "electrum" | "coreRpc";

export interface ElectrumBackend {
  url: string;
  /** An `.onion` URL is proxied regardless of this flag — the crate rejects one without a proxy. */
  useTor: boolean;
}

export interface NodeBackend {
  host: string;
  port: number;
  username: string;
  password: string;
  zmqPort: number;
}

/** One server the connection gate offers. Fixed in Rust; the choice is never persisted. */
export interface ElectrumPreset {
  label: string;
  url: string;
  /** "bitcoin" or "signet" — the picker colours by this, since one spends real money. */
  network: string;
}

export interface ChainBackendConfig {
  kind: ChainBackendKind;
  electrum: ElectrumBackend;
  /** Prefilled with the standard local-node values so the connection gate can show them. */
  node: NodeBackend | null;
}

/** Mirrors `RouterDefaultsDto`. Deliberately not duplicated as constants here: these are the
 *  protocol crate's own `MakerServerConfig::default()`, and a second copy in the UI is exactly
 *  how the app ended up running ten times core's figures without anyone noticing. */
export interface RouterDefaults {
  fidelityAmount: number;
  /** The crate's minimum bond amount, read from its own config check. */
  minFidelityAmount?: number;
  fidelityTimelock: number;
  /** sats/vB for the bond transaction; the crate's relay minimum is also the floor. */
  fidelityFeerate: number;
  requiredConfirms: number;
  baseFee: number;
  amountRelativeFeePct: number;
  timeRelativeFeePct: number;
}

export interface BackendStatus {
  reachable: boolean;
  error?: string;
  chain?: string;
  blocks?: number;
  synced: boolean;
  /** Bitcoin Core only; Electrum has no version string to report. */
  subversion?: string;
  verificationProgress?: number;
  /** Hex of the signet challenge script, on signet only. Absent over Electrum even on a
   *  signet — it has no way to report one. */
  signetChallenge?: string;
}

// bootstrapProgress is informational only — openswap's own init doesn't gate on it.
export interface TorStatus {
  reachable: boolean;
  /** Independent SOCKS5 greeting result, even when the control port fails. */
  socksReachable: boolean;
  authenticated: boolean;
  bootstrapProgress?: number;
  /** Tor's own name for the phase it is in, e.g. "Loading relay descriptors". */
  bootstrapSummary?: string;
  error?: string;
  /** Loopback ports Portal's own Tor was started on; freshly chosen each run. */
  socksPort?: number;
  controlPort?: number;
}

/** Work a quit would interrupt rather than finish. Payload of `app://quit-blocked`. */
export interface QuitBlockers {
  swapRunning: boolean;
  recoveryRunning: boolean;
  runningRouters: string[];
}

export type ConnectionType = "tor" | "clearnet";

export interface InitConfig {
  walletName: string;
  walletPassword?: string;
  connectionType: ConnectionType;
  dataDir?: string;
}

export interface InitResult {
  walletName: string;
  dataDir: string;
  /** The wallet was already open in another browser and this one joined it. */
  joined: boolean;
  /** Something worth telling the user about how they joined. */
  note?: string;
}

/** Whether a wallet is open. Never an error — "nothing open" is the ordinary answer. */
export interface SessionState {
  initialized: boolean;
  /** The connection gate was cleared this session, with or without a wallet open. */
  connected: boolean;
  walletName: string | null;
  dataDir: string | null;
}

export interface WalletInfo {
  walletName: string;
  walletPath: string;
  dataDir: string;
}

/** Desktop only. The web host reports a storage label through its session view instead —
 *  a browser is never given a server path. */
export interface Paths {
  dataDir: string;
  walletsDir: string;
}

export interface RestoreSelection {
  selectionId: string;
  displayName: string;
}

// Mirrors src-tauri/src/error.rs — every failed invoke() rejects with this.
export interface AppError {
  code: ErrorCode;
  message: string;
  /** How the UI should behave, decided once in Rust beside the codes themselves. Optional so
   *  an error from an older backend still parses. */
  class?: ErrorClass;
  details?: unknown;
}

/** Mirrors `ErrorClass` in `core/src/error.rs`. */
export type ErrorClass =
  | "transient"
  | "needs-input"
  | "needs-restart"
  | "unsafe-to-retry"
  | "silent"
  | "bug";

export function isAppError(e: unknown): e is AppError {
  return typeof e === "object" && e !== null && "code" in e;
}

export type ErrorCode =
  | "RPC_UNREACHABLE"
  | "RPC_AUTH_FAILED"
  | "TOR_UNREACHABLE"
  | "ZMQ_UNREACHABLE"
  | "WALLET_NOT_FOUND"
  | "WALLET_WRONG_PASSWORD"
  | "WALLET_LOAD_FAILED"
  | "WALLET_OPEN_ELSEWHERE"
  | "WALLET_NETWORK_MISMATCH"
  | "NOT_INITIALIZED"
  | "SWAP_IN_PROGRESS"
  | "INSUFFICIENT_FUNDS"
  | "NOT_ENOUGH_ROUTERS"
  | "ROUTER_NOT_FOUND"
  | "ROUTER_NOT_INITIALIZED"
  | "ROUTER_ALREADY_RUNNING"
  | "ROUTER_NOT_RUNNING"
  | "ROUTER_BUSY"
  | "REPORT_NOT_FOUND"
  | "USER_CANCELLED"
  | "AUTHORIZATION_DENIED"
  | "SENSITIVE_OPERATION_IN_PROGRESS"
  | "INSECURE_DATA_DIRECTORY"
  | "INVALID_FILE_SELECTION"
  | "BACKEND_ROUTE_CHANGED"
  | "CONTRACTS_BROADCASTED"
  | "INVALID_INPUT"
  | "STATE_POISONED"
  | "IO"
  | "INTERNAL";

// ---------------------------------------------------------------------------
// Wallet operations
// ---------------------------------------------------------------------------

export interface Balances {
  regular: number;
  swap: number;
  contract: number;
  fidelity: number;
  spendable: number;
}

export interface SwapLiquidity {
  spendable: number;
  regular: number;
  swap: number;
  maxSwappable: number;
}

export type AddressType = "p2wpkh" | "p2tr";

export interface NewAddress {
  address: string;
  addressType: string;
  /** Full path from the master key, e.g. `m/86'/1'/0'/0/7`. */
  derivationPath?: string;
}

export interface AddressValidation {
  valid: boolean;
  error?: string;
}

export interface TxSummary {
  txid: string;
  category: string;
  amountSats: number;
  confirmations: number;
  address?: string;
  time: number;
  feeSats?: number;
  label?: string;
  /** Unix seconds Portal first saw this transaction. */
  firstSeen?: number;
  /** Full HD path of `address` when Portal knows it. Taker listings only, for now. */
  derivationPath?: string;
}

/** One of the wallet's own HD addresses: handed out for receiving, or holding a coin now. */
export interface WalletAddress {
  address: string;
  addressType: AddressType;
  change: boolean;
  derivationPath: string;
  balanceSats: number;
  /** Some of the balance has no confirmation yet. */
  unconfirmed: boolean;
}

export interface UtxoEntry {
  txid: string;
  vout: number;
  amountSats: number;
  confirmations: number;
  address?: string;
  /** Seed coins only; swap and contract coins are not on an HD key. */
  derivationPath?: string;
  spendable: boolean;
  solvable: boolean;
  spendType: string;
}

export interface Outpoint {
  txid: string;
  vout: number;
}

/** One coin named in a swap report — where the money sat, not which transaction moved it. */
export interface ReportUtxo {
  address: string;
  valueSats: number;
}

export interface SendResult {
  txid: string;
}

/** Sats/vB per tier; null where the chain server has no estimate for it. */
export interface FeeEstimate {
  fast: number | null;
  medium: number | null;
  slow: number | null;
}

export interface PriceEstimate {
  usd: number;
  /** The live request failed and Portal returned its last successfully saved quote. */
  cached: boolean;
  /** Unix timestamp in seconds for the live quote or persisted fallback. */
  fetchedAt: number;
}

// ---------------------------------------------------------------------------
// Market / offerbook
// ---------------------------------------------------------------------------

export interface Offer {
  /** Empty for offers saved before routers sent names. */
  name: string;
  baseFee: number;
  amountRelativeFeePct: number;
  timeRelativeFeePct: number;
  requiredConfirms: number;
  maxSize: number;
  minSize: number;
  bondAmountSats: number;
  bondLocktimeHeight: number;
  bondTxid: string;
  bondVout: number;
  bondIsSpent: boolean;
}

export interface Router {
  address: string;
  protocol?: string;
  offer?: Offer;
  state: "good" | "bad" | "unresponsive";
}

export interface OfferBookView {
  good: Router[];
  bad: Router[];
  unresponsive: Router[];
  syncing: boolean;
}

// ---------------------------------------------------------------------------
// Router operations
// ---------------------------------------------------------------------------

/** The mining fee a send would pay, sized from the coins it would spend. An upper bound: it
 *  assumes a change output. */
export interface SendFeeEstimate {
  feeSats: number;
  vsize: number;
  inputs: number;
}

/** A wallet in the picker. `network` is absent until Portal can tell, and `"test"` when all it
 *  knows is that the wallet is on some test network. */
export interface WalletListing {
  name: string;
  network?: string;
}

export interface RouterSettings {
  routerId: string;
  walletName: string;
  /** Published in the router's offer, where wallets show it. */
  name: string;
  networkPort: number;
  rpcPort: number;
  socksPort: number;
  controlPort: number;
  fidelityAmount: number;
  fidelityTimelock: number;
  /** sats/vB for the bond transaction; the crate's relay minimum is also the floor. */
  fidelityFeerate: number;
  requiredConfirms: number;
  baseFee: number;
  amountRelativeFeePct: number;
  timeRelativeFeePct: number;
  dataDir?: string;
  /** The chain this router's wallet is on, once Portal has seen it start. */
  network?: string;
}

export interface RouterInitConfig extends RouterSettings {
  walletPassword: string;
  /** Restore the router's wallet from this registered backup file, with `walletPassword` being
   *  the backup's password. */
  restoreSelection?: string;
}

export interface SuggestedRouterPorts {
  networkPort: number;
  rpcPort: number;
}

/** Per-port conflict message, absent when the port is usable. */
export interface RouterPortCheck {
  networkPort?: string;
  rpcPort?: string;
}

export type RouterPhase =
  | { phase: "notConfigured" }
  | { phase: "restoring" }
  | { phase: "initializing" }
  | { phase: "starting" }
  | { phase: "running" }
  | { phase: "stopping" }
  | { phase: "stopped" }
  | { phase: "failed"; message: string };

export interface RouterStatus {
  routerId: string;
  phase: RouterPhase;
  running: boolean;
  torAddress?: string;
  networkPort: number;
  /** Undefined when the wallet file could not be inspected. */
  walletEncrypted?: boolean;
  /** Whether the running router already holds an unspent bond. Undefined while stopped or busy. */
  hasBond?: boolean;
}

export interface FidelityBond {
  outpoint: Outpoint;
  amountSats: number;
  lockTimeHeight: number;
  isSpent: boolean;
  isLocked: boolean;
}

export interface RouterSwapReportSummary {
  swapId: string;
  /** Same `SwapStatus` enum the wallet's own reports carry — one `status_label` serves both. */
  status: SwapStatus;
  startTimestamp: number;
  endTimestamp: number;
  incomingAmountSats: number;
  outgoingAmountSats: number;
  feeEarnedSats: number;
}

export interface RouterSwapReportDetail extends RouterSwapReportSummary {
  network: string;
  swapDurationSeconds: number;
  incomingContractOutpoint?: Outpoint;
  outgoingContractOutpoint?: Outpoint;
  incomingUtxos: ReportUtxo[];
  outgoingUtxos: ReportUtxo[];
  timelock: number;
  deniabilityProof: Record<string, unknown> | null;
}

// ---------------------------------------------------------------------------
// Swap
// ---------------------------------------------------------------------------

export type ProtocolVersion = "legacy" | "taproot";

export interface SwapRequest {
  protocol: ProtocolVersion;
  amountSats: number;
  /** Omitted requests get the backend's two-router route default. */
  routerCount?: number;
  outpoints?: Outpoint[];
  preferredRouters?: string[];
  /** Funding transactions per hop, 1..=10. Omitted keeps the backend default. */
  txCount?: number;
  /** Sats/vB for every transaction in the swap; whole numbers only. */
  feeRate: number;
  /** Pay a third party instead of the wallet: `amountSats` is then what the receiver gets. */
  paymentAddress?: string;
}

export interface SwapFundingEstimate {
  /** The next three are totals across the wallet's funding split, which may be several txs. */
  inputCount: number;
  vbytes: number;
  feeSats: number;
  /** Wallet UTXOs the funding transactions consume. */
  outgoingUtxoCount: number;
  /** Most the last hop can pay back — one UTXO per contract. A ceiling: a router short of
   *  liquidity commits to fewer splits, and the rest of the route inherits that. */
  incomingUtxoCount: number;
  /** Ceiling: a router's funding splits at the full input budget plus one claim per contract. */
  routeMiningFeePerRouterSats: number;
  /** Claiming the incoming contracts at the end of the swap; depends on the protocol. */
  sweepFeeSats: number;
  /** Dust limit of each swept incoming coin. */
  receiveDustSats: number;
}

export interface RouterFeeInfo {
  address: string;
  protocol: string;
  baseFee: number;
  amountRelativeFeePct: number;
  timeRelativeFeePct: number;
  locktime: number;
  estimatedFeeSats: number;
}

export interface SwapSummary {
  swapId: string;
  protocol: string;
  sendAmountSats: number;
  routers: RouterFeeInfo[];
  /** Ceiling: router fees, every hop's funding and sweep reimbursement at its negotiated
   * maximum, and the wallet's own funding tx. The settled cost can only come in under it. */
  totalEstimatedFeeSats: number;
  /** What the wallet gets back if every cost hits its ceiling, so a floor, not a forecast.
   *  Zero when paying a third party: the receiver is paid and nothing comes back. */
  estimatedReceiveAmountSats: number;
  /** `totalEstimatedFeeSats`, split the same way the summary panel and the report split it. */
  routerFeeSats: number;
  miningFeeSats: number;
  /** Present only when this swap pays a third-party receiver. */
  payment?: PaymentQuote;
}

export interface PaymentQuote {
  address: string;
  /** Exact amount the receiver gets. */
  amountSats: number;
}

// Coarse in-memory lifecycle — for live per-router detail, see SwapTrackerProgress/getSwapTracker.

export interface SwapProgress {
  swapId: string;
  startedAt?: number;
  /** Replayed so a remount can restore what only `prepareSwap` ever returned. */
  summary: SwapSummary;
}

/** One of the crate's own per-maker flags, by field name — see `MILESTONE_LABEL` for the prose. */
export interface RouterMilestone {
  key: string;
  done: boolean;
}

/**
 * Where one router is in the protocol, in order. Coarser than the flags below because the taker
 * only flushes its tracker a few times per hop — see `router_stage` in `commands/taker_swap.rs`.
 */
export type RouterStage =
  | "waiting"
  | "negotiated"
  | "handshaking"
  | "confirming"
  | "routed"
  | "key_received"
  | "settled";

export interface RouterProgress {
  address: string;
  stage: RouterStage;
  /** The raw flags behind `stage`; the set differs by protocol (5 taproot / 12 legacy). */
  milestones: RouterMilestone[];
}

export type TrackerPhase =
  | "routers_discovered"
  | "negotiated"
  | "funding_created"
  | "funds_broadcast"
  | "contracts_exchanged"
  | "finalizing"
  | "privkeys_forwarded"
  | "completed"
  | "failed";

// Read straight from the crate's own swap_tracker.cbor — same file the old Electron app polled.
export interface SwapTrackerProgress {
  phase: TrackerPhase;
  sendAmountSats: number;
  routerCount: number;
  failureReason?: string;
  routers: RouterProgress[];
  /**
   * Contract transactions recorded so far on each kind of leg — a hop is funded by up to
   * `txCount` splits, not one transaction, and these fill in as each txid is recorded.
   * `watchonly` is one flat list across every router-to-router leg, with no per-hop grouping.
   */
  outgoingContractTxids: string[];
  incomingContractTxids: string[];
  watchonlyContractTxids: string[];
  /** Our own funding has confirmed — the only signal taproot gives for the first leg. */
  outgoingConfirmed: boolean;
  protocol: ProtocolVersion;
  /** Each leg's real on-chain txids, in route order, from the swap's own confirmation waits.
   *  Filled while this process runs the swap; empty after a restart. */
  fundingWaitTxids: string[][];
  /** Echoed from the tracker, so a remounted page knows a PaySwap without the prepared quote. */
  paymentAddress?: string;
  paymentAmountSats?: number;
}

/** How far a blocking `prepareSwap` has got, read off the crate's own tracker file. */
export interface SwapPreparation {
  phase: "routers_discovered" | "negotiated";
  routerCount: number;
  /** Routers that have agreed terms so far. */
  negotiatedCount: number;
}

export type ContractResolution =
  | "hashlock"
  | "timelock"
  | "key_path"
  | "discarded"
  | "unresolved";

export type RecoveryPhase =
  | "not_started"
  | "preimage_stamped"
  | "swapcoins_persisted"
  | "incoming_recovered"
  | "outgoing_recovered"
  | "cleaned_up";

/** A contract still holding funds, from the wallet's live UTXO set. */
export interface RecoveryContract {
  outpoint: Outpoint;
  amountSats: number;
  /** `hashlock` is spendable once confirmed; `timelock` waits out the refund delay. */
  claimPath: "hashlock" | "timelock";
  confirmations: number;
  /** The first router's payment for a swap this wallet already received — the router's to
   *  claim, not this wallet's to reclaim. */
  routerOwed: boolean;
  /** Blocks still to wait. Timelock only. */
  blocksRemaining?: number;
  /** The refund delay in full, so progress through it can be shown. Timelock only. */
  lockBlocks?: number;
}

/** A contract the recovery loop has already claimed back. */
export interface RecoveredContract {
  contractTxid: string;
  /** The same resolution means opposite things on the two legs. */
  leg: "incoming" | "outgoing";
  resolution: ContractResolution;
  spendingTxid?: string;
}

/**
 * One swap inside the recovery. The crate runs a single recovery loop over every unfinished
 * swap rather than one per swap, so a list of these is a list of what that one recovery is
 * still working through — not a list of concurrent recoveries.
 */
export interface RecoverySummary {
  swapId: string;
  phase: RecoveryPhase;
  failureReason?: string;
  failedAtPhase?: TrackerPhase;
  routerCount: number;
  sendAmountSats: number;
  /** How many of this swap's contracts have already been claimed back. */
  resolvedCount: number;
  active: boolean;
  updatedAt: number;
  /** How the funds came back, once recovery reported it: "hashlock", "timelock". */
  recoveryTypes: string[];
  recoveryTxids: string[];
}

export interface RecoveryStatus {
  active: boolean;
  /** True when OpenSwap's recovery worker is alive; false when an unfinished recovery has no
   * worker; absent when the taker is busy and cannot be inspected without blocking. */
  recoveryRunning?: boolean;
  swapId?: string;
  phase: RecoveryPhase;
  failureReason?: string;
  /** Where the swap stopped — decides whether there is an incoming leg at all. */
  failedAtPhase?: TrackerPhase;
  routerCount: number;
  sendAmountSats: number;
  /** The swap's incoming coins were claimed: it went through, and only the router's payment
   *  is left to settle. */
  swapReceived: boolean;
  /** Contracts still holding funds. */
  pending: RecoveryContract[];
  /** Contracts already claimed back. */
  resolved: RecoveredContract[];
  /** The longest wait left across every pending timelock contract. */
  blocksRemaining?: number;
  /** This wallet's own funds still in contracts. */
  lockedSats: number;
  /** Router payments still in contracts — not this wallet's. */
  routerOwedSats: number;
  updatedAt?: number;
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

/**
 * The first four come from the report file. The last three are synthesised from the swap tracker
 * for a swap that never got a report — the process died mid-swap, so `start_swap`'s failure
 * arm never ran and `cleanup_incomplete` marked it Failed at the next launch without writing one.
 */
export type SwapStatus =
  | "success"
  | "recovery_hashlock"
  | "recovery_timelock"
  | "failed"
  | "interrupted"
  | "recovered"
  | "unfinished";

export interface SwapReportSummary {
  swapId: string;
  status: SwapStatus;
  /** False when the row came from the tracker, so only its coarse figures are known. */
  reported: boolean;
  startTimestamp: number;
  /** Absent on a tracker-derived row: the swap has no recorded end, only a start. */
  endTimestamp?: number;
  outgoingAmountSats: number;
  receivedAmountSats: number;
  feePaidSats: number;
  routersCount: number;
}

export interface ReportRouterFee {
  routerIndex: number;
  baseFeeSats: number;
  amountRelativeFeeSats: number;
  timeRelativeFeeSats: number;
  totalFeeSats: number;
}

/** The coin a completed swap paid the user, recovered from chain rather than the report file. */
export interface SwapUtxo {
  txid: string;
  vout: number;
  amountSats: number;
  address: string | null;
}

export interface SwapReportDetail {
  swapId: string;
  status: SwapStatus;
  network: string;
  swapDurationSeconds: number;
  startTimestamp: number;
  endTimestamp: number;
  errorMessage?: string;
  outgoingAmountSats: number;
  receivedAmountSats: number;
  feePaidSats: number;
  miningFeeSats: number;
  feePercentage: number;
  totalRouterFeesSats: number;
  outgoingUtxos: ReportUtxo[];
  incomingUtxos: ReportUtxo[];
  fundingTxids: string[][];
  routersCount: number;
  routerAddresses: string[];
  routerFeeInfo: ReportRouterFee[];
  /** Paid a third-party receiver rather than back to this wallet. */
  paySwap: boolean;
  /** The exact outpoint verify_deniability checks on-chain. */
  /** The contract UTXO this wallet funded — an outpoint, since a Taproot contract output is not
   *  necessarily vout 0. Absent for swaps whose report carries no deniability proof. */
  outgoingContractOutpoint?: Outpoint;
  /** The contract UTXO the route paid back; also the outpoint `verifyDeniability` checks. */
  incomingContractOutpoint?: Outpoint;
  /** Raw pass-through of the crate's DeniabilityProof (Taproot or Legacy variant) — rendered generically. */
  deniabilityProof: Record<string, unknown> | null;
  /** This swap's entry in the report file, verbatim. */
  raw: string;
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

export interface LogLine {
  line: string;
}

// ---------------------------------------------------------------------------
// Address blocklist
// ---------------------------------------------------------------------------

export interface BlocklistEntry {
  address: string;
  label: string | null;
}

export interface BlocklistImport {
  added: number;
  updated: number;
  /** Rows not imported, by their 1-based line in the uploaded file. */
  rejected: { line: number; address: string; reason: string }[];
}
