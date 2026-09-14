import { getPaths } from "../../api/commands";

export type WalletChoice =
  | { mode: "create"; walletName: string; password: string }
  | { mode: "load"; walletName: string; password?: string }
  | { mode: "restore"; walletName: string; selectionId: string; displayName: string; password?: string };

// Asked of the backend rather than rebuilt here: only it knows the crate's own default and
// whether the user has already pointed the session somewhere else. The native picker also
// needs a real resolved path, not a "~/..." string it has no shell to expand.
export async function getDefaultDataDir(): Promise<string> {
  return (await getPaths()).dataDir;
}

export async function getDefaultWalletsDir(): Promise<string> {
  return (await getPaths()).walletsDir;
}

// Key keeps its old spelling through the .coinswap → .openswap rename: it stores an explicit
// user-chosen path, which renaming the key would silently discard.
const DATA_DIR_KEY = "coinswap_data_dir";

/** User-chosen data dir override ("Change location"), or undefined to use
 * the backend's own default (~/.openswap/taker). */
export function loadDataDir(): string | undefined {
  return localStorage.getItem(DATA_DIR_KEY) ?? undefined;
}

export function saveDataDir(dir: string) {
  localStorage.setItem(DATA_DIR_KEY, dir);
}
