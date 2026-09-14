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
    // The desktop app owns its process; a browser session cannot stop the server.
    canQuit: true,
    // The process belongs to whoever launched it; a second password here would protect
    // nothing that the OS account does not already protect.
    requiresLogin: false,
  },
  session: {
    restore: async () => ({ authenticated: true, hasOwner: true }),
    login: async () => {},
    claim: async () => {},
    logout: async () => {},
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
};
