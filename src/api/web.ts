/** Web host: same-origin HTTP command routes and one shared SSE stream per tab. */
import type {
  Host,
  RestoreSelection,
  UnresolvedOperation,
} from "./host-contract";

const API = `${import.meta.env.BASE_URL.replace(/\/$/, "")}/api/v1`;

/** Required on every mutation, and issued only by a login. `GET /session` never returns it: on
 *  Umbrel every app shares one host, so the session cookie reaches them all, and a token
 *  readable with the cookie alone would let any of them act as the owner. Kept in localStorage,
 *  which — unlike a cookie — is separated by port, so it survives reloads and new tabs. */
const CSRF_KEY = "portal.csrf";

function storedCsrfToken(): string | null {
  try {
    return localStorage.getItem(CSRF_KEY);
  } catch {
    return null;
  }
}

let csrfToken: string | null = storedCsrfToken();

export function setCsrfToken(token: string | null) {
  csrfToken = token;
  try {
    if (token) localStorage.setItem(CSRF_KEY, token);
    else localStorage.removeItem(CSRF_KEY);
  } catch {
    // Storage refused (private mode): the token still works for this page's lifetime.
  }
}

/**
 * Produces the UUID expected by the durable-operation journal. `randomUUID()` is restricted
 * to secure browser contexts, while `getRandomValues()` remains available when Portal is
 * intentionally served over plain HTTP on a trusted network.
 */
function operationId(): string {
  if (typeof crypto.randomUUID === "function") return crypto.randomUUID();

  const bytes = crypto.getRandomValues(new Uint8Array(16));
  // RFC 9562 UUIDv4 version and variant bits.
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/** Normalizes a failed response into the same raw shape `isAppError` accepts, so callers
 *  switch on `code` exactly as they do on desktop rather than unwrapping a JS `Error`. */
async function toAppError(response: Response): Promise<unknown> {
  // A command refused for want of a session means it expired or the server restarted. The
  // page holds nothing worth keeping past that, so it starts over from the login screen.
  // Not for the login routes themselves, where a 401 is a wrong password.
  if (response.status === 401 && !/\/(session|auth\/\w+)$/.test(new URL(response.url).pathname)) {
    window.location.hash = "#/login";
    window.location.reload();
  }
  const body = await response.json().catch(() => null);
  if (body && typeof body === "object" && "error" in body) return (body as { error: unknown }).error;
  return {
    code: response.status === 401 ? "NOT_AUTHENTICATED" : "INTERNAL",
    message: `${response.status} ${response.statusText}`,
  };
}

async function call<T>(
  name: string,
  args?: Record<string, unknown>,
): Promise<T> {
  // One key per deliberate invocation — two deliberate sends are two operations, not a
  // retry of one. The key is kept here so that if the response is lost in flight we can ask
  // the server what became of it rather than resubmitting blind.
  const key = operationId();
  let response: Response;
  try {
    response = await fetch(`${API}/commands/${name}`, {
      method: "POST",
      credentials: "same-origin",
      headers: {
        "Content-Type": "application/json",
        ...(csrfToken ? { "X-CSRF-Token": csrfToken } : {}),
        "Idempotency-Key": key,
      },
      body: JSON.stringify(args ?? {}),
    });
  } catch (networkFailure) {
    // The request may well have been accepted before the connection dropped. Resubmitting
    // now is how a payment gets made twice, so ask first.
    return (await reconcileLostResponse(key, networkFailure)) as T;
  }
  if (!response.ok) throw await toAppError(response);

  // A unit result is an explicit JSON `null`, so an empty body means an empty body.
  const text = await response.text();
  const body = text.length === 0 ? null : JSON.parse(text);

  // Durable work is accepted, not completed, so that closing the tab cannot cancel it. The
  // caller still wants the value its desktop counterpart returns, so wait for the record to
  // settle and hand back what it holds.
  if (response.status === 202) {
    return (await settle((body as { operationId: string }).operationId)) as T;
  }
  return body as T;
}

/**
 * A request whose response never arrived. If the server durably accepted it, that record is
 * the answer and the work is already running; a 404 means acceptance never happened and the
 * caller is free to try again.
 */
async function reconcileLostResponse(
  key: string,
  cause: unknown,
): Promise<unknown> {
  try {
    const probe = await fetch(`${API}/operations/${key}`, {
      credentials: "same-origin",
    });
    if (probe.ok) return await settle(key);
  } catch {
    // Still unreachable — fall through and report the original failure.
  }
  throw {
    code: "SERVER_UNREACHABLE",
    message:
      (cause as { message?: string })?.message ??
      "The request did not reach Portal's API.",
  };
}

/** Longest an accepted operation is followed before the caller is told to look at it
 *  separately. Generous on purpose: initialization runs any outstanding recovery inline,
 *  which blocks on a block being mined. */
const SETTLE_TIMEOUT_MS = 30 * 60 * 1000;

async function settle(operationId: string): Promise<unknown> {
  const deadline = Date.now() + SETTLE_TIMEOUT_MS;
  let wait = 300;
  for (;;) {
    // A status check that fails says nothing about the operation, which runs on regardless.
    // Reporting it as a failure is what invites a second press of Send, so it is retried. Only
    // a real answer ends the wait: a 404 (the server restarted and forgot it, so its outcome is
    // unknown) or a refused session.
    const response = await fetch(`${API}/operations/${operationId}`, {
      credentials: "same-origin",
    }).catch(() => null);
    if (response?.status === 404) {
      throw {
        code: "OPERATION_UNRESOLVED",
        message: "Portal lost track of this operation. Check its outcome before retrying.",
      };
    }
    if (response && (response.status === 401 || response.status === 403)) {
      throw await toAppError(response);
    }
    if (!response?.ok) {
      if (Date.now() > deadline) {
        throw {
          code: "OPERATION_PENDING",
          message: "Still running. It keeps going on the server — reopen Portal to see it.",
        };
      }
      await new Promise((resolve) => setTimeout(resolve, wait));
      wait = Math.min(wait * 1.5, 2000);
      continue;
    }
    const record = (await response.json()) as {
      state: string;
      result?: unknown;
      error?: { code: string; message: string };
    };
    if (record.state === "succeeded") return record.result ?? null;
    if (record.state === "failed") {
      throw (
        record.error ?? { code: "INTERNAL", message: "the operation failed" }
      );
    }
    // Never reported as a plain failure: the effect may have happened, and saying otherwise
    // is what invites someone to send the same payment twice.
    if (record.state === "indeterminate") {
      throw {
        code: "OPERATION_UNRESOLVED",
        message:
          record.error?.message ??
          "Portal could not confirm whether this completed. Check its outcome before retrying.",
      };
    }
    if (Date.now() > deadline) {
      throw {
        code: "OPERATION_PENDING",
        message:
          "Still running. It keeps going on the server — reopen Portal to see it.",
      };
    }
    await new Promise((resolve) => setTimeout(resolve, wait));
    wait = Math.min(wait * 1.5, 2000);
  }
}

/** One EventSource per tab, shared by every subscriber — not one per component. */
let stream: EventSource | null = null;
const handlers = new Map<string, Set<(payload: unknown) => void>>();

function ensureStream() {
  if (stream) return;
  stream = new EventSource(`${API}/events`, { withCredentials: true });
  stream.onmessage = (message) => {
    const { name, payload } = JSON.parse(message.data) as {
      name: string;
      payload: unknown;
    };
    handlers.get(name)?.forEach((handler) => handler(payload));
  };
  // Named events do not reach `onmessage`. Without this listener the server's warning that
  // we fell behind is silently dropped and the tab stays stale forever, which is the exact
  // failure the signal exists to prevent.
  stream.addEventListener("resync-required", () => {
    // Nothing here knows which entities changed, so every subscriber re-reads. The domain
    // events are invalidations anyway; a reconnect is the honest way to catch up.
    window.dispatchEvent(new CustomEvent("portal:resync"));
  });
}

/** Posts to an auth endpoint, which sits outside the command table. */
async function post(path: string, body: unknown): Promise<Response> {
  return fetch(`${API}${path}`, {
    method: "POST",
    credentials: "same-origin",
    headers: {
      "Content-Type": "application/json",
      ...(csrfToken ? { "X-CSRF-Token": csrfToken } : {}),
    },
    body: JSON.stringify(body),
  });
}

async function runRestore(): Promise<{
  authenticated: boolean;
  hasOwner: boolean;
}> {
  // A failure to reach the server at all is not a failure to authenticate. Letting the
  // two collapse into one puts a password prompt in front of someone whose API simply
  // is not running, which is impossible to diagnose from the screen.
  let response: Response;
  try {
    // The server checks it against the session, so a token left over from another session
    // reads as signed out rather than as a session that can change nothing.
    response = await fetch(`${API}/session`, {
      credentials: "same-origin",
      headers: csrfToken ? { "X-CSRF-Token": csrfToken } : {},
    });
  } catch {
    throw {
      code: "SERVER_UNREACHABLE",
      message: "Portal's API is not responding.",
    };
  }
  if (response.status === 401) {
    // The 401 body carries it: probing with a real login attempt would burn the
    // server's throttle, and a handful of page loads would lock the owner out.
    const body = await response.json().catch(() => null);
    // `details`, not the error itself: that is where `AppError` carries structured data,
    // and reading the wrong level silently yielded `undefined` — which the fallback below
    // then turned into "there is an owner", so an unclaimed server offered a sign-in form
    // for a password that did not exist yet.
    const hasOwner =
      (body as { error?: { details?: { hasOwner?: boolean } } } | null)?.error
        ?.details?.hasOwner ?? true;
    setCsrfToken(null);
    return { authenticated: false, hasOwner };
  }
  // Only a 401 means "log in". A dev proxy with nothing behind it answers 500 rather
  // than failing the connection, so every other status is the server being absent —
  // showing a password prompt for that sends the user somewhere with no way out.
  if (!response.ok) {
    throw {
      code: "SERVER_UNREACHABLE",
      message: "Portal's API is not responding.",
    };
  }
  const body = (await response.json()) as { hasOwner: boolean };
  return { authenticated: true, hasOwner: body.hasOwner };
}

export const host: Host = {
  invoke: call,
  subscribe: async (event, handler) => {
    ensureStream();
    const typed = handler as (payload: unknown) => void;
    const set = handlers.get(event) ?? new Set();
    set.add(typed);
    handlers.set(event, set);
    return () => {
      set.delete(typed);
    };
  },
  capabilities: {
    nativeFilePicker: false,
    // The server has no user's Router Dashboard to read; the command is desktop-only.
    localDashboardImport: false,
    canQuit: false,
    selfUpdate: false,
  },
  operations: {
    blocking: async () => {
      const response = await fetch(`${API}/operations`, {
        credentials: "same-origin",
      });
      if (!response.ok) return [];
      const body = (await response.json()) as {
        blockingConflicts?: UnresolvedOperation[];
      };
      return body.blockingConflicts ?? [];
    },
    reconcile: async (id) => {
      await post(`/operations/${id}/reconcile`, {});
    },
    acknowledge: async (id) => {
      const response = await post(`/operations/${id}/acknowledge`, {});
      if (!response.ok) throw await toAppError(response);
    },
  },
  session: {
    restore: runRestore,
    login: async (password) => {
      const response = await post("/auth/login", { password });
      if (!response.ok) throw await toAppError(response);
      const body = (await response.json()) as { csrfToken: string };
      setCsrfToken(body.csrfToken);
    },
    claim: async (password) => {
      const claimed = await post("/auth/claim", { password });
      if (!claimed.ok) throw await toAppError(claimed);
      await host.session.login(password);
    },
  },
  openExternal: async (url) => {
    // Validated and opened with no opener handle, so a link cannot reach back into the app.
    const parsed = new URL(url);
    if (parsed.protocol !== "https:")
      throw new Error("only https links can be opened");
    window.open(parsed.href, "_blank", "noopener,noreferrer");
  },
  // There is no server filesystem to browse from a browser; the web flows upload instead.
  pickDirectory: async () => null,
  pickFile: async () => null,
  selectBackup: async () => {
    const file = await chooseLocalFile();
    if (!file) throw { code: "USER_CANCELLED", message: "no backup chosen" };
    const response = await fetch(`${API}/uploads`, {
      method: "POST",
      credentials: "same-origin",
      headers: {
        "Content-Type": "application/json",
        ...(csrfToken ? { "X-CSRF-Token": csrfToken } : {}),
      },
      body: await file.text(),
    });
    if (!response.ok) throw await toAppError(response);
    return (await response.json()) as RestoreSelection;
  },
  // Two steps because the server only hands a backup over once: producing it returns an id,
  // and the download consumes and deletes it, so a retry means producing a fresh one.
  createBackup: async (password, routerId, walletPassword) => {
    const created = await post("/backups", { password, routerId, walletPassword });
    if (!created.ok) throw await toAppError(created);
    const { artifactId } = (await created.json()) as { artifactId: string };
    const download = await post(`/backups/${artifactId}/download`, {});
    if (!download.ok) throw await toAppError(download);
    const stem = routerId === undefined ? "portal-wallet-backup" : `portal-router-${routerId}-backup`;
    const name = `${stem}-${Math.floor(Date.now() / 1000)}.json`;
    const url = URL.createObjectURL(await download.blob());
    const link = document.createElement("a");
    link.href = url;
    link.download = name;
    link.click();
    // Revoked on the next turn, not immediately: some browsers read the URL after `click`
    // returns, and a revoked one downloads nothing.
    setTimeout(() => URL.revokeObjectURL(url), 0);
    return name;
  },
};

/** A hidden input rather than a native dialog: the browser owns file choice, and the page
 *  only ever sees the bytes the user picked. */
function chooseLocalFile(): Promise<File | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "application/json,.json";
    input.onchange = () => resolve(input.files?.[0] ?? null);
    // Cancelling fires no `change` in most browsers; `cancel` is the one that does.
    input.oncancel = () => resolve(null);
    input.click();
  });
}
