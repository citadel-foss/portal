import { AlertTriangle, ArrowRight, CheckCircle2, Clock, RefreshCw, ShieldCheck } from "lucide-react";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Link, useParams } from "react-router-dom";
import { getLogs, getRecoveryStatus, getSwapTracker, recoverSwap } from "../../api/commands";
import { isAppError } from "../../api/types";
import type {
  LogLine,
  RecoveredContract,
  RecoveryContract,
  RecoveryStatus,
  SwapTrackerProgress,
} from "../../api/types";
import {
  BackButton,
  Card,
  CopyButton,
  Disclosure,
  EmptyState,
  ExternalLinkButton,
  Identifier,
  LogViewer,
  MicroLabel,
  Notice,
  SatsAmount,
  StatusChip,
} from "../../components/ui/display";
import { Button } from "../../components/ui/inputs";
import { Checklist, type CheckState } from "../../components/ui/Checklist";
import { formatBlockWait } from "../../lib/wallet-format";
import { useToastStore } from "../../store/toast";
import { SwapCircuit } from "./circuit/SwapCircuit";
import { useSwapCircuit } from "./circuit/useSwapCircuit";

// The crate's recovery loop retries once a minute. This status read also consults the chain, so
// matching that cadence avoids opening several redundant Electrum connections per recovery pass.
const POLL_MS = 60_000;
const PHASE_LABEL: Record<string, string> = {
  preimage_stamped: "Preimage stamped",
  swapcoins_persisted: "Swapcoins persisted",
  incoming_recovered: "Incoming leg reclaimed",
  outgoing_recovered: "Outgoing leg reclaimed",
  cleaned_up: "Fully reclaimed",
};

function recoveryLabel(phase: string, running: boolean | undefined): string {
  if (phase !== "not_started") return PHASE_LABEL[phase] ?? phase;
  if (running === true) return "Recovery running";
  if (running === false) return "Recovery not running";
  return "Recovery status unavailable";
}

/** Whose money a settled contract turned out to be. The crate's resolution says how the output
 *  was spent, not by whom, so it reads differently per leg: an outgoing contract spent by
 *  someone else ("discarded") is the router taking its payment. */
function settled(r: RecoveredContract, swapReceived: boolean): { label: string; yours: boolean } {
  if (r.resolution === "unresolved") return { label: "Unresolved", yours: false };
  if (r.leg === "incoming") {
    return r.resolution === "hashlock" || r.resolution === "key_path"
      ? { label: "Claimed into your wallet", yours: true }
      : { label: "Taken back by the router", yours: false };
  }
  if (r.resolution === "timelock") return { label: "Refunded to you", yours: true };
  return r.resolution === "discarded" && !swapReceived
    ? { label: "Never funded — nothing to reclaim", yours: false }
    : { label: "Claimed by the router", yours: false };
}

function SettledRow({ contract, swapReceived }: { contract: RecoveredContract; swapReceived: boolean }) {
  const outcome = settled(contract, swapReceived);
  return (
    <div className="flex items-center justify-between gap-3 py-3">
      <span className="flex min-w-0 flex-col gap-1.5">
        <Identifier value={contract.contractTxid} className="text-[12px] leading-[1.45] text-muted" />
        <StatusChip tone={outcome.yours ? "success" : "subtle"} className="self-start">
          {outcome.label}
        </StatusChip>
      </span>
      {contract.spendingTxid && (
        <span className="flex flex-none items-center gap-2">
          <CopyButton text={contract.spendingTxid} />
          <ExternalLinkButton txid={contract.spendingTxid} />
        </span>
      )}
    </div>
  );
}

function Stat({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div>
      <MicroLabel>{label}</MicroLabel>
      <p className="mt-1.5 font-numeric text-[14px] text-foreground">{value}</p>
    </div>
  );
}

/** A block-by-block bar. Discrete rather than a smooth fill because the wait advances in
 *  whole blocks — a continuously creeping bar would imply progress between them that is not
 *  happening. */
function LockProgress({ elapsed, total }: { elapsed: number; total: number }) {
  return (
    <span className="mt-1.5 flex items-center gap-2">
      <span className="flex h-1.5 flex-1 overflow-hidden rounded-pill bg-white/[0.07]">
        <span
          className="h-full rounded-pill bg-warning transition-[width] duration-500"
          style={{ width: `${Math.min(100, (elapsed / total) * 100)}%` }}
        />
      </span>
      <span className="flex-none font-numeric text-[10.5px] text-subtle">
        {elapsed}/{total}
      </span>
    </span>
  );
}

function ContractRow({ contract }: { contract: RecoveryContract }) {
  if (contract.routerOwed) {
    return (
      <div className="flex items-center justify-between gap-3 py-3">
        <span className="flex min-w-0 flex-col gap-1.5">
          <Identifier
            value={`${contract.outpoint.txid}:${contract.outpoint.vout}`}
            className="text-[12px] leading-[1.45] text-muted"
          />
          <StatusChip tone="subtle" className="self-start">
            The router's payment — waiting for its claim
          </StatusChip>
        </span>
        <span className="flex flex-none items-center gap-2">
          <span className="font-numeric text-[12.5px] text-muted">
            <SatsAmount sats={contract.amountSats} />
          </span>
          <ExternalLinkButton txid={contract.outpoint.txid} />
        </span>
      </div>
    );
  }
  const blocks = contract.blocksRemaining;
  const waiting = blocks !== undefined && blocks > 0;
  const total = contract.lockBlocks;

  return (
    <div className="flex items-center justify-between gap-3 py-3">
      <span className="flex min-w-0 flex-col gap-1.5">
        <Identifier
          value={`${contract.outpoint.txid}:${contract.outpoint.vout}`}
          className="text-[12px] leading-[1.45] text-muted"
        />
        <span className="flex flex-wrap items-center gap-1.5">
          <StatusChip tone={waiting ? "warning" : "primary"} className="self-start">
            {contract.claimPath === "hashlock"
              ? "Claimable now"
              : waiting
                ? `Locked for ~${blocks} more blocks`
                : "Lock expired — claiming"}
          </StatusChip>
          {contract.confirmations === 0 && (
            <StatusChip tone="subtle" className="self-start">
              Unconfirmed — the lock starts when it confirms
            </StatusChip>
          )}
        </span>
        {waiting && total !== undefined && (
          <LockProgress elapsed={total - blocks} total={total} />
        )}
      </span>
      <span className="flex flex-none items-center gap-2">
        <span className="font-numeric text-[12.5px] text-foreground">
          <SatsAmount sats={contract.amountSats} />
        </span>
        <ExternalLinkButton txid={contract.outpoint.txid} />
      </span>
    </div>
  );
}

export function RecoveryPage() {
  const pushToast = useToastStore((s) => s.push);
  // Absent when something still links straight here rather than through the list, in which case
  // the newest unfinished swap is the one to show.
  const { swapId } = useParams();
  const [status, setStatus] = useState<RecoveryStatus | null>(null);
  const [tracker, setTracker] = useState<SwapTrackerProgress | null>(null);
  const [logs, setLogs] = useState<LogLine[]>([]);
  const [logsOpen, setLogsOpen] = useState(false);
  const [checking, setChecking] = useState(false);

  const load = useCallback(
    // `live` is checked after every await: opening one recovery and then another leaves the
    // first read in flight, and without this it lands afterwards and paints the first swap's
    // funds under the second swap's id.
    async (live: () => boolean) => {
      const next = await getRecoveryStatus(swapId);
      if (!live()) return;
      setStatus(next);
      if (!next.swapId) return;
      const tracker = await getSwapTracker(next.swapId);
      if (live()) setTracker(tracker);
    },
    [swapId],
  );

  useEffect(() => {
    let current = true;
    const live = () => current;
    void load(live).catch(() => {});
    const id = setInterval(() => void load(live).catch(() => {}), POLL_MS);
    return () => {
      current = false;
      clearInterval(id);
    };
  }, [load]);

  useEffect(() => {
    if (!logsOpen) return;
    void getLogs(120).then(setLogs).catch(() => {});
  }, [logsOpen]);

  const circuit = useSwapCircuit(tracker, null, true);

  async function checkNow() {
    setChecking(true);
    try {
      await recoverSwap();
      pushToast("success", "Recovery restarted.");
      // Always current: this only runs from a click on the page being viewed.
      await load(() => true);
    } catch (e) {
      pushToast("error", isAppError(e) ? e.message : "Could not start recovery.");
    } finally {
      setChecking(false);
    }
  }

  // Reading recovery status includes live chain queries for the tip and contract transactions.
  // Do not render zero-value recovery cards while that first authoritative read is still running.
  if (status === null) {
    return (
      <div className="h-full overflow-y-auto px-8 py-10">
        <div className="mx-auto flex w-full max-w-5xl flex-col gap-4">
          <BackButton to="/swap/recovery" label="Back to Recovery" />
          <Card className="grid min-h-[220px] place-items-center border-line-strong">
            <div className="flex flex-col items-center gap-2.5 text-center text-[13px] text-subtle">
              <RefreshCw size={28} strokeWidth={1.6} className="animate-spin text-primary" />
              <span>Reading recovery state from the wallet and chain…</span>
            </div>
          </Card>
        </div>
      </div>
    );
  }

  // A finished recovery still has a story to tell — it is reachable from the history, where the
  // question is "what happened to that swap?", not "is anything outstanding?".
  if (status && !status.active && status.swapId) {
    const refunded = status.resolved.some((r) => r.leg === "outgoing" && r.resolution === "timelock");
    return (
      <div className="h-full overflow-y-auto px-8 py-10">
        <div className="mx-auto flex w-full max-w-4xl flex-col gap-4">
          <BackButton to="/swap/recovery" label="Back to Recovery" />
          <div className="flex items-start gap-3">
            <span className="mt-0.5 grid h-10 w-10 flex-none place-items-center rounded-card border border-success/40 bg-success/[0.08] text-success">
              <CheckCircle2 size={20} strokeWidth={1.8} />
            </span>
            <div>
              <h1 className="font-header text-[26px] font-bold text-foreground">
                {status.swapReceived ? "Swap completed" : "Recovered"}
              </h1>
              <p className="mt-1 max-w-2xl text-[13.5px] leading-6 text-muted">
                {!status.swapReceived
                  ? "This swap stopped after its funds were committed, and Portal has claimed every contract back into your wallet. Nothing is outstanding."
                  : refunded
                    ? "You received your swapped coins, and the router's payment came back to you because the router backed out of the swap. Nothing is outstanding."
                    : "You received your swapped coins, and the router claimed its payment for routing them. Nothing is outstanding."}
              </p>
            </div>
          </div>

          <Card className="flex flex-col gap-4 border-line-strong p-5">
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-mono text-[12px] text-muted">{status.swapId}</span>
              <CopyButton text={status.swapId} title="Copy swap id" />
            </div>
            <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
              <Stat label="Amount" value={<SatsAmount sats={status.sendAmountSats} />} />
              <Stat label="Routers" value={String(status.routerCount)} />
              <Stat label="Contracts settled" value={String(status.resolved.length)} />
              <Stat
                label="Outcome"
                value={
                  status.swapReceived
                    ? "Completed"
                    : recoveryLabel(status.phase, status.recoveryRunning)
                }
              />
            </div>
            {status.failureReason && (
              <div>
                <MicroLabel>Why it stopped</MicroLabel>
                <p className="mt-1.5 break-words font-mono text-[11.5px] leading-5 text-muted">
                  {status.failureReason}
                </p>
              </div>
            )}
          </Card>

          {status.resolved.length > 0 && (
            <Card className="flex flex-col border-line-strong p-5">
              <h2 className="font-header text-[14px] font-bold text-foreground">
                How each contract settled
              </h2>
              <div className="mt-1 divide-y divide-line">
                {status.resolved.map((c) => (
                  <SettledRow
                    key={`${c.contractTxid}-${c.spendingTxid ?? ""}`}
                    contract={c}
                    swapReceived={status.swapReceived}
                  />
                ))}
              </div>
            </Card>
          )}
        </div>
      </div>
    );
  }

  if (status && !status.active) {
    return (
      <div className="h-full overflow-y-auto px-8 py-10">
        <div className="mx-auto w-full max-w-4xl">
          <BackButton to="/swap/recovery" label="Back to Recovery" />
          <EmptyState
            icon={<CheckCircle2 size={22} strokeWidth={1.8} />}
            title="Nothing to recover"
            description="No swap has funds sitting in a contract. If a swap stops after its funding transaction is broadcast, Portal claims the funds back and this page tracks it."
          />
        </div>
      </div>
    );
  }

  const blocks = status?.blocksRemaining;
  const pending = status?.pending ?? [];
  const yours = pending.filter((c) => !c.routerOwed);
  const routerOwed = pending.filter((c) => c.routerOwed);
  // The swap went through and nothing of the user's own is left in a contract: all that remains
  // is the router taking its payment, which Portal neither does nor can speed up.
  const routerSide = status?.swapReceived === true && yours.length === 0;
  // Router payments carry no lock, so they sort first and the contract holding everything up
  // is still the last one.
  const longest = pending[pending.length - 1];
  const lockTotal = longest?.lockBlocks;
  const lockElapsed =
    lockTotal !== undefined && blocks !== undefined ? Math.max(0, lockTotal - blocks) : undefined;
  // Nothing is counting down until the contracts confirm.
  const unconfirmed = yours.filter((c) => c.confirmations === 0).length;
  const waiting = blocks !== undefined && blocks > 0;
  const claimableNow = yours.filter(
    (c) => c.claimPath === "hashlock" || (c.blocksRemaining ?? 0) === 0,
  );
  const resolvedCount = status?.resolved.length ?? 0;
  const pendingCount = pending.length;
  const routerSettled = status?.resolved.find(
    (r) => r.leg === "outgoing" && r.resolution !== "unresolved",
  );

  // The three things that actually happen, in order. Each state is read off the contracts rather
  // than a timer: the crate writes no progress until a claim lands.
  const steps: { label: string; state: CheckState }[] = routerSide
    ? [
        { label: "Swapped coins received in your wallet", state: "passed" },
        routerOwed.length > 0
          ? { label: "Waiting for the router to claim its payment", state: "running" }
          : routerSettled?.resolution === "timelock"
            ? { label: "Refunded to you — the router backed out of the swap", state: "passed" }
            : routerSettled
              ? { label: "The router claimed its payment", state: "passed" }
              : // Spent, so gone from the wallet's coins, but the crate records it only once
                // the spend confirms.
                { label: "The router's claim is confirming", state: "running" },
        // Never "passed" here: a settled recovery renders the completed view instead.
        {
          label: "Swap settled",
          state: routerSettled && routerOwed.length === 0 ? "running" : "idle",
        },
      ]
    : [
        {
          label: "Funds identified in their contracts",
          state: pendingCount > 0 || resolvedCount > 0 ? "passed" : "running",
        },
        {
          label: waiting
            ? unconfirmed > 0
              ? `Waiting for the contracts to confirm · the ${lockTotal ?? blocks}-block refund lock starts then`
              : `Waiting out the refund lock · ${lockElapsed ?? 0} of ${lockTotal ?? blocks} blocks · ${formatBlockWait(blocks)} left`
            : "Refund lock matured",
          state: waiting ? "running" : pendingCount > 0 || resolvedCount > 0 ? "passed" : "idle",
        },
        {
          label:
            resolvedCount > 0 && yours.length === 0
              ? "Claimed back into your wallet"
              : claimableNow.length > 0 && !waiting
                ? "Broadcasting the claim transaction"
                : "Claiming back into your wallet",
          state: resolvedCount > 0 && yours.length === 0 ? "passed" : waiting ? "idle" : "running",
        },
      ];

  return (
    <div className="h-full overflow-y-auto px-8 py-10">
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-4">
        <BackButton to="/swap/recovery" label="Back to Recovery" />

        <div className="flex items-start gap-3">
          <span className="mt-0.5 grid h-10 w-10 flex-none place-items-center rounded-card border border-success/40 bg-success/[0.08] text-success">
            {routerSide ? (
              <CheckCircle2 size={20} strokeWidth={1.8} />
            ) : (
              <ShieldCheck size={20} strokeWidth={1.8} />
            )}
          </span>
          <div>
            <h1 className="font-header text-[26px] font-bold text-foreground">
              {routerSide ? "Your swap went through" : "Your funds are safe"}
            </h1>
            <p className="mt-1 max-w-2xl text-[13.5px] leading-6 text-muted">
              {routerSide ? (
                <>
                  You received your swapped coins. What is left in a contract is{" "}
                  <strong className="text-foreground">the router's payment</strong> for routing
                  them, not your money: the router claims it with its own key, and Portal leaves
                  it alone. Nothing here needs you to act.
                </>
              ) : (
                <>
                  This swap stopped after its funds were already committed, so they are sitting in
                  Bitcoin contracts that <strong className="text-foreground">only you</strong> can
                  spend. {status.recoveryRunning === false
                    ? "Recovery is not running. Select Check now to restart it."
                    : "Portal is claiming them back. Nothing here needs you to act."}
                </>
              )}
            </p>
          </div>
        </div>

        <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_360px]">
          <Card className="flex flex-col gap-5 border-line-strong p-5">
            <div>
              <h2 className="font-header text-[14px] font-bold text-foreground">
                What happens next
              </h2>
              <p className="mt-1 text-[11.5px] leading-5 text-muted">
                {routerSide
                  ? "Portal checks the chain every minute. This page follows it on its own."
                  : status.recoveryRunning === false
                    ? "Recovery is not running. Select Check now to restart it."
                    : status.recoveryRunning === true
                      ? "Portal retries every minute. This page follows it on its own."
                      : "Portal could not inspect the recovery worker while it was busy. This page will check again."}
              </p>
            </div>
            <Checklist steps={steps} />
            {waiting && (
              <p className="border-t border-line pt-4 text-[11.5px] leading-5 text-subtle">
                The wait is a delay written into the contract you signed, not network congestion —
                it exists so the other side has time to act first, and paying a higher fee cannot
                shorten it. Your coins cannot move anywhere else in the meantime.
              </p>
            )}
          </Card>

          <div className="flex flex-col gap-4">
            <Card className="flex flex-col gap-3 border-line-strong p-4.5">
              {!routerSide && (
                <div className="flex items-baseline justify-between gap-3">
                  <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
                    Yours, in contracts
                  </span>
                  <strong className="font-numeric text-[15px] text-foreground">
                    <SatsAmount sats={status?.lockedSats ?? 0} />
                  </strong>
                </div>
              )}
              {(routerSide || (status?.routerOwedSats ?? 0) > 0) && (
                <div className="flex items-baseline justify-between gap-3">
                  <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
                    Router's payment
                  </span>
                  <strong className="font-numeric text-[15px] text-muted">
                    {routerOwed.length > 0 ? <SatsAmount sats={status?.routerOwedSats ?? 0} /> : "Claimed"}
                  </strong>
                </div>
              )}
              {waiting && (
                <div className="flex flex-col gap-2 border-t border-line pt-3">
                  <div className="flex items-baseline justify-between gap-3">
                    <span className="text-[12px] text-muted">Refund lock progress</span>
                    <strong className="font-numeric text-[13px] text-warning">
                      {lockTotal === undefined
                        ? `~${blocks} blocks`
                        : `${lockElapsed} / ${lockTotal} blocks`}
                    </strong>
                  </div>
                  {lockTotal !== undefined && lockElapsed !== undefined && (
                    <LockProgress elapsed={lockElapsed} total={lockTotal} />
                  )}
                  {unconfirmed > 0 && (
                    <p className="text-[11.5px] leading-5 text-subtle">
                      Not counting yet — the delay runs from the contract confirming, and{" "}
                      {unconfirmed === 1 ? "one is" : `${unconfirmed} are`} still in the mempool.
                    </p>
                  )}
                </div>
              )}
              <div className="flex items-center gap-2 border-t border-line pt-3 text-[11.5px] text-subtle">
                <Clock size={13} strokeWidth={2} className="flex-none" />
                Checked every minute
              </div>
              <Button size="sm" variant="secondary" onClick={() => void checkNow()} loading={checking}>
                Check now
              </Button>
            </Card>

            {/* The claim is signed with this wallet's key, so only Portal can make it — but it
                is the maturing lock that gates it, not elapsed uptime. Telling the user to sit
                through the whole wait would be both wrong and unusable at ten hours. */}
            {routerSide ? (
              <Notice tone="primary" icon={<ShieldCheck size={16} strokeWidth={2} />}>
                Nothing to wait for here. The router claims its payment on its own, whether Portal
                is open or not. Portal refunds it to you only if the router backs out of the swap.
              </Notice>
            ) : (
              <Notice tone="warning" icon={<AlertTriangle size={16} strokeWidth={2} />}>
                {waiting ? (
                  <>
                    You don't have to wait here. Portal claims the funds itself, but only while it is
                    running — so quit if you like and reopen it once the lock has matured
                    {blocks !== undefined && ` (${formatBlockWait(blocks)})`}. The contracts are
                    unaffected by anything that happens in between.
                  </>
                ) : (
                  <>
                    Leave Portal open while it claims. The claim transactions are signed here, so
                    quitting pauses recovery until the next launch — the funds stay safe either way.
                  </>
                )}
              </Notice>
            )}

            <Card className="flex flex-col gap-2.5 border-line-strong p-4.5">
              <p className="text-[12px] leading-5 text-muted">
                Recovery holds nothing else up — send, receive and start another swap while it runs.
              </p>
              <Link
                to="/swap"
                className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-primary hover:text-primary-hover"
              >
                Start another swap
                <ArrowRight size={13} strokeWidth={2} />
              </Link>
            </Card>
          </div>
        </div>

        <Card className="flex flex-col border-line-strong">
          <header className="flex items-baseline gap-3 border-b border-line px-4.5 py-3.5">
            <h2 className="font-header text-[14px] font-bold text-foreground">Contracts</h2>
            <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
              {pendingCount} holding funds · {resolvedCount} settled
            </span>
          </header>
          <div className="flex flex-col divide-y divide-line px-4.5">
            {(status?.pending ?? []).map((c) => (
              <ContractRow key={`${c.outpoint.txid}:${c.outpoint.vout}`} contract={c} />
            ))}
            {(status?.resolved ?? []).map((r) => (
              <SettledRow key={r.contractTxid} contract={r} swapReceived={status?.swapReceived ?? false} />
            ))}
            {pendingCount === 0 && resolvedCount === 0 && (
              <p className="py-5 text-[12px] text-subtle">
                Reading the contracts off the chain…
              </p>
            )}
          </div>
        </Card>

        {tracker && (
          <Disclosure label="The route this swap was taking">
            <div className="flex justify-center pt-2">
              {/* Router labels hang *inside* the ring, so their clearance from the centre
                  readout falls with the radius. At 460 the two-router case put them straight
                  through the readout's bottom rows, which is its tallest state. */}
              <SwapCircuit view={circuit} maxSize={620} />
            </div>
          </Disclosure>
        )}

        {status?.failureReason && (
          <Disclosure label="Why the swap stopped">
            <p className="pt-1 text-[12px] leading-5 text-muted">{status.failureReason}</p>
          </Disclosure>
        )}

        <Disclosure label="Logs" onOpenChange={setLogsOpen}>
          <div className="pt-2">
            <LogViewer lines={logs} className="max-h-64" />
          </div>
        </Disclosure>
      </div>
    </div>
  );
}
