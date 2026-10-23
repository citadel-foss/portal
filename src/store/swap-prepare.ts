import { create } from "zustand";
import { prepareSwap } from "../api/commands";
import { isAppError, type SwapRequest, type SwapSummary } from "../api/types";
import { useWalletCacheStore } from "./wallet-cache";

/**
 * A swap preparation and its result, kept outside the Swap page. The backend call keeps running
 * when the page unmounts, so its outcome has to land somewhere a returning page can find it.
 * Stamped with the wallet it belongs to: a review is only ever valid for that wallet.
 */
interface SwapPrepareState {
  walletPath: string | null;
  /** Unix seconds the running preparation started; null when none is running. */
  since: number | null;
  review: SwapSummary | null;
  /** When the review arrived, for its expiry. */
  reviewAt: number | null;
  error: string | null;
  run: (request: SwapRequest) => void;
  clearReview: () => void;
  clearError: () => void;
}

const currentWallet = () => useWalletCacheStore.getState().info?.walletPath ?? null;

const useSwapPrepareStore = create<SwapPrepareState>((set, get) => ({
  walletPath: null,
  since: null,
  review: null,
  reviewAt: null,
  error: null,
  run: (request) => {
    if (get().since !== null) return;
    const walletPath = currentWallet();
    set({ walletPath, since: Math.floor(Date.now() / 1000), review: null, reviewAt: null, error: null });
    void prepareSwap(request)
      .then((review) => {
        if (get().walletPath === walletPath) set({ since: null, review, reviewAt: Date.now() });
      })
      .catch((e) => {
        if (get().walletPath !== walletPath) return;
        set({ since: null, error: (isAppError(e) ? e.message : null) ?? "Failed to start swap." });
      });
  },
  clearReview: () => set({ review: null, reviewAt: null }),
  clearError: () => set({ error: null }),
}));

/** The store's state, or nothing if it belongs to a wallet other than the open one. */
export function useSwapPrepare() {
  const state = useSwapPrepareStore();
  const walletPath = useWalletCacheStore((s) => s.info?.walletPath ?? null);
  const mine = state.walletPath === walletPath;
  return {
    since: mine ? state.since : null,
    review: mine ? state.review : null,
    reviewAt: mine ? state.reviewAt : null,
    error: mine ? state.error : null,
    run: state.run,
    clearReview: state.clearReview,
    clearError: state.clearError,
  };
}
