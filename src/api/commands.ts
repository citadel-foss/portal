import { invoke } from "./transport";
import type {
  SwapPreparation,
  AddressType,
  AddressValidation,
  Balances,
  BackendStatus,
  ChainBackendConfig,
  FeeEstimate,
  FidelityBond,
  InitConfig,
  InitResult,
  LogLine,
  Router,
  RouterInitConfig,
  RouterSwapReportDetail,
  RouterSwapReportSummary,
  RouterPortCheck,
  RouterSettings,
  RouterStatus,
  NewAddress,
  OfferBookView,
  Outpoint,
  Paths,
  PriceEstimate,
  ProtocolVersion,
  RecoveryStatus,
  RecoverySummary,
  SendResult,
  SwapFundingEstimate,
  SwapProgress,
  SwapReportDetail,
  SwapReportSummary,
  SwapUtxo,
  SwapRequest,
  SwapSummary,
  SwapTrackerProgress,
  SuggestedRouterPorts,
  TorStatus,
  TxSummary,
  UtxoEntry,
  RouterDefaults,
  SessionState,
  WalletInfo,
} from "./types";

/** What a new router starts with, straight from the protocol crate's defaults. */
export function getRouterDefaults(): Promise<RouterDefaults> {
  return invoke("get_router_defaults");
}

export function getChainBackend(): Promise<ChainBackendConfig> {
  return invoke("get_chain_backend");
}

export function setChainBackend(config: ChainBackendConfig): Promise<void> {
  return invoke("set_chain_backend", { config });
}

/**
 * Probes a chain backend with a real chain query. Pass `config` to test unsaved edits;
 * omit it to probe whichever backend this session is using.
 */
export function checkBackend(config?: ChainBackendConfig): Promise<BackendStatus> {
  return invoke("check_backend", { config: config ?? null });
}

/** Starts Portal's own Tor if it isn't up yet; the result carries the ports it landed on. */
export function checkTor(): Promise<TorStatus> {
  return invoke("check_tor");
}

/**
 * Makes the running Tor discard its current attempt and bootstrap again. Tor itself is not
 * restarted — it cannot be, within one process — so this is the only real retry there is.
 */
export function restartTorBootstrap(): Promise<void> {
  return invoke("restart_tor_bootstrap");
}

/** Confirms a quit the user was warned about; the process exits once teardown finishes. */
export function quitApp(): Promise<void> {
  return invoke("quit_app");
}

export function listWallets(dataDir?: string): Promise<string[]> {
  return invoke("list_wallets", { dataDir });
}

export function initWallet(config: InitConfig): Promise<InitResult> {
  return invoke("init_taker", { config });
}

/** Releases the wallet so a different one can be unlocked, without stopping Portal. Refused
 *  while a swap is running; routers keep running either way. */
export function lockWallet(): Promise<void> {
  return invoke("shutdown_taker");
}

/** Where this host keeps wallet data. Never derived in the frontend: only the backend knows
 *  the crate's default and whether the session already points somewhere else. */
export function getPaths(): Promise<Paths> {
  return invoke("get_paths");
}

export function getSessionState(): Promise<SessionState> {
  return invoke("get_session_state");
}

export function getWalletInfo(): Promise<WalletInfo> {
  return invoke("get_wallet_info");
}

export function restoreWallet(
  walletName: string,
  socksPort: number | undefined,
  selectionId: string,
  password?: string,
  dataDir?: string,
): Promise<void> {
  return invoke("restore_wallet", {
    dataDir,
    walletName,
    socksPort,
    selectionId,
    password,
  });
}

export function backupWallet(
  password: string,
): Promise<string> {
  return invoke("backup_wallet", { password });
}

// ---------------------------------------------------------------------------
// Wallet operations
// ---------------------------------------------------------------------------

export function getBalances(): Promise<Balances> {
  return invoke("get_balances");
}

export async function estimateSwapFunding(
  amountSats: number,
  protocol: ProtocolVersion,
  outpoints?: Outpoint[],
  txCount?: number,
): Promise<SwapFundingEstimate> {
  return invoke("estimate_swap_funding", { amountSats, protocol, outpoints, txCount });
}

export function getNewAddress(addressType: AddressType): Promise<NewAddress> {
  return invoke("get_new_address", { addressType });
}

/** The chain-querying half of address issuance; slow, so it runs after the panel has painted. */
export function verifyLastAddress(addressType: AddressType): Promise<NewAddress> {
  return invoke("verify_last_address", { addressType });
}

export function validateAddress(address: string): Promise<AddressValidation> {
  return invoke("validate_address", { address });
}

export function getTransactions(
  count?: number,
  skip?: number,
): Promise<TxSummary[]> {
  return invoke("get_transactions", { count, skip });
}

export function listUtxos(): Promise<UtxoEntry[]> {
  return invoke("list_utxos");
}

export function sendToAddress(
  address: string,
  amountSats: number,
  feeRate?: number,
  outpoints?: Outpoint[],
): Promise<SendResult> {
  return invoke("send_to_address", { address, amountSats, feeRate, outpoints });
}

export function syncWallet(): Promise<void> {
  return invoke("sync_wallet");
}

export function estimateFees(): Promise<FeeEstimate> {
  return invoke("estimate_fees");
}

export function getBtcPrice(): Promise<PriceEstimate> {
  return invoke("get_btc_price");
}

// ---------------------------------------------------------------------------
// Market / offerbook
// ---------------------------------------------------------------------------

export function getOffers(): Promise<OfferBookView> {
  return invoke("get_offers");
}

export function syncOfferbook(): Promise<void> {
  return invoke("sync_offerbook");
}

export function pollRouter(address: string): Promise<Router> {
  return invoke("poll_maker", { address });
}

export function removeRouter(address: string): Promise<boolean> {
  return invoke("remove_maker", { address });
}

// ---------------------------------------------------------------------------
// Router operations
// ---------------------------------------------------------------------------

export function listRouters(): Promise<RouterSettings[]> {
  return invoke("list_makers");
}

export function listDashboardImports(): Promise<RouterSettings[]> {
  return invoke("list_dashboard_imports");
}

export function importDashboardRouters(routerIds: string[]): Promise<RouterSettings[]> {
  return invoke("import_dashboard_makers", { routerIds });
}

export function getRouterStatus(routerId: string): Promise<RouterStatus> {
  return invoke("get_maker_status", { routerId });
}

export function initRouter(config: RouterInitConfig): Promise<RouterStatus> {
  return invoke("init_maker", { config });
}

export function updateRouterSettings(routerId: string, settings: RouterSettings): Promise<RouterSettings> {
  return invoke("update_maker_settings", { routerId, settings });
}

export function startRouter(routerId: string, walletPassword?: string): Promise<void> {
  return invoke("start_maker", { routerId, walletPassword });
}

export function stopRouter(routerId: string): Promise<void> {
  return invoke("stop_maker", { routerId });
}

export function getRouterInfo(routerId: string): Promise<WalletInfo> {
  return invoke("get_maker_info", { routerId });
}

export function getSavedRouterSettings(routerId: string): Promise<RouterSettings | null> {
  return invoke("get_saved_maker_settings", { routerId });
}

export function clearRouterSettings(routerId: string): Promise<void> {
  return invoke("clear_maker_settings", { routerId });
}

export function getSuggestedRouterPorts(): Promise<SuggestedRouterPorts> {
  return invoke("get_suggested_maker_ports");
}

/** Verifies a router's listener ports are bindable and unclaimed. Empty result means both are fine. */
export function checkRouterPorts(networkPort: number, rpcPort: number): Promise<RouterPortCheck> {
  return invoke("check_maker_ports", { networkPort, rpcPort });
}

export function getRouterBalances(routerId: string): Promise<Balances> {
  return invoke("get_maker_balances", { routerId });
}

export function listRouterUtxos(routerId: string): Promise<UtxoEntry[]> {
  return invoke("list_maker_utxos", { routerId });
}

export function getRouterTransactions(routerId: string, count?: number, skip?: number): Promise<TxSummary[]> {
  return invoke("get_maker_transactions", { routerId, count, skip });
}

/** Spend from a router's own wallet. Durable, like the taker spend it mirrors. */
export function sendRouterToAddress(
  routerId: string,
  address: string,
  amountSats: number,
  feeRate?: number,
  outpoints?: Outpoint[],
): Promise<SendResult> {
  return invoke("send_maker_to_address", { routerId, address, amountSats, feeRate, outpoints });
}

export function getRouterNewAddress(routerId: string, addressType: AddressType): Promise<NewAddress> {
  return invoke("get_maker_new_address", { routerId, addressType });
}

export function syncRouterWallet(routerId: string): Promise<void> {
  return invoke("sync_maker_wallet", { routerId });
}

export function listRouterFidelityBonds(routerId: string): Promise<FidelityBond[]> {
  return invoke("list_maker_fidelity_bonds", { routerId });
}

export function listRouterSwapReports(routerId: string): Promise<RouterSwapReportSummary[]> {
  return invoke("list_maker_swap_reports", { routerId });
}

export function getRouterSwapReport(routerId: string, swapId: string): Promise<RouterSwapReportDetail> {
  return invoke("get_maker_swap_report", { routerId, swapId });
}

export function verifyRouterDeniability(routerId: string, swapId: string): Promise<boolean> {
  return invoke("verify_maker_deniability", { routerId, swapId });
}

export function getRouterLogs(routerId: string, lines?: number): Promise<LogLine[]> {
  return invoke("get_maker_logs", { routerId, lines });
}

// ---------------------------------------------------------------------------
// Swap
// ---------------------------------------------------------------------------

export function prepareSwap(request: SwapRequest): Promise<SwapSummary> {
  return invoke("prepare_swap", { request });
}

// Result arrives via the "swap://finished" / "swap://failed" events (see
// src-tauri/src/commands/swap.rs); poll getSwapProgress for a snapshot.
export function startSwap(swapId: string): Promise<void> {
  return invoke("start_swap", { swapId });
}

export function getSwapProgress(): Promise<SwapProgress | null> {
  return invoke("get_swap_progress");
}

// Live per-router detail read straight from swap_tracker.cbor — poll this every couple seconds
// while a swap is running, same cadence as the old Electron app's disk-read poll.
/**
 * Progress for a `prepareSwap` still in flight. Safe to poll while it blocks — it reads the
 * tracker file, not the taker. `since` is the unix second preparation began.
 */
export function getSwapPreparation(since: number): Promise<SwapPreparation | null> {
  return invoke("get_swap_preparation", { since });
}

/** Omit `swapId` for the running swap; pass one to read a swap the app has already released. */
export function getSwapTracker(swapId?: string): Promise<SwapTrackerProgress | null> {
  return invoke("get_swap_tracker", { swapId: swapId ?? null });
}

export function recoverSwap(): Promise<void> {
  return invoke("recover_swap");
}

/** Omit `swapId` for the newest unfinished swap, which is what the summary poll wants. */
export function getRecoveryStatus(swapId?: string): Promise<RecoveryStatus> {
  return invoke("get_recovery_status", { swapId });
}

/** Every swap the one recovery loop is still working through, newest first. */
export function listRecoveries(): Promise<RecoverySummary[]> {
  return invoke("list_recoveries");
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

export function listSwapReports(): Promise<SwapReportSummary[]> {
  return invoke("list_swap_reports");
}

export function getSwapReport(swapId: string): Promise<SwapReportDetail> {
  return invoke("get_swap_report", { swapId });
}

/** Walks the chain to find the coin this swap paid out; null when the sweep isn't found. */
export function getIncomingSwapUtxo(swapId: string): Promise<SwapUtxo | null> {
  return invoke("get_incoming_swap_utxo", { swapId });
}

export function verifyDeniability(swapId: string): Promise<boolean> {
  return invoke("verify_deniability", { swapId });
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

export function getLogs(lines?: number): Promise<LogLine[]> {
  return invoke("get_logs", { lines });
}
