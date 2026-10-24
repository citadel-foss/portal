import { useCallback, useEffect, useState } from "react";
import { HashRouter, Navigate, Outlet, Route, Routes, useLocation, useParams } from "react-router-dom";
import { AppShell } from "./components/app/AppShell";
import { QuitShutdown } from "./components/app/QuitShutdown";
import { RecoveryPhraseGate } from "./components/app/RecoveryPhrase";
import { ConnectPage } from "./pages/connect/ConnectPage";
import { LoginPage } from "./pages/auth/LoginPage";
import { LaunchPage } from "./pages/launch/LaunchPage";
import { LogsPage } from "./pages/logs/LogsPage";
import { RouterPage } from "./pages/router/RouterPage";
import { AddRouterPage } from "./pages/router/AddRouterPage";
import { RouterWorkspacePage } from "./pages/router/RouterWorkspacePage";
import { RouterSetupPage } from "./pages/router/RouterSetupPage";
import { RouterSwapReportPage } from "./pages/router/RouterSwapReportPage";
import { MarketPage } from "./pages/market/MarketPage";
import { SendPage } from "./pages/send/SendPage";
import { SetupPage } from "./pages/setup/SetupPage";
import { RecoveriesPage } from "./pages/swap/RecoveriesPage";
import { RecoveryPage } from "./pages/swap/RecoveryPage";
import { SwapPage } from "./pages/swap/SwapPage";
import { SwapReportPage } from "./pages/swap/SwapReportPage";
import { SwapReportsPage } from "./pages/swap/SwapReportsPage";
import { WalletPage } from "./pages/wallet/WalletPage";
import {
  confirmRecoveryPhraseSaved,
  confirmRouterRecoveryPhraseSaved,
  getSessionState,
  recoveryPhraseSaved,
  revealRecoveryPhrase,
  revealRouterRecoveryPhrase,
  routerRecoveryPhraseSaved,
} from "./api/commands";
import { refreshWalletCache } from "./lib/wallet-sync";
import { watchUnresolved } from "./store/unresolved";
import { useSessionStore } from "./store/session";
import { session } from "./platform";
import { ServerUnreachable } from "./pages/auth/ServerUnreachable";
import { REFRESH_INTERVAL_MS } from "./store/wallet-cache";

/**
 * Guards the wallet half only. Router routes are deliberately outside it: router commands
 * resolve their wallet and data dir from the router's own registration, so a router session
 * needs no wallet. A wallet existing on disk never satisfies this — only one the user has
 * actually unlocked, which `RestoreRuntime` reads back from Rust.
 */
function RequireWallet() {
  const initialized = useSessionStore((s) => s.initialized);

  // Scoped here rather than in AppShell because this is the only subtree where a wallet is
  // guaranteed to exist — a router-only session would otherwise sync a wallet that isn't
  // there. Runs regardless of the active wallet route so Send/Swap never depend on the
  // Wallet page having been mounted recently.
  useEffect(() => {
    if (!initialized) return;
    // Unresolved payments are watched from here too: Send and Swap both gate on them, and
    // the recheck that clears one must run whether or not either page is open.
    watchUnresolved();
    const id = setInterval(
      () => void refreshWalletCache(),
      REFRESH_INTERVAL_MS,
    );
    return () => clearInterval(id);
  }, [initialized]);

  // The wallet picker, not the role picker: reaching a wallet route already answers which
  // role was wanted, and the picker offers its own way back to the role choice. The role
  // picker stays the entry point the connection gate hands off to.
  if (!initialized) return <Navigate to="/setup" replace />;
  // Asked here, not after `init_taker`, so every way into a wallet passes it: a create, an
  // unlock of one never confirmed, a reload.
  return (
    <RecoveryPhraseGate
      saved={recoveryPhraseSaved}
      reveal={revealRecoveryPhrase}
      confirm={confirmRecoveryPhraseSaved}
      subject="wallet"
    >
      <Outlet />
    </RecoveryPhraseGate>
  );
}

/** A router's phrase comes before anything else on it, its setup included: the router can be
 *  funded the moment it runs, and its words must be saved before there is anything to lose. */
function RequireRouterPhrase() {
  const id = useParams<{ routerId: string }>().routerId!;
  // Asked again on every page of the router: Rust only learns a stopped router's phrase is unsaved
  // when a start opens its wallet, and that start's error sends the user here.
  const { pathname } = useLocation();
  const saved = useCallback(() => routerRecoveryPhraseSaved(id), [id]);
  const reveal = useCallback((password: string) => revealRouterRecoveryPhrase(id, password), [id]);
  const confirm = useCallback((password: string) => confirmRouterRecoveryPhraseSaved(id, password), [id]);
  return (
    <RecoveryPhraseGate key={pathname} saved={saved} reveal={reveal} confirm={confirm} subject="router">
      <Outlet />
    </RecoveryPhraseGate>
  );
}

/**
 * Reload is not relaunch. The page is replaced, Rust is not: the wallet the user unlocked is
 * still open in its memory, along with the chain backend and Tor the gate checked. Asking it
 * once on mount is what keeps a refresh from dumping the user back at the role picker with a
 * password prompt for a wallet that was never closed.
 *
 * An open wallet is itself proof the gate was cleared — nothing can initialize without a
 * backend that answered and a bootstrapped Tor — so one call restores both flags.
 */
function RestoreRuntime() {
  const initialized = useSessionStore((s) => s.initialized);
  const setInitialized = useSessionStore((s) => s.setInitialized);
  const setNotInitialized = useSessionStore((s) => s.setNotInitialized);
  const setConnected = useSessionStore((s) => s.setConnected);
  const [unreachable, setUnreachable] = useState(false);

  useEffect(() => {
    if (initialized !== null || unreachable) return;
    void getSessionState()
      .then((state) => {
        setUnreachable(false);
        // Before the wallet check: a router session has no wallet, and losing the gate on a
        // reload would send it back to /connect from whatever router page it was on.
        if (state.connected) setConnected();
        if (!state.initialized) return setNotInitialized();
        setInitialized({ walletName: state.walletName ?? "", dataDir: state.dataDir ?? "" });
      })
      // The server answers this one even with no wallet open, so a rejection is the transport
      // failing, not an empty session. Saying "no wallet" here would send someone with a live
      // wallet to the role picker to unlock a second one over the top of it.
      .catch(() => setUnreachable(true));
  }, [initialized, unreachable, setInitialized, setNotInitialized, setConnected]);

  if (unreachable) return <ServerUnreachable onRetry={() => setUnreachable(false)} />;
  // Nothing rather than a flash of the role picker: the answer decides which page is correct.
  if (initialized === null) return null;
  return <Outlet />;
}

/**
 * Everything past the connection gate needs a backend that answers and a bootstrapped Tor,
 * so `/connect` is the entry for both roles.
 *
 * Read straight from the store rather than asked of Rust: a round trip would leave this
 * rendering nothing until it resolved, and any hiccup in that one call would bounce the user
 * back to the gate they just completed. A reload replays the gate, which is cheap once Tor
 * is already up.
 */
function RequireConnection() {
  const connected = useSessionStore((s) => s.connected);
  if (!connected) return <Navigate to="/connect" replace />;
  return <Outlet />;
}

/**
 * Ahead of every other gate, including the connection one: on the web nothing behind this is
 * readable unauthenticated, so asking the chain backend anything first would only produce a
 * 401. Desktop resolves immediately and renders straight through.
 */
function RequireSession() {
  const authenticated = useSessionStore((s) => s.authenticated);
  const setAuthenticated = useSessionStore((s) => s.setAuthenticated);
  const setHasOwner = useSessionStore((s) => s.setHasOwner);
  const [unreachable, setUnreachable] = useState(false);

  const askAgain = () => {
    setUnreachable(false);
    setAuthenticated(null as unknown as boolean);
  };

  useEffect(() => {
    if (authenticated !== null || unreachable) return;
    void session
      .restore()
      .then((info) => {
        setHasOwner(info.hasOwner);
        setAuthenticated(info.authenticated);
      })
      .catch((e) => {
        // Only a refusal means log in. Anything else is the server being absent, which a
        // password cannot fix and must not be disguised as.
        if ((e as { code?: string })?.code === "SERVER_UNREACHABLE") setUnreachable(true);
        else setAuthenticated(false);
      });
  }, [authenticated, unreachable, setAuthenticated, setHasOwner]);

  if (unreachable) return <ServerUnreachable onRetry={askAgain} />;
  if (authenticated === null) return null;
  if (!authenticated) return <Navigate to="/login" replace />;
  return <Outlet />;
}


function App() {
  return (
    <HashRouter>
      {/* Outside the routes: a quit can be requested from any page, including the ones
          that render before a wallet exists, and the access warning belongs on all of them. */}
      <QuitShutdown />
      <Routes>
        <Route path="/login" element={<LoginPage />} />

        <Route element={<RequireSession />}>
          <Route element={<RestoreRuntime />}>
            <Route path="/connect" element={<ConnectPage />} />

            <Route element={<RequireConnection />}>
              <Route path="/launch" element={<LaunchPage />} />
              <Route path="/setup" element={<SetupPage />} />

              <Route element={<AppShell />}>
                <Route element={<RequireWallet />}>
                  <Route path="/" element={<WalletPage />} />
                  <Route path="/market" element={<MarketPage />} />
                  <Route path="/send" element={<SendPage />} />
                  <Route path="/swap" element={<SwapPage />} />
                  <Route path="/swap/recovery" element={<RecoveriesPage />} />
                  <Route
                    path="/swap/recovery/:swapId"
                    element={<RecoveryPage />}
                  />
                  <Route path="/swap/reports" element={<SwapReportsPage />} />
                  <Route
                    path="/swap/reports/:swapId"
                    element={<SwapReportPage />}
                  />
                  <Route path="/logs" element={<LogsPage />} />
                </Route>

                <Route path="/router" element={<RouterPage />} />
                <Route path="/router/new" element={<AddRouterPage />} />
                <Route path="/router/:routerId" element={<RequireRouterPhrase />}>
                  <Route index element={<RouterWorkspacePage />} />
                  <Route path="setup" element={<RouterSetupPage />} />
                  <Route path="report/:swapId" element={<RouterSwapReportPage />} />
                </Route>
              </Route>
            </Route>
          </Route>
        </Route>

        <Route path="*" element={<Navigate to="/connect" replace />} />
      </Routes>
    </HashRouter>
  );
}

export default App;
