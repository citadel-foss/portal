/** Desktop host: Tauri's per-command IPC and its window event channel. */
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";

import type { Host, RestoreSelection } from "./host-contract";

/**
 * This bundle talks to Tauri's IPC, which only exists inside the app window. Opened in an
 * ordinary browser it would otherwise fail deep inside the Tauri client with
 * "Cannot read properties of undefined (reading 'invoke')", which says nothing about the
 * actual mistake.
 */
function assertInsideTauri() {
  if (!("__TAURI_INTERNALS__" in window)) {
    throw new Error(
      "This is the desktop build, which only runs inside the Portal app window. " +
        "For the browser, run `npm run web:dev` and open http://localhost:1430 instead.",
    );
  }
}

export const host: Host = {
  invoke: (name, args) => {
    assertInsideTauri();
    return tauriInvoke(name, args);
  },
  subscribe: async (event, handler) => {
    const stop = await listen(event, (e) => handler(e.payload as never));
    return stop;
  },
  capabilities: {
    nativeFilePicker: true,
    localDashboardImport: true,
    // The desktop app owns its process; a browser session cannot stop the server.
    canQuit: true,
  },
  // Desktop commands do not go through the operations journal, so nothing is ever blocked
  // and there is nothing to settle.
  operations: {
    blocking: async () => [],
    reconcile: async () => {},
    acknowledge: async () => {},
  },
  // Rust owns whether the app is signed in, so a webview reload keeps it and a relaunch asks
  // again — the same lifetime a web session has against its server.
  session: {
    restore: () => host.invoke("auth_session"),
    login: (password) => host.invoke("auth_login", { password }),
    claim: async (password) => {
      await host.invoke("auth_claim", { password });
      await host.invoke("auth_login", { password });
    },
    logout: () => host.invoke("auth_logout"),
  },
  openExternal: (url) => openUrl(url),
  pickDirectory: async (defaultPath) => {
    const path = await openDialog({ directory: true, defaultPath });
    return typeof path === "string" ? path : null;
  },
  pickFile: async (defaultPath) => {
    const path = await openDialog({ multiple: false, defaultPath });
    return typeof path === "string" ? path : null;
  },
  // Rust owns the picker so the renderer only ever sees the opaque selection ID.
  selectBackup: () => tauriInvoke<RestoreSelection>("choose_restore_backup"),
  createBackup: (password) => tauriInvoke<string>("backup_wallet", { password }),
};
