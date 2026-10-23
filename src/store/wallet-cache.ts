import { create } from "zustand";
import type {
  AddressType,
  Balances,
  NewAddress,
  TxSummary,
  UtxoEntry,
  WalletAddress,
  WalletInfo,
} from "../api/types";

// Survives WalletPage unmount/remount so revisits paint instantly.
export const REFRESH_INTERVAL_MS = 2 * 60 * 1000;
const SYNC_TIME_STORAGE_KEY = "coinswap_wallet_last_sync";

type WalletSyncStatus = "idle" | "syncing" | "synced" | "error";
type WalletHistoryStatus = "idle" | "loading" | "loaded" | "error";

function loadSyncTimes(): Record<string, number> {
  try {
    const parsed = JSON.parse(localStorage.getItem(SYNC_TIME_STORAGE_KEY) ?? "{}") as unknown;
    return parsed && typeof parsed === "object" ? (parsed as Record<string, number>) : {};
  } catch {
    return {};
  }
}

function saveSyncTime(walletKey: string, timestamp: number) {
  try {
    const times = loadSyncTimes();
    times[walletKey] = timestamp;
    localStorage.setItem(SYNC_TIME_STORAGE_KEY, JSON.stringify(times));
  } catch {
    // Timestamp persistence is informational and must not break wallet state.
  }
}

interface WalletCacheState {
  info: WalletInfo | null;
  balances: Balances | null;
  utxos: UtxoEntry[];
  transactions: TxSummary[];
  walletKey: string | null;
  syncStatus: WalletSyncStatus;
  syncError: string | null;
  lastSuccessfulSyncAt: number | null;
  historyStatus: WalletHistoryStatus;
  historyError: string | null;
  /** The Receive panel's address per type and its address list, each stamped with
   *  the `lastSuccessfulSyncAt` it was read at. Both only change when a sync lands, so a
   *  matching stamp means the panel can paint from here instead of asking again. */
  receiveAddresses: Partial<Record<AddressType, { address: NewAddress; syncedAt: number | null }>>;
  addressList: { addresses: WalletAddress[]; syncedAt: number | null } | null;
  setReceiveAddress: (type: AddressType, address: NewAddress, syncedAt: number | null) => void;
  setAddressList: (addresses: WalletAddress[], syncedAt: number | null) => void;
  beginSession: (walletName: string, dataDir: string, restored: boolean) => void;
  setSyncing: () => void;
  setSyncSuccess: (timestamp?: number) => void;
  setSyncError: (message: string) => void;
  setHistoryLoading: () => void;
  setHistoryData: (transactions: TxSummary[]) => void;
  setHistoryError: (message: string) => void;
  setStoredData: (data: { info: WalletInfo; balances: Balances; utxos: UtxoEntry[] }) => void;
  reset: () => void;
}

export const useWalletCacheStore = create<WalletCacheState>((set) => ({
  info: null,
  balances: null,
  utxos: [],
  transactions: [],
  walletKey: null,
  syncStatus: "idle",
  syncError: null,
  lastSuccessfulSyncAt: null,
  historyStatus: "idle",
  historyError: null,
  receiveAddresses: {},
  addressList: null,
  setReceiveAddress: (type, address, syncedAt) =>
    set((state) => ({ receiveAddresses: { ...state.receiveAddresses, [type]: { address, syncedAt } } })),
  setAddressList: (addresses, syncedAt) => set({ addressList: { addresses, syncedAt } }),
  beginSession: (walletName, dataDir, restored) => {
    const walletKey = `${dataDir}/wallets/${walletName}`;
    const lastSuccessfulSyncAt = restored ? Date.now() : (loadSyncTimes()[walletKey] ?? null);
    if (restored) saveSyncTime(walletKey, lastSuccessfulSyncAt);
    set((state) => ({
      walletKey,
      syncStatus: restored ? "synced" : "idle",
      syncError: null,
      lastSuccessfulSyncAt,
      // Never let a process-local snapshot from another wallet flash on screen.
      ...(state.walletKey !== null && state.walletKey !== walletKey
        ? {
            info: null,
            balances: null,
            utxos: [],
            transactions: [],
            historyStatus: "idle" as const,
            historyError: null,
            receiveAddresses: {},
            addressList: null,
          }
        : {}),
    }));
  },
  setSyncing: () => set({ syncStatus: "syncing", syncError: null }),
  setSyncSuccess: (timestamp = Date.now()) =>
    set((state) => {
      if (state.walletKey) saveSyncTime(state.walletKey, timestamp);
      return { syncStatus: "synced", syncError: null, lastSuccessfulSyncAt: timestamp };
    }),
  setSyncError: (message) => set({ syncStatus: "error", syncError: message }),
  setHistoryLoading: () => set({ historyStatus: "loading", historyError: null }),
  setHistoryData: (transactions) => set({ transactions, historyStatus: "loaded", historyError: null }),
  setHistoryError: (message) => set({ historyStatus: "error", historyError: message }),
  // Wallet balances and UTXOs are backed by the encrypted wallet file. Transaction
  // history is rebuilt from the chain backend and has a separate loading state.
  setStoredData: (data) => set(data),
  reset: () => set({
    info: null,
    balances: null,
    utxos: [],
    transactions: [],
    walletKey: null,
    syncStatus: "idle",
    syncError: null,
    lastSuccessfulSyncAt: null,
    historyStatus: "idle",
    historyError: null,
    receiveAddresses: {},
    addressList: null,
  }),
}));
