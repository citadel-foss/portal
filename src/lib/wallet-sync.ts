import { checkBackend, getBalances, getTransactions, getWalletInfo, listUtxos, syncWallet } from "../api/commands";
import { useTxNoticeStore } from "../store/tx-notifications";
import { useWalletCacheStore } from "../store/wallet-cache";

let hydrateInFlight: Promise<void> | null = null;
let refreshInFlight: Promise<void> | null = null;

/** Read the encrypted wallet file's persisted UTXO snapshot; no network I/O. */
export function hydrateWalletCache(): Promise<void> {
  if (hydrateInFlight) return hydrateInFlight;
  hydrateInFlight = Promise.all([getWalletInfo(), getBalances(), listUtxos()])
    .then(([info, balances, utxos]) => {
      useWalletCacheStore.getState().setStoredData({ info, balances, utxos });
    })
    .finally(() => {
      hydrateInFlight = null;
    });
  return hydrateInFlight;
}

/** Run one coalesced Electrum sync, then refresh the visible wallet snapshot. */
export function refreshWalletCache(): Promise<void> {
  if (refreshInFlight) return refreshInFlight;

  refreshInFlight = (async () => {
    const cache = useWalletCacheStore.getState();
    cache.setSyncing();
    let slowTimer: ReturnType<typeof setTimeout> | undefined;
    try {
      // Avoid entering openswap's retry-forever sync while the endpoint is already known
      // to be unreachable — that loop only exits on success or app shutdown. Probes the
      // saved backend over the same route the sync itself takes.
      const reachability = await checkBackend();
      if (!reachability.reachable) {
        throw new Error(reachability.error ?? "The chain backend is unreachable.");
      }

      slowTimer = setTimeout(() => {
        if (useWalletCacheStore.getState().syncStatus === "syncing") {
          useWalletCacheStore
            .getState()
            .setSyncError("Wallet sync is taking longer than expected. Spending remains disabled.");
        }
      }, 15_000);

      await syncWallet();
      clearTimeout(slowTimer);
      cache.setSyncSuccess();

      await hydrateWalletCache();
      cache.setHistoryLoading();
      try {
        // Electrum reconstructs this from watched-script history and fetches
        // transaction inputs, so keep the initial window intentionally small.
        const history = await getTransactions(10, 0);
        cache.setHistoryData(history);
        // After the cache, so the wallet page a notification links to already has the row.
        useTxNoticeStore.getState().observe(history);
      } catch (error) {
        cache.setHistoryError(
          (error as { message?: string })?.message ?? "Transaction history could not be loaded.",
        );
      }
    } catch (error) {
      if (slowTimer) clearTimeout(slowTimer);
      cache.setSyncError((error as { message?: string })?.message ?? "Wallet synchronization failed.");
    }
  })().finally(() => {
    refreshInFlight = null;
  });

  return refreshInFlight;
}

/** Always hydrate before starting network work so persisted data can paint first. */
export async function startWalletSynchronization(): Promise<void> {
  await hydrateWalletCache().catch(() => {});
  await refreshWalletCache();
}
