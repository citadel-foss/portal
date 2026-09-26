import { create } from "zustand";
import type { InitResult } from "../api/types";

interface SessionState {
  /** null until the host has been asked; false shows the login gate. Desktop resolves to
   *  true immediately — the process is already the user's. */
  authenticated: boolean | null;
  hasOwner: boolean;
  setAuthenticated: (authenticated: boolean) => void;
  setHasOwner: (hasOwner: boolean) => void;
  /** The connection gate passed: a backend answered a chain query and Tor bootstrapped. */
  connected: boolean;
  setConnected: () => void;
  /** null until Rust has been asked. A reload replaces the page, not the process: the
   *  wallet the user unlocked is still open in Rust, so this is restored rather than
   *  assumed false. `false` means genuinely no wallet — asked and answered. */
  initialized: boolean | null;
  walletName: string | null;
  dataDir: string | null;
  setInitialized: (result: Pick<InitResult, "walletName" | "dataDir">) => void;
  setNotInitialized: () => void;
  reset: () => void;
}

export const useSessionStore = create<SessionState>((set) => ({
  authenticated: null,
  hasOwner: true,
  setAuthenticated: (authenticated) => set({ authenticated }),
  setHasOwner: (hasOwner) => set({ hasOwner }),
  connected: false,
  setConnected: () => set({ connected: true }),
  initialized: null,
  walletName: null,
  dataDir: null,
  setNotInitialized: () => set({ initialized: false }),
  setInitialized: (result) =>
    set({
      initialized: true,
      walletName: result.walletName,
      dataDir: result.dataDir,
    }),
  reset: () =>
    set({ initialized: false, walletName: null, dataDir: null }),
}));
