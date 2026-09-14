/** What every host must provide. Kept apart from `transport.ts` so both hosts can import the
 *  shape without importing each other. */
/** What the host knows about the current viewer. Desktop is always "in": the process is the
 *  user's own. Web has to ask the server. */
export interface SessionInfo {
  authenticated: boolean;
  /** False on a fresh self-hosted install: the first visitor must claim it with the
   *  one-time bootstrap secret before any password exists to log in with. */
  hasOwner: boolean;
}

export interface HostSession {
  /** Called once at startup, before any command runs. */
  restore(): Promise<SessionInfo>;
  login(password: string): Promise<void>;
  /** Claim an unowned installation with its one-time secret, then log in. */
  claim(secret: string, password: string): Promise<void>;
  logout(): Promise<void>;
}

export interface HostCapabilities {
  /** A native picker exists; the web host uploads instead and hides location controls. */
  nativeFilePicker: boolean;
  /** Quitting the process is offered. Web offers logout — the supervisor owns the daemon. */
  canQuit: boolean;
  /** Whether the UI must show a login gate at all. Constant on desktop; on web the server
   *  decides, because a local run with no credential configured has nothing to log in to. */
  requiresLogin: boolean;
}

export interface Host {
  invoke<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  subscribe<T>(event: string, handler: (payload: T) => void): Promise<() => void>;
  capabilities: HostCapabilities;
  session: HostSession;
  openExternal(url: string): Promise<void>;
  pickDirectory(defaultPath?: string): Promise<string | null>;
  pickFile(defaultPath?: string): Promise<string | null>;
  /** Chooses a wallet backup and registers it with the server, returning the opaque ID a
   *  restore consumes. Desktop opens a native picker; web uploads the bytes. Either way the
   *  browser never learns a server path. */
  selectBackup(): Promise<RestoreSelection>;
}

/** Mirrors `RestoreSelectionView` in core. */
export interface RestoreSelection {
  selectionId: string;
  displayName: string;
}
