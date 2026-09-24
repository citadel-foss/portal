import { create } from "zustand";
import { checkBackend, getChainBackend } from "../api/commands";
import type { BackendStatus, ChainBackendConfig } from "../api/types";

/**
 * Whether the chain backend adopted at the connection gate is still answering, re-probed on a
 * timer so the header can say so rather than implying it.
 *
 * `checkBackend` is a real chain query over a fresh connection, not a cached flag, so the
 * interval is generous: liveness is not something that flips between frames, and each probe
 * costs a TLS (and on Tor, a circuit) handshake.
 */
const PROBE_INTERVAL_MS = 20_000;

/** Our signet's challenge script, hex, exactly as bitcoind prints it in `getblockchaininfo`. */
const OUR_SIGNET_CHALLENGE = "0014a3ec9c731da66d9725d54947aede5c830623f33d";

/** Consulted only when no challenge is available, which is every Electrum session: Electrum
 *  synthesizes its chain info from the header tip, and all signets share a genesis hash, so
 *  the endpoint is the only remaining evidence of which signet this is. */
const OUR_INFRASTRUCTURE = /(^|\.)citadelfoss\.xyz$/;

export const FAUCET_URL = "https://faucet.citadelfoss.xyz";

interface ConnectionState {
  /** Fixed for the session — adopted at the gate and never re-read. */
  config: ChainBackendConfig | null;
  /** null until the first probe answers, so "unknown" is never rendered as "down". */
  status: BackendStatus | null;
  refresh: () => Promise<void>;
}

export const useConnectionStore = create<ConnectionState>((set) => ({
  config: null,
  status: null,
  refresh: async () => {
    if (!useConnectionStore.getState().config) {
      await getChainBackend()
        .then((config) => set({ config }))
        .catch(() => {});
    }
    // A probe that throws is the transport failing, which is itself an answer: the backend is
    // not reachable. Anything else would leave a dead node showing as live forever.
    await checkBackend()
      .then((status) => set({ status }))
      .catch(() => set({ status: null }));
  },
}));

/** Starts the probe timer once for the app's lifetime. Safe to call from several places. */
let timer: ReturnType<typeof setInterval> | null = null;
export function watchConnection() {
  void useConnectionStore.getState().refresh();
  if (timer) return;
  timer = setInterval(() => void useConnectionStore.getState().refresh(), PROBE_INTERVAL_MS);
}

/** Electrum and Core disagree on the name of the same chain ("main"/"test" vs the BIP70
 *  "bitcoin"/"testnet"), so which backend you picked must not change what the header says. */
export function chainName(status: BackendStatus | null): string | null {
  const chain = status?.chain;
  if (chain === undefined) return null;
  if (chain === "main") return "bitcoin";
  if (chain === "test") return "testnet";
  return chain;
}

function endpointHost(config: ChainBackendConfig): string {
  const raw = config.kind === "electrum" ? config.electrum.url : (config.node?.host ?? "");
  const noScheme = raw.split("://").pop() ?? "";
  return noScheme.split("/")[0].replace(/:\d+$/, "").toLowerCase();
}

/** Our own signet, as opposed to any other signet a user could point the app at. */
export function isOurSignet(state: ConnectionState): boolean {
  if (!state.status?.reachable || chainName(state.status) !== "signet") return false;
  return state.status.signetChallenge !== undefined
    ? state.status.signetChallenge.toLowerCase() === OUR_SIGNET_CHALLENGE
    : state.config !== null && OUR_INFRASTRUCTURE.test(endpointHost(state.config));
}
