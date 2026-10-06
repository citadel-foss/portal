import { create } from "zustand";
import { getRouterTransactions, listRouterUtxos } from "../api/commands";
import type { AddressType, NewAddress, TxSummary, UtxoEntry } from "../api/types";

// Survives the Tx tab unmounting — a tab switch, or leaving the workspace and coming back —
// so a revisit paints the last snapshot at once and refreshes behind it. Memory only: a
// sign-out reloads the page and takes it with the rest of the session.

/** The same window the Tx tab has always asked for. */
const TRANSACTION_WINDOW = 30;

export interface RouterWalletSnapshot {
  utxos: UtxoEntry[];
  transactions: TxSummary[];
  /** Per type, so switching Taproot/SegWit shows the last address at once while it is re-checked. */
  addresses: Partial<Record<AddressType, NewAddress>>;
}

const EMPTY: RouterWalletSnapshot = { utxos: [], transactions: [], addresses: {} };

interface RouterWalletCacheState {
  byRouter: Record<string, RouterWalletSnapshot>;
  setLists: (routerId: string, utxos: UtxoEntry[], transactions: TxSummary[]) => void;
  setAddress: (routerId: string, type: AddressType, address: NewAddress) => void;
}

export const useRouterWalletCacheStore = create<RouterWalletCacheState>((set) => ({
  byRouter: {},
  setLists: (routerId, utxos, transactions) =>
    set((state) => ({
      byRouter: {
        ...state.byRouter,
        [routerId]: { ...(state.byRouter[routerId] ?? EMPTY), utxos, transactions },
      },
    })),
  setAddress: (routerId, type, address) =>
    set((state) => {
      const current = state.byRouter[routerId] ?? EMPTY;
      // Same address back is the common case; returning the old state spares every subscriber a
      // re-render.
      if (current.addresses[type]?.address === address.address) return state;
      return {
        byRouter: {
          ...state.byRouter,
          [routerId]: { ...current, addresses: { ...current.addresses, [type]: address } },
        },
      };
    }),
}));

export function useRouterWalletSnapshot(routerId: string): RouterWalletSnapshot {
  return useRouterWalletCacheStore((s) => s.byRouter[routerId] ?? EMPTY);
}

/** Re-reads what the router's wallet already holds. Does not sync — see `syncRouterWallet`. */
export async function refreshRouterWallet(routerId: string): Promise<void> {
  const [utxos, transactions] = await Promise.all([
    listRouterUtxos(routerId),
    getRouterTransactions(routerId, TRANSACTION_WINDOW, 0),
  ]);
  useRouterWalletCacheStore.getState().setLists(routerId, utxos, transactions);
}
