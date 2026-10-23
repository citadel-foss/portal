import { create } from "zustand";
import type { UtxoEntry } from "../api/types";

const STORAGE_KEY = "coinswap_pending_sends";

/**
 * A send this app broadcast, recorded locally because the wallet's own history cannot always
 * show it.
 *
 * The Electrum backend builds history per output: an output that is ours is only reported when
 * the transaction is *not* a spend of ours, mirroring Core's "paying yourself is not a payment"
 * rule (`wallet/blockchain/electrum.rs`). A payment to an address from this wallet's own Receive
 * panel therefore has no reportable output at all — every output is both ours and part of a send
 * — so it never appears in `get_transactions`, mined or not, even though its coins are right
 * there in the UTXO list.
 *
 * Recording the txid at broadcast is what makes a send visible immediately and keeps a self-send
 * visible at all.
 */
interface PendingSend {
  txid: string;
  /** The wallet's on-disk path, so a record can never surface under a different wallet. */
  walletPath: string;
  address: string;
  amountSats: number;
  feeRate: number;
  /** Unix seconds. */
  createdAt: number;
}

function load(): PendingSend[] {
  try {
    const parsed = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "[]") as unknown;
    return Array.isArray(parsed) ? (parsed as PendingSend[]) : [];
  } catch {
    return [];
  }
}

function save(sends: PendingSend[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(sends));
  } catch {
    // Losing the record only costs the pending row; the transaction is on-chain either way.
  }
}

interface PendingSendsState {
  sends: PendingSend[];
  record: (send: PendingSend) => void;
  /**
   * Drops anything now confirmed. A send always leaves at least one output this wallet owns —
   * the change, or the payment itself on a self-send — so a matching UTXO's confirmation count
   * is the transaction's own.
   */
  reconcile: (walletPath: string, utxos: UtxoEntry[], historyTxids: string[]) => void;
}

export const usePendingSendsStore = create<PendingSendsState>((set) => ({
  sends: load(),
  record: (send) =>
    set((s) => {
      const sends = [send, ...s.sends.filter((x) => x.txid !== send.txid)];
      save(sends);
      return { sends };
    }),
  reconcile: (walletPath, utxos, historyTxids) =>
    set((s) => {
      const confirmed = new Set(
        utxos.filter((u) => u.confirmations > 0).map((u) => u.txid),
      );
      const inHistory = new Set(historyTxids);
      const sends = s.sends.filter(
        (x) =>
          x.walletPath !== walletPath ||
          (!confirmed.has(x.txid) && !inHistory.has(x.txid)),
      );
      if (sends.length !== s.sends.length) save(sends);
      return { sends };
    }),
}));

/** Confirmations for a recorded send, from whichever of its outputs this wallet owns. */
export function sendConfirmations(txid: string, utxos: UtxoEntry[]): number | null {
  const own = utxos.filter((u) => u.txid === txid);
  if (own.length === 0) return null;
  return Math.max(...own.map((u) => u.confirmations));
}
