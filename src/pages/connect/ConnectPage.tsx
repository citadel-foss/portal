import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { ChevronDown } from "lucide-react";
import {
  checkBackend,
  checkTor,
  getChainBackend,
  getElectrumPresets,
  restartTorBootstrap,
  setChainBackend,
} from "../../api/commands";
import type {
  ChainBackendConfig,
  ChainBackendKind,
  ElectrumPreset,
  NodeBackend,
} from "../../api/types";
import {
  SettingsSection,
  TestResultRows,
  type TestRow,
} from "../../components/ui/display";
import {
  Button,
  CheckRow,
  PasswordField,
  SegmentedToggle,
  SummaryGroup,
  SummaryRow,
} from "../../components/ui/inputs";
import { IntroStage } from "../../components/ui/IntroStage";
import { wait } from "../../lib/timing";
import { useSessionStore } from "../../store/session";
import { formatDuration, formatNumber } from "../../lib/wallet-format";

const TOR_POLL_MS = 1_200;
// Watches for progress stopping rather than capping total time: an absolute ceiling fails a slow
// bootstrap that would have finished seconds later.
//
// Ten minutes, not four. A first bootstrap on a fresh install downloads the whole consensus and
// relay directory, and one unroutable guard costs a ninety-second retry before Tor moves to the
// next — a real run here sat at 10% for 90s and then finished in 70. Anything that calls that a
// stall teaches people to hit Restart on a Tor that was about to succeed, which throws the work
// away and starts the same download again.
const TOR_STALL_MS = 600_000;
// Consecutive probe misses tolerated before the panel reports trouble. A miss against a busy,
// still-bootstrapping Tor is routine, so this is about a run of them, not a single one.
const TOR_MAX_CONSECUTIVE_FAILURES = 6;

/** Signet coins are worthless and mainnet coins are not, so the network leads each row. */
const PRESET_TONE: Record<string, string> = {
  bitcoin: "border-success/40 bg-success/[0.08] text-success",
  signet: "border-warning/40 bg-warning/[0.08] text-warning",
};

/**
 * Picks one of the servers Rust ships, or leaves the URL alone.
 *
 * A menu rather than a native `<select>`: the monospace shell cannot style one, and WKWebView
 * renders it as a platform control that looks nothing like the rest of the gate. The URL field
 * below stays authoritative — this only writes into it, so a hand-typed server is never a
 * second-class citizen.
 */
function ServerPicker({
  presets,
  value,
  onPick,
}: {
  presets: ElectrumPreset[];
  value: string;
  onPick: (url: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const selected = presets.find((p) => p.url === value.trim());

  if (presets.length === 0) return null;
  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center justify-between gap-3 text-left outline-none focus-visible:shadow-ring"
      >
        <span className="flex min-w-0 items-center gap-2.5">
          <span className="font-mono text-[13px] text-foreground">
            {selected?.label ?? "Custom server"}
          </span>
          {selected && (
            <span
              className={`rounded-pill border px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-[0.12em] ${
                PRESET_TONE[selected.network] ?? "border-line text-subtle"
              }`}
            >
              {selected.network}
            </span>
          )}
        </span>
        <ChevronDown
          size={15}
          strokeWidth={2}
          className={`flex-none text-subtle transition-transform ${open ? "rotate-180" : ""}`}
        />
      </button>
      {open && (
        <div className="absolute left-0 right-0 top-full z-20 mt-1 flex flex-col overflow-hidden rounded-control border border-line-strong bg-surface-raised shadow-[0_16px_32px_-16px_rgba(0,0,0,0.8)]">
          {presets.map((preset) => (
            <button
              key={preset.url}
              type="button"
              onClick={() => {
                onPick(preset.url);
                setOpen(false);
              }}
              className={`flex flex-col gap-0.5 px-3.5 py-2.5 text-left outline-none hover:bg-[var(--color-hover)] focus-visible:bg-[var(--color-hover)] ${
                preset.url === selected?.url ? "bg-[var(--color-hover)]" : ""
              }`}
            >
              <span className="flex items-center gap-2.5">
                <span className="font-mono text-[12.5px] text-foreground">{preset.label}</span>
                <span
                  className={`rounded-pill border px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-[0.12em] ${
                    PRESET_TONE[preset.network] ?? "border-line text-subtle"
                  }`}
                >
                  {preset.network}
                </span>
              </span>
              <span className="font-mono text-[10.5px] text-subtle">{preset.url}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * The first screen of every launch, ahead of the role picker, because a wallet and a router
 * both need the same two things before they can do anything: a chain backend that answers,
 * and a bootstrapped Tor.
 *
 * Nothing entered here is written to disk. The fields arrive prefilled from Rust and an edit
 * lasts for the session, which is why a node's RPC password never ends up at rest.
 */
export function ConnectPage() {
  const navigate = useNavigate();
  const setConnected = useSessionStore((s) => s.setConnected);

  const [kind, setKind] = useState<ChainBackendKind>("electrum");
  const [electrumUrl, setElectrumUrl] = useState("");
  const [electrumUseTor, setElectrumUseTor] = useState(false);
  const [presets, setPresets] = useState<ElectrumPreset[]>([]);
  const [node, setNode] = useState<NodeBackend | null>(null);

  const [torProgress, setTorProgress] = useState<number | null>(null);
  // Tor's own account of itself: the phase it is in, and why it says it is struggling. Kept
  // apart from `torError`, which is Portal failing to reach Tor rather than Tor failing.
  const [torPhase, setTorPhase] = useState<string | null>(null);
  const [torError, setTorError] = useState<string | null>(null);
  // Set when the percentage stops climbing. Not a terminal state — the poll keeps running
  // underneath it, so a bootstrap that recovers on its own clears this without being asked to.
  const [torStalled, setTorStalled] = useState(false);
  const [torElapsed, setTorElapsed] = useState(0);
  const [torChecks, setTorChecks] = useState(0);
  const [torRestarting, setTorRestarting] = useState(false);
  const [torRestarts, setTorRestarts] = useState(0);
  // Null means nothing has been checked for the config on screen — which is what puts the
  // Test button back. Starts pending because the arrival probe below is already on its way,
  // and a button that appears for one frame and then vanishes reads as a glitch.
  const [backendRow, setBackendRow] = useState<TestRow | null>({
    label: "Electrum",
    state: "pending",
    message: "Checking…",
  });
  const [advancing, setAdvancing] = useState(false);
  // The config a probe last passed against, as a snapshot: comparing it to the current one
  // tells Next whether a fresh probe would learn anything, without tracking which field changed.
  const [verified, setVerified] = useState<string | null>(null);

  // Bumped by Retry to re-arm the poll after it gave up.
  const [torAttempt, setTorAttempt] = useState(0);

  useEffect(() => {
    void getChainBackend()
      .then((config) => {
        setKind(config.kind);
        setElectrumUrl(config.electrum.url);
        setElectrumUseTor(config.electrum.useTor);
        // The view deliberately omits the password, so it has to be reinstated before this
        // object can be sent back as a config. Empty means "keep the session's own", which
        // is what `merge_preserved_password` fills in on the Rust side.
        const node = config.node && { ...config.node, password: "" };
        setNode(node);
        // Probed on arrival because Tor takes a minute or more to bootstrap and this takes a
        // second: by the time Next unlocks the answer is already in, so pressing it doesn't
        // re-run a check the user has been looking at the result of.
        return probe({
          kind: config.kind,
          electrum: { url: config.electrum.url.trim(), useTor: config.electrum.useTor },
          node,
        });
      })
      // Without a config there is nothing to probe, so hand the user the button instead of
      // leaving the row pending forever.
      .catch(() => setBackendRow(null));
  }, []);

  // No re-entrancy latch: a `useRef` guard survives StrictMode's double-invoke while the run
  // it guarded does not, so the second run finds it set and never polls. The closure flag is
  // enough, since the effect only re-runs when `torAttempt` changes.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      // A probe that misses is not a Tor that failed: checkTor re-runs the full readiness wait
      // on every call, so a single miss against a busy, still-bootstrapping Tor is routine.
      let consecutiveFailures = 0;
      let best = -1;
      const startedAt = Date.now();
      let lastProgressAt = startedAt;
      for (;;) {
        if (cancelled) return;
        setTorElapsed(Math.floor((Date.now() - startedAt) / 1000));
        try {
          const status = await checkTor();
          if (cancelled) return;
          setTorChecks((n) => n + 1);
          if (!(status.reachable && status.authenticated)) {
            throw new Error(status.error ?? "Tor control port unreachable.");
          }
          consecutiveFailures = 0;
          setTorError(null);
          const progress = status.bootstrapProgress ?? 0;
          setTorProgress(progress);
          setTorPhase(status.bootstrapSummary ?? null);
          if (progress === 100) {
            setTorStalled(false);
            return;
          }
          if (progress > best) {
            best = progress;
            lastProgressAt = Date.now();
            setTorStalled(false);
          } else {
            setTorStalled(Date.now() - lastProgressAt > TOR_STALL_MS);
          }
        } catch (e) {
          if (cancelled) return;
          consecutiveFailures += 1;
          if (consecutiveFailures >= TOR_MAX_CONSECUTIVE_FAILURES) {
            setTorError((e as { message?: string })?.message ?? "Tor could not be started.");
            // Deliberately keeps looping. Nothing here can restart Tor — `tor_main` only runs
            // once per process — so the useful thing left is to keep watching the one that is
            // already up, and let it clear its own error if it comes back.
            consecutiveFailures = 0;
          }
        }
        await wait(TOR_POLL_MS);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [torAttempt]);

  /** Makes Tor start its bootstrap over, then re-arms the poll against it. */
  async function restartBootstrap() {
    setTorRestarting(true);
    try {
      await restartTorBootstrap();
      setTorRestarts((n) => n + 1);
      setTorError(null);
      setTorPhase(null);
      setTorProgress(null);
    } catch (e) {
      setTorError(
        (e as { message?: string })?.message ?? "Could not restart Tor's bootstrap.",
      );
    } finally {
      setTorRestarting(false);
      // Re-armed whatever happened, so the stall clock starts over with it: even a refused
      // restart leaves a Tor running that is still worth watching.
      setTorStalled(false);
      setTorAttempt((n) => n + 1);
    }
  }

  const torReady = torProgress === 100;

  // Rust forces Tor for an onion host whatever the flag says, so the toggle follows rather than
  // contradicts it — a control that disagrees with the transport actually used is worse than none.
  const electrumHost = electrumUrl.trim().split("://").pop() ?? "";
  const onionElectrum = electrumHost.split(":")[0].endsWith(".onion");
  const torForced = onionElectrum;

  function currentConfig(): ChainBackendConfig {
    return {
      kind,
      electrum: { url: electrumUrl.trim(), useTor: torForced || electrumUseTor },
      node,
    };
  }

  /** Resolves only when `config` answered a real chain query. */
  async function probe(config: ChainBackendConfig): Promise<boolean> {
    // From the config under test, not from render state: the arrival probe runs before the
    // prefill has been committed to it.
    const label = config.kind === "coreRpc" ? "Bitcoin Core" : "Electrum";
    setBackendRow({ label, state: "pending", message: "Checking…" });
    try {
      const status = await checkBackend(config);
      setBackendRow({
        label,
        state: status.reachable ? "ok" : "failed",
        message: status.reachable
          ? `${status.chain ?? "connected"}${status.blocks !== undefined ? ` · block ${formatNumber(status.blocks)}` : ""}`
          : (status.error ?? "No answer."),
      });
      setVerified(status.reachable ? JSON.stringify(config) : null);
      return status.reachable;
    } catch (e) {
      setBackendRow({
        label,
        state: "failed",
        // Falls through to the raw value: a rejection with no `message` is a plumbing
        // fault, and reporting it as an unreachable server sends the user hunting the
        // wrong thing.
        message: (e as { message?: string })?.message ?? String(e),
      });
      setVerified(null);
      return false;
    }
  }

  async function next() {
    // One snapshot throughout: the config adopted below is the exact one that answered, even
    // if the user edits a field while a probe is in flight.
    const config = currentConfig();
    setAdvancing(true);
    try {
      // A pass already stands for this exact config — the green row on screen is that answer,
      // so probing again would only make Next slower than the check it repeats.
      if (JSON.stringify(config) !== verified && !(await probe(config))) return;
      // Adopting the config is what marks the gate satisfied, so a failure here has to be
      // shown: navigating anyway would bounce straight back and read as the page reloading
      // itself for no reason.
      await setChainBackend(config);
      setConnected();
      navigate("/launch", { replace: true });
    } catch (e) {
      setBackendRow({
        label: config.kind === "coreRpc" ? "Bitcoin Core" : "Electrum",
        state: "failed",
        message: (e as { message?: string })?.message ?? String(e),
      });
    } finally {
      setAdvancing(false);
    }
  }

  // An edit retires the standing pass and the result reporting it, which is what brings the
  // Test button back: nothing on screen describes the config the user now has.
  useEffect(() => {
    void getElectrumPresets()
      .then(setPresets)
      .catch(() => setPresets([]));
  }, []);

  function invalidate() {
    setBackendRow(null);
    setVerified(null);
  }

  function editNode(patch: Partial<NodeBackend>) {
    setNode((n) => (n ? { ...n, ...patch } : n));
    invalidate();
  }

  return (
    <IntroStage
      lead="Let's get you"
      accent="connected."
      caption="Pick where Portal reads the chain from. Tor starts on its own and carries every swap."
      className="min-h-screen"
    >
      <div className="mx-auto w-full max-w-4xl text-left">
        <div className="grid gap-4 md:grid-cols-2">
          <SettingsSection
            title="Choose your backend"
            subtitle="An Electrum server, or a Bitcoin Core node you run yourself"
            bodyClassName="flex flex-col gap-4 p-5"
          >
            <div className="flex flex-col gap-1.5">
              <label className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
                Backend
              </label>
              <SegmentedToggle
                groupId="chain-backend"
                value={kind}
                onChange={(next) => {
                  setKind(next);
                  invalidate();
                }}
                options={[
                  { value: "electrum", label: "Electrum" },
                  { value: "coreRpc", label: "Bitcoin Core" },
                ]}
              />
            </div>

            {kind === "electrum" ? (
              <>
                <SummaryGroup title="Server">
                  <ServerPicker
                    presets={presets}
                    value={electrumUrl}
                    onPick={(url) => {
                      setElectrumUrl(url);
                      invalidate();
                    }}
                  />
                  <SummaryRow
                    label="Server URL"
                    value={electrumUrl}
                    inputMode="text"
                    hint="The default works out of the box"
                    onCommit={(url) => {
                      setElectrumUrl(url);
                      invalidate();
                    }}
                  />
                </SummaryGroup>
                <CheckRow
                  checked={torForced || electrumUseTor}
                  onToggle={(next) => {
                    if (torForced) return;
                    setElectrumUseTor(next);
                    invalidate();
                  }}
                  primary="Reach this server over Tor"
                  secondary={
                    torForced
                      ? "Always on for an onion address"
                      : "Hides which server you ask, and costs some speed"
                  }
                />
                <p className="text-[11.5px] leading-5 text-subtle">
                  Off by default so the chain stays readable even when Tor is slow to bootstrap.
                  Swap traffic always goes over Tor either way.
                </p>
              </>
            ) : (
              node && (
                <>
                  <SummaryGroup title="Node connection">
                    <SummaryRow
                      label="RPC Host"
                      value={node.host}
                      inputMode="text"
                      onCommit={(host) => editNode({ host })}
                    />
                    <SummaryRow
                      label="RPC Port"
                      value={String(node.port)}
                      onCommit={(port) => editNode({ port: Number(port) || node.port })}
                    />
                    <SummaryRow
                      label="RPC Username"
                      value={node.username}
                      inputMode="text"
                      onCommit={(username) => editNode({ username })}
                    />
                    <SummaryRow
                      label="ZMQ Port"
                      value={String(node.zmqPort)}
                      onCommit={(zmqPort) => editNode({ zmqPort: Number(zmqPort) || node.zmqPort })}
                    />
                  </SummaryGroup>
                  <PasswordField
                    label="RPC Password"
                    placeholder={
                      node.passwordConfigured
                        ? "Portal's default (enter to replace)"
                        : "Enter RPC password"
                    }
                    autoComplete="current-password"
                    value={node.password}
                    onChange={(e) => editNode({ password: e.target.value })}
                  />
                </>
              )
            )}

            {/* A result and the button that produces it are the same control in two states,
                never both: the check runs on its own until an edit leaves nothing to report. */}
            {backendRow && <TestResultRows rows={[backendRow]} />}
            {(backendRow === null || backendRow.state === "failed") && (
              <div>
                <Button size="sm" variant="secondary" onClick={() => void probe(currentConfig())}>
                  {backendRow ? "Try again" : "Test connection"}
                </Button>
              </div>
            )}
          </SettingsSection>

          <SettingsSection
            title="Tor connection"
            subtitle="Portal's own Tor, started fresh for this session"
            bodyClassName="flex flex-col gap-4 p-5"
          >
            <p className="text-[12px] leading-5 text-muted">
              Tor routes all swap traffic, so your IP and coin history stay private. Portal
              never touches a Tor already running on this machine. Next unlocks once it has
              fully bootstrapped.
            </p>
            <TestResultRows
              rows={[
                {
                  label: "Tor",
                  state: torError ? "failed" : torReady ? "ok" : "pending",
                  message: torError
                    ? torError
                    : torReady
                      ? "Bootstrap complete — Tor is ready"
                      : torProgress === null
                        ? "Starting…"
                        : // Tor's own phase text plus the clock, so a long wait says which
                          // step it is on and how long it has been going, rather than only
                          // that the number has not moved.
                          `${torProgress}%${torPhase ? ` · ${torPhase}` : ""}${
                            torElapsed >= 20 ? ` · ${formatDuration(torElapsed)}` : ""
                          }`,
                },
              ]}
            />
            {!torReady && (torStalled || torError) && (
              <div className="flex flex-col gap-1.5 rounded-control border border-warning/30 bg-warning/[0.06] p-4">
                {torStalled && (
                  <p className="text-[12px] leading-5 text-foreground">
                    No progress past {torProgress}% for {Math.round(TOR_STALL_MS / 60_000)}{" "}
                    minutes. Usually a slow or filtered network. Portal is still watching —
                    this clears itself if Tor gets through.
                  </p>
                )}
                <p className="text-[11.5px] text-subtle">
                  {torChecks} {torChecks === 1 ? "check" : "checks"}
                  {torRestarts > 0 &&
                    ` · ${torRestarts} ${torRestarts === 1 ? "restart" : "restarts"}`}
                </p>
              </div>
            )}
            {!torReady && (torStalled || torError) && (
              <div>
                <Button
                  size="sm"
                  variant="secondary"
                  loading={torRestarting}
                  onClick={() => void restartBootstrap()}
                >
                  Restart bootstrap
                </Button>
              </div>
            )}
          </SettingsSection>
        </div>

        <div className="mt-4 flex items-center justify-end gap-3">
          {!torReady && !torError && (
            <span className="text-[12px] text-muted">Waiting for Tor to finish…</span>
          )}
          <Button disabled={!torReady} loading={advancing} onClick={() => void next()}>
            Next
          </Button>
        </div>
      </div>
    </IntroStage>
  );
}
