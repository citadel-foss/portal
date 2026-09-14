/**
 * The single boundary between the React app and whichever host is running it.
 *
 * `@host` resolves at build time to `desktop.ts` (Tauri IPC) or `web.ts` (same-origin HTTP and
 * SSE) — see `vite.config.ts`. Nothing outside this module and `platform/` may import a host
 * directly, which is what keeps `@tauri-apps/*` out of the web bundle.
 */
import { host } from "@host";

/** Raised for a failure the host could not express as an `AppError` envelope. */
export interface HostFailure {
  code: string;
  message: string;
  details?: unknown;
}

export interface Transport {
  /** Invoke one operation by its contract name. Callers go through `commands.ts`, which
   *  pairs each name with its own request and result types. */
  invoke<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  /** Subscribe to a domain event. Resolves to an unsubscribe function. */
  subscribe<T>(event: string, handler: (payload: T) => void): Promise<() => void>;
}

export const invoke: Transport["invoke"] = (name, args) => host.invoke(name, args);
export const subscribe: Transport["subscribe"] = (event, handler) =>
  host.subscribe(event, handler);
