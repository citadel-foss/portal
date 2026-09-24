import { create } from "zustand";
import type { TxSummary } from "../api/types";

/**
 * Money moving in or out of the open wallet, announced once.
 *
 * Derived by diffing successive history loads rather than pushed by the backend: the crate
 * reports no such event, and the sync that rebuilds history is already the only moment the
 * app learns a transaction exists.
 */
const DISMISS_AFTER_MS = 12_000;

export interface TxNotice {
  id: number;
  txid: string;
  amountSats: number;
  incoming: boolean;
}

interface TxNoticeState {
  notices: TxNotice[];
  observe: (transactions: TxSummary[]) => void;
  dismiss: (id: number) => void;
  reset: () => void;
}

/**
 * null until the first history load lands. That one is the baseline, not news — without this
 * every wallet open would fire a notification for each of the last ten transactions.
 */
let seen: Set<string> | null = null;
let nextId = 0;
const timers = new Map<number, ReturnType<typeof setTimeout>>();

// A self-send appears as two entries sharing a txid, so the amount is part of the identity.
const key = (tx: TxSummary) => `${tx.txid}:${tx.amountSats}`;

export const useTxNoticeStore = create<TxNoticeState>((set) => ({
  notices: [],
  observe: (transactions) => {
    const keys = new Set(transactions.map(key));
    if (seen === null) {
      seen = keys;
      return;
    }
    const fresh = transactions.filter((tx) => !seen!.has(key(tx)));
    // Union rather than replacement: the history window is the last ten, so a transaction
    // that scrolls out of it must not be announced again if it ever comes back.
    for (const k of keys) seen.add(k);
    if (fresh.length === 0) return;

    const notices = fresh.map((tx) => ({
      id: ++nextId,
      txid: tx.txid,
      amountSats: Math.abs(tx.amountSats),
      incoming: tx.amountSats >= 0,
    }));
    for (const notice of notices) {
      timers.set(
        notice.id,
        setTimeout(() => useTxNoticeStore.getState().dismiss(notice.id), DISMISS_AFTER_MS),
      );
    }
    set((s) => ({ notices: [...s.notices, ...notices] }));
  },
  dismiss: (id) => {
    const timer = timers.get(id);
    if (timer) {
      clearTimeout(timer);
      timers.delete(id);
    }
    set((s) => ({ notices: s.notices.filter((n) => n.id !== id) }));
  },
  reset: () => {
    for (const timer of timers.values()) clearTimeout(timer);
    timers.clear();
    // Back to "no baseline": the next wallet's first history load is its own baseline, and
    // its transactions are not news either.
    seen = null;
    set({ notices: [] });
  },
}));
