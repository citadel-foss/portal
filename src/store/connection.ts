import { create } from "zustand";
import { checkBackend, getChainBackend } from "../api/commands";
import type { BackendStatus, ChainBackendConfig } from "../api/types";

/** Whether the chain backend adopted at the connection gate answered the shell's initial read. */

/** Our signet's challenge script, hex, exactly as bitcoind prints it in `getblockchaininfo`. */
const OUR_SIGNET_CHALLENGE = "0014a3ec9c731da66d9725d54947aede5c830623f33d";

/** Consulted only when no challenge is available, which is every Electrum session: Electrum
 *  synthesizes its chain info from the header tip, and all signets share a genesis hash, so
 *  the endpoint is the only remaining evidence of which signet this is. */
const OUR_INFRASTRUCTURE = /(^|\.)(citadelfoss\.xyz|openswap\.live)$/;

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

/** The status now, probing first if nothing has answered yet: a list filtered by chain before
 *  the first probe lands would show every wallet on every chain. */
export async function currentStatus(): Promise<BackendStatus | null> {
  if (!useConnectionStore.getState().status) await useConnectionStore.getState().refresh();
  return useConnectionStore.getState().status;
}

/** Reads the backend and network once for header state, filtering and explorer links. Operations
 * perform their own reachability checks, so keeping a polling connection open adds no guard. */
export function loadConnectionStatus() {
  return useConnectionStore.getState().refresh();
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

/** Whether a wallet or router recorded on `network` belongs to the chain this session reached.
 *  `"test"` means Portal only knows it is on some test network. One it cannot place yet stays
 *  visible: hiding it would make a wallet disappear for no reason the user could see. */
export function onCurrentChain(network: string | undefined, status: BackendStatus | null): boolean {
  const chain = chainName(status);
  if (!network || !chain) return true;
  if (network === "test") return chain !== "bitcoin" && chain !== "regtest";
  return network === chain;
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
