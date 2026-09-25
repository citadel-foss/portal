import {
  Check,
  Copy,
  Inbox,
  Plus,
  RefreshCw,
  Square,
  Play,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { motion } from "framer-motion";
import {
  getRouterBalances,
  getRouterStatus,
  listRouterSwapReports,
  listRouters,
  startRouter,
  stopRouter,
} from "../../api/commands";
import {
  isAppError,
  type Balances,
  type RouterPhase,
  type RouterSettings,
  type RouterStatus,
} from "../../api/types";
import {
  EmptyState,
  EntityMonogram,
  Modal,
  SatsAmount,
  StatStrip,
  StatusChip,
} from "../../components/ui/display";
import {
  Button,
  LinkButton,
  PasswordField,
  SegmentedToggle,
} from "../../components/ui/inputs";
import { formatTorEndpoint } from "../../lib/market-format";
import { IntroStage } from "../../components/ui/IntroStage";
import { RouterIntro } from "./RouterIntro";
import { useToastStore } from "../../store/toast";
import { DashboardImport } from "./DashboardImport";
import { copyText } from "../../lib/clipboard";
import { formatNumber } from "../../lib/wallet-format";

interface OwnedRouter {
  settings: RouterSettings;
  status: RouterStatus | null;
  balances: Balances | null;
  earningsSats: number | null;
  reportCount: number | null;
}

type RouterFilter = "all" | "running" | "stopped";

// Per app launch, not per mount: crossing into the router side deserves the arrival, returning
// to the fleet from a workspace does not.
let introPlayed = false;

const PHASE_CLASS: Record<RouterPhase["phase"], string> = {
  notConfigured: "bg-subtle",
  initializing:
    "bg-warning shadow-[0_0_10px_color-mix(in_oklab,var(--color-warning)_45%,transparent)]",
  starting:
    "bg-warning shadow-[0_0_10px_color-mix(in_oklab,var(--color-warning)_45%,transparent)]",
  running:
    "bg-success shadow-[0_0_10px_color-mix(in_oklab,var(--color-success)_50%,transparent)]",
  stopping:
    "bg-warning shadow-[0_0_10px_color-mix(in_oklab,var(--color-warning)_45%,transparent)]",
  stopped: "bg-subtle",
  failed:
    "bg-danger shadow-[0_0_10px_color-mix(in_oklab,var(--color-danger)_45%,transparent)]",
};

function phaseLabel(phase: RouterPhase["phase"]): string {
  return phase.replace(/([A-Z])/g, " $1").toLowerCase();
}

function phaseTone(
  phase: RouterPhase["phase"],
): "success" | "warning" | "danger" | "subtle" {
  if (phase === "running") return "success";
  if (phase === "failed") return "danger";
  if (["initializing", "starting", "stopping"].includes(phase))
    return "warning";
  return "subtle";
}

function BalanceValue({ label, sats, tone }: { label: string; sats: number; tone?: string }) {
  return (
    <div className="min-w-0 px-4 py-3.5">
      <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">{label}</span>
      <strong className={`mt-1.5 block truncate font-mono text-[12px] font-semibold ${tone ?? "text-foreground"}`}>
        <SatsAmount sats={sats} />
      </strong>
    </div>
  );
}

function RouterCard({
  router,
  onChanged,
}: {
  router: OwnedRouter;
  onChanged: () => Promise<void>;
}) {
  const pushToast = useToastStore((state) => state.push);
  const [copied, setCopied] = useState(false);
  const [actionLoading, setActionLoading] = useState(false);
  const [unlocking, setUnlocking] = useState(false);
  const [password, setPassword] = useState("");
  const [unlockError, setUnlockError] = useState<string | undefined>();
  const { settings, status, balances } = router;
  const phase = status?.phase.phase ?? "notConfigured";
  const running = phase === "running" || phase === "starting";
  const transitioning = ["initializing", "starting", "stopping"].includes(
    phase,
  );
  const torAddress = status?.torAddress;

  function copyTorAddress() {
    if (!torAddress) return;
    void copyText(torAddress).then((ok) => {
      if (!ok) return;
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    });
  }

  async function toggleRouter() {
    setActionLoading(true);
    try {
      if (running) await stopRouter(settings.routerId);
      else if (status?.walletEncrypted) {
        setPassword("");
        setUnlockError(undefined);
        setUnlocking(true);
        return;
      } else await startRouter(settings.routerId);
      pushToast(
        "success",
        `${settings.routerId} ${running ? "stopped" : "is starting"}.`,
      );
      await onChanged();
    } catch (error) {
      if (!running && isAppError(error) && error.code === "WALLET_WRONG_PASSWORD") {
        setPassword("");
        setUnlockError(undefined);
        setUnlocking(true);
      } else {
        pushToast(
          "error",
          (error as { message?: string })?.message ??
            `Could not ${running ? "stop" : "start"} router.`,
        );
      }
    } finally {
      setActionLoading(false);
    }
  }

  async function submitUnlock() {
    if (!password) return setUnlockError("Enter the router wallet password.");
    setActionLoading(true);
    setUnlockError(undefined);
    try {
      await startRouter(settings.routerId, password);
      setUnlocking(false);
      setPassword("");
      pushToast("success", `${settings.routerId} is starting.`);
      await onChanged();
    } catch (error) {
      setUnlockError(
        isAppError(error) && error.code === "WALLET_WRONG_PASSWORD"
          ? "Wrong password for this router's wallet."
          : ((error as { message?: string })?.message ?? "Could not start router."),
      );
    } finally {
      setActionLoading(false);
    }
  }

  return (
    <article className={`lift flex flex-col rounded-card border border-line-strong bg-surface-raised/55 p-5 hover:border-primary/35 ${balances || running ? "min-h-[350px]" : "min-h-[250px]"}`}>
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-3">
          <EntityMonogram name={settings.routerId} size="sm" />
          <span className={`h-2.5 w-2.5 shrink-0 rounded-full ${PHASE_CLASS[phase]}`} />
          <h3 className="min-w-0 truncate text-[18px] font-bold text-foreground" title={settings.routerId}>{settings.routerId}</h3>
        </div>
        <StatusChip tone={phaseTone(phase)}>{phaseLabel(phase)}</StatusChip>
      </div>

      <div className="mt-5 flex h-[62px] items-center gap-3 rounded-card border border-line bg-surface/70 px-4">
        <span className="shrink-0 font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Tor</span>
        <code
          className="min-w-0 flex-1 truncate text-[11.5px] text-muted"
          title={torAddress}
        >
          {torAddress
            ? formatTorEndpoint(torAddress, 16, 10, true)
            : running
              ? "Waiting for address…"
              : "Available after start"}
        </code>
        <button
          type="button"
          onClick={copyTorAddress}
          disabled={!torAddress}
          aria-label="Copy Tor address"
          className="grid h-8 w-8 shrink-0 place-items-center rounded-control text-muted outline-none hover:bg-[var(--color-hover)] hover:text-foreground focus-visible:shadow-ring active:translate-y-px disabled:opacity-30"
        >
          {copied ? (
            <Check size={15} className="text-success" />
          ) : (
            <Copy size={15} />
          )}
        </button>
      </div>

      {balances ? (
        <div className="mt-4 grid grid-cols-2 overflow-hidden rounded-card border border-line bg-line [&>*:nth-child(odd)]:mr-px [&>*:nth-child(-n+2)]:mb-px [&>*]:bg-surface/80">
          <BalanceValue label="Spendable" sats={balances.spendable} tone="text-primary" />
          <BalanceValue label="Regular" sats={balances.regular} />
          <BalanceValue label="Swap" sats={balances.swap} tone="text-success" />
          <BalanceValue label="Fidelity" sats={balances.fidelity} tone="text-warning" />
        </div>
      ) : running ? (
        <div className="mt-4 grid min-h-[114px] place-items-center rounded-card border border-dashed border-line px-5 text-center">
          <div>
            <strong className="text-[12px] text-foreground">Wallet data is loading</strong>
            <span className="mt-1 block text-[11px] text-subtle">Refresh shortly to see live balances.</span>
          </div>
        </div>
      ) : null}

      <div className="mt-auto flex items-end justify-between gap-3 pt-5">
        <div className="min-w-0">
          <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
            {router.reportCount === null ? (
              "Reports unavailable"
            ) : (
              <>
                {router.reportCount} reports ·{" "}
                <SatsAmount sats={router.earningsSats ?? 0} /> earned
              </>
            )}
          </span>
          {phase === "failed" && (
            <span className="mt-1 block max-w-[240px] truncate text-[10px] text-danger" title={status?.phase.phase === "failed" ? status.phase.message : undefined}>
              {status?.phase.phase === "failed" ? status.phase.message : ""}
            </span>
          )}
        </div>
        <div className="flex shrink-0 gap-2">
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void toggleRouter()}
            loading={actionLoading}
            disabled={transitioning}
          >
            {running ? <Square size={12} /> : <Play size={12} />}
            {running ? "Stop" : "Start"}
          </Button>
          <LinkButton
            to={`/router/${encodeURIComponent(settings.routerId)}`}
            size="sm"
          >
            Manage
          </LinkButton>
        </div>
      </div>

      {/* Portalled: each card sits inside a framer-motion wrapper whose transform would
          otherwise become the containing block for the modal's fixed overlay, trapping it
          inside the card. Leaving the tree also leaves AppShell's accent scope, so the
          wrapper re-declares it. */}
      {unlocking &&
        createPortal(
          <div data-accent="router">
            <Modal
              title={`Unlock ${settings.routerId}`}
              onClose={() => setUnlocking(false)}
              footer={
                <>
                  <Button variant="secondary" onClick={() => setUnlocking(false)}>
                    Cancel
                  </Button>
                  <Button onClick={() => void submitUnlock()} loading={actionLoading}>
                    <Play size={13} /> Start router
                  </Button>
                </>
              }
            >
              <p className="text-[12.5px] text-muted">
                This router's wallet is encrypted. Its password isn't stored between app
                launches, so it's needed again to start the router.
              </p>
              <div className="mt-4">
                <PasswordField
                  label="Router wallet password"
                  autoComplete="current-password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && void submitUnlock()}
                  error={unlockError}
                />
              </div>
            </Modal>
          </div>,
          document.body,
        )}
    </article>
  );
}

export function RouterPage() {
  const [routers, setRouters] = useState<OwnedRouter[]>([]);
  const [filter, setFilter] = useState<RouterFilter>("all");
  const [loading, setLoading] = useState(true);
  const [introDone, setIntroDone] = useState(introPlayed);
  const pushToast = useToastStore((state) => state.push);

  const load = useCallback(async () => {
    const registrations = await listRouters();
    const rows = await Promise.all(
      registrations.map(async (settings): Promise<OwnedRouter> => {
        const [status, balances, reports] = await Promise.allSettled([
          getRouterStatus(settings.routerId),
          getRouterBalances(settings.routerId),
          listRouterSwapReports(settings.routerId),
        ]);
        const reportRows =
          reports.status === "fulfilled" ? reports.value : null;
        return {
          settings,
          status: status.status === "fulfilled" ? status.value : null,
          balances: balances.status === "fulfilled" ? balances.value : null,
          earningsSats:
            reportRows?.reduce(
              (sum, report) => sum + report.feeEarnedSats,
              0,
            ) ?? null,
          reportCount: reportRows?.length ?? null,
        };
      }),
    );
    setRouters(rows);
  }, []);

  const refresh = useCallback(async () => {
    try {
      await load();
    } catch (error) {
      pushToast(
        "error",
        (error as { message?: string })?.message ??
          "Failed to load your routers.",
      );
    } finally {
      setLoading(false);
    }
  }, [load, pushToast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const stats = useMemo(() => {
    const running = routers.filter(
      (router) => router.status?.phase.phase === "running",
    ).length;
    return {
      running,
      stopped: routers.length - running,
      spendable: routers.reduce(
        (sum, router) => sum + (router.balances?.spendable ?? 0),
        0,
      ),
      earnings: routers.reduce(
        (sum, router) => sum + (router.earningsSats ?? 0),
        0,
      ),
    };
  }, [routers]);
  const visibleRouters = useMemo(
    () =>
      routers.filter(
        (router) =>
          filter === "all" ||
          (filter === "running"
            ? router.status?.phase.phase === "running"
            : router.status?.phase.phase !== "running"),
      ),
    [filter, routers],
  );

  // One stage covers both beats — the fleet loading behind the wordmark, and the create-first-
  // router form for an empty fleet — so the arrival never replays between them. No page padding:
  // the stage sets its own, and its backdrop has to reach the page edges.
  if (!introDone || loading || routers.length === 0) {
    return (
      <div className="h-full overflow-y-auto">
        <IntroStage
          lead="Welcome to"
          accent="Portal"
          caption="Your router dashboard"
          instant={introPlayed}
          onDone={() => {
            introPlayed = true;
            setIntroDone(true);
          }}
          className="min-h-full"
        >
          {loading ? (
            <div className="flex items-center justify-center gap-2.5 text-[12.5px] text-muted">
              <RefreshCw size={14} strokeWidth={1.9} className="animate-spin text-primary" />
              Loading your routers…
            </div>
          ) : (
            <RouterIntro onImported={() => void refresh()} />
          )}
        </IntroStage>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto p-8">
      <div className="mx-auto w-full max-w-[1500px]">
        <header className="flex items-start justify-between gap-6">
          <div>
            <div className="mb-3 flex items-center gap-2 font-mono text-[9.5px] font-semibold uppercase tracking-[0.18em] text-primary">
              <span className="h-1.5 w-1.5 rounded-full bg-primary shadow-[0_0_8px_color-mix(in_oklab,var(--color-primary)_70%,transparent)]" />
              Router Console · Signet
            </div>
            <h1 className="text-[28px] font-bold leading-none text-foreground">Router fleet</h1>
            <p className="mt-2 text-[12.5px] text-muted">Operate liquidity services, wallets, and earnings from one workspace.</p>
          </div>
          <div className="flex gap-2">
            <LinkButton to="/router/new"><Plus size={15} /> Add router</LinkButton>
          </div>
        </header>

        <div className="mt-6">
          <DashboardImport onImported={() => void refresh()} />
        </div>

        <StatStrip
          className="mt-6"
          items={[
            { label: "Routers", value: formatNumber(routers.length), detail: `${stats.running} running` },
            { label: "Running", value: formatNumber(stats.running), detail: `${stats.stopped} stopped`, tone: "success" },
            { label: "Spendable", value: <SatsAmount sats={stats.spendable} />, detail: "across router wallets" },
            { label: "Net earnings", value: <SatsAmount sats={stats.earnings} />, detail: "from saved reports", tone: "success" },
          ]}
        />

        <section className="mt-7">
          <div className="flex flex-wrap items-center justify-between gap-4">
            <div className="flex flex-wrap items-center gap-4">
              <h2 className="text-[20px] font-bold text-foreground">Routers</h2>
              <SegmentedToggle
                groupId="router-filter"
                value={filter}
                onChange={setFilter}
                options={[
                  {
                    value: "all",
                    label: "All",
                    suffix: <span>{routers.length}</span>,
                  },
                  {
                    value: "running",
                    label: "Running",
                    suffix: <span>{stats.running}</span>,
                  },
                  {
                    value: "stopped",
                    label: "Stopped",
                    suffix: <span>{stats.stopped}</span>,
                  },
                ]}
              />
            </div>
            <span className="font-mono text-[9.5px] uppercase tracking-[0.18em] text-subtle">{visibleRouters.length} shown</span>
          </div>

          {visibleRouters.length === 0 ? (
            <div className="mt-5 rounded-card border border-line-strong bg-surface-raised/45">
              <EmptyState
                size="lg"
                icon={<Inbox size={38} />}
                title="No routers in this view"
                description="Choose another status filter."
              />
            </div>
          ) : (
            <div className="mt-4 grid grid-cols-1 gap-4 lg:grid-cols-2 xl:grid-cols-3">
              {visibleRouters.map((router, i) => (
                <motion.div
                  key={router.settings.routerId}
                  initial={{ opacity: 0, y: 10 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{
                    duration: 0.42,
                    delay: i * 0.07,
                    ease: [0.16, 1, 0.3, 1],
                  }}
                >
                  <RouterCard router={router} onChanged={refresh} />
                </motion.div>
              ))}
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
