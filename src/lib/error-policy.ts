import { isAppError, type AppError, type ErrorClass } from "../api/types";

/**
 * How to present a failed command.
 *
 * The class comes from the backend, decided once beside the error codes, so this file holds
 * presentation only — never a second copy of the classification.
 */
interface Presentation {
  /** False for the ones nobody should be interrupted over. */
  show: boolean;
  tone: "warning" | "error";
  message: string;
  /** Extra sentence naming what to do, where the code implies one specific action. */
  guidance?: string;
}

/** Messages worth replacing, because the backend's own text is a crate internal. */
const REWRITTEN: Partial<Record<string, string>> = {
  WALLET_LOAD_FAILED: "That file isn't a Portal wallet, or it's damaged.",
  INTERNAL: "Something went wrong inside Portal.",
  STATE_POISONED: "Something went wrong inside Portal.",
  IO: "Portal couldn't read or write a file it needed.",
};

const GUIDANCE: Partial<Record<string, string>> = {
  TOR_UNREACHABLE: "Quit and reopen Portal to start a fresh Tor.",
  RPC_AUTH_FAILED: "Check the RPC username and password on the connection screen.",
  ZMQ_UNREACHABLE: "Your node's ZMQ publisher isn't reachable. Check its port in the node settings.",
  WALLET_LOAD_FAILED: "Try restoring from a backup instead.",
  INSECURE_DATA_DIRECTORY: "Only your user account may read the wallet folder.",
  BACKEND_ROUTE_CHANGED: "Reconnect before continuing.",
  WALLET_OPEN_ELSEWHERE: "Close it in the other Portal, or open it there.",
  CONTRACTS_BROADCASTED: "Your funds are in contracts. Portal is reclaiming them on the Recovery page.",
  INTERNAL: "If it keeps happening, the Logs page has the detail.",
  STATE_POISONED: "Restarting Portal should clear it.",
  IO: "If it keeps happening, the Logs page has the detail.",
};

const TONE: Record<ErrorClass, "warning" | "error"> = {
  transient: "warning",
  "needs-input": "error",
  "needs-restart": "error",
  "unsafe-to-retry": "warning",
  silent: "warning",
  bug: "error",
};

export function present(error: unknown, fallback: string): Presentation {
  const appError = isAppError(error) ? (error as AppError) : null;
  // An error from before the backend carried a class, or a plain JS throw: treat it the way
  // everything was treated before, rather than guessing.
  // Checked against the table rather than trusted: `isAppError` only proves there is a code,
  // so a class this build has never heard of would index `TONE` to `undefined` and produce a
  // toast with no kind and no timeout — one that never goes away.
  const raw = appError?.class;
  const cls = raw && Object.prototype.hasOwnProperty.call(TONE, raw) ? raw : undefined;
  if (!appError || !cls) {
    const message = typeof appError?.message === "string" ? appError.message : fallback;
    return { show: true, tone: "error", message };
  }

  // Cancelling is a choice, not a fault.
  if (cls === "silent") return { show: false, tone: "warning", message: "" };

  const code = appError.code as string;
  const message =
    REWRITTEN[code] ??
    (typeof appError.message === "string" && appError.message.length > 0
      ? appError.message
      : fallback);

  return { show: true, tone: TONE[cls], message, guidance: GUIDANCE[code] };
}
