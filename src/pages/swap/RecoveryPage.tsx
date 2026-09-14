import { AlertTriangle, ArrowRight, CheckCircle2, Clock, ShieldCheck } from "lucide-react";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Link, useParams } from "react-router-dom";
import { getLogs, getRecoveryStatus, getSwapTracker, recoverSwap } from "../../api/commands";
import { isAppError } from "../../api/types";
import type {
  LogLine,
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
  LogViewer,
  MicroLabel,
  Notice,
  SatsAmount,
  StatusChip,
} from "../../components/ui/display";
import { Button } from "../../components/ui/inputs";
import { Checklist, type CheckState } from "../../components/ui/Checklist";
import { formatBlockWait, truncateMiddle } from "../../lib/wallet-format";
import { useToastStore } from "../../store/toast";
import { SwapCircuit } from "./circuit/SwapCircuit";
import { useSwapCircuit } from "./circuit/useSwapCircuit";

// The crate's recovery loop retries once a minute, so anything faster only re-reads the same file.
const POLL_MS = 12_000;
const PHASE_LABEL: Record<string, string> = {
  not_started: "Not started",
  preimage_stamped: "Preimage stamped",
  swapcoins_persisted: "Swapcoins persisted",
  incoming_recovered: "Incoming leg reclaimed",
  outgoing_recovered: "Outgoing leg reclaimed",
  cleaned_up: "Fully reclaimed",
};

const RESOLUTION_LABEL: Record<string, string> = {
  hashlock: "Claimed with the preimage",
  timelock: "Refunded after the lock",
  key_path: "Swept with the key",
  discarded: "Discarded",
  unresolved: "Unresolved",
};

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
  const blocks = contract.blocksRemaining;
  const waiting = blocks !== undefined && blocks > 0;
  const total = contract.lockBlocks;

  return (
    <div className="flex items-center justify-between gap-3 py-3">
      <span className="flex min-w-0 flex-col gap-1.5">
        <span
          className="truncate font-mono text-[12px] text-muted"
          title={`${contract.outpoint.txid}:${contract.outpoint.vout}`}
        >
          {truncateMiddle(contract.outpoint.txid, 10, 6)}:{contract.outpoint.vout}
        </span>
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

  // A finished recovery still has a story to tell — it is reachable from the history, where the
  // question is "what happened to that swap?", not "is anything outstanding?".
  if (status && !status.active && status.swapId) {
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
                Recovered
              </h1>
              <p className="mt-1 max-w-2xl text-[13.5px] leading-6 text-muted">
                This swap stopped after its funds were committed, and Portal has claimed every
                contract back into your wallet. Nothing is outstanding.
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
              <Stat label="Contracts reclaimed" value={String(status.resolved.length)} />
              <Stat label="Outcome" value={PHASE_LABEL[status.phase] ?? status.phase} />
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
                How each contract came back
              </h2>
              <div className="mt-1 divide-y divide-line">
                {status.resolved.map((c) => (
                  <div
                    key={`${c.contractTxid}-${c.spendingTxid ?? ""}`}
                    className="flex items-center justify-between gap-3 py-3"
                  >
                    <span className="flex min-w-0 flex-col gap-1.5">
                      <span className="truncate font-mono text-[12px] text-muted" title={c.contractTxid}>
                        {truncateMiddle(c.contractTxid, 10, 6)}
                      </span>
                      <StatusChip tone="success" className="self-start">
                        {RESOLUTION_LABEL[c.resolution] ?? c.resolution}
                      </StatusChip>
                    </span>
                    {c.spendingTxid && <ExternalLinkButton txid={c.spendingTxid} />}
                  </div>
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
  // The pending list is sorted longest-wait-last, so the contract holding everything up is
  // the one whose lock the headline should count against.
  const longest = status?.pending?.[(status.pending.length ?? 1) - 1];
  const lockTotal = longest?.lockBlocks;
  const lockElapsed =
    lockTotal !== undefined && blocks !== undefined ? Math.max(0, lockTotal - blocks) : undefined;
  // Nothing is counting down until the contracts confirm.
  const unconfirmed = (status?.pending ?? []).filter((c) => c.confirmations === 0).length;
  const waiting = blocks !== undefined && blocks > 0;
  const claimableNow = (status?.pending ?? []).filter(
    (c) => c.claimPath === "hashlock" || (c.blocksRemaining ?? 0) === 0,
  );
  const resolvedCount = status?.resolved.length ?? 0;
  const pendingCount = status?.pending.length ?? 0;

  // The three things that actually happen, in order. Each state is read off the contracts rather
  // than a timer: the crate writes no progress until a claim lands.
  const steps: { label: string; state: CheckState }[] = [
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
        resolvedCount > 0 && pendingCount === 0
          ? "Claimed back into your wallet"
          : claimableNow.length > 0 && !waiting
            ? "Broadcasting the claim transaction"
            : "Claiming back into your wallet",
      state: resolvedCount > 0 && pendingCount === 0 ? "passed" : waiting ? "idle" : "running",
    },
  ];

  return (
    <div className="h-full overflow-y-auto px-8 py-10">
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-4">
        <BackButton to="/swap/recovery" label="Back to Recovery" />

        <div className="flex items-start gap-3">
          <span className="mt-0.5 grid h-10 w-10 flex-none place-items-center rounded-card border border-success/40 bg-success/[0.08] text-success">
            <ShieldCheck size={20} strokeWidth={1.8} />
          </span>
          <div>
            <h1 className="font-header text-[26px] font-bold text-foreground">
              Your funds are safe
            </h1>
            <p className="mt-1 max-w-2xl text-[13.5px] leading-6 text-muted">
              This swap stopped after its funds were already committed, so they are sitting in
              Bitcoin contracts that <strong className="text-foreground">only you</strong> can
              spend. Portal is claiming them back. Nothing here needs you to act.
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
                Portal retries every minute. This page follows it on its own.
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
              <div className="flex items-baseline justify-between gap-3">
                <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
                  Held in contracts
                </span>
                <strong className="font-numeric text-[15px] text-foreground">
                  <SatsAmount sats={status?.lockedSats ?? 0} />
                </strong>
              </div>
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
              {pendingCount} holding funds · {resolvedCount} claimed
            </span>
          </header>
          <div className="flex flex-col divide-y divide-line px-4.5">
            {(status?.pending ?? []).map((c) => (
              <ContractRow key={`${c.outpoint.txid}:${c.outpoint.vout}`} contract={c} />
            ))}
            {(status?.resolved ?? []).map((r) => (
              <div key={r.contractTxid} className="flex items-center justify-between gap-3 py-3">
                <span className="flex min-w-0 flex-col gap-1.5">
                  <span className="truncate font-mono text-[12px] text-muted" title={r.contractTxid}>
                    {truncateMiddle(r.contractTxid, 10, 6)}
                  </span>
                  <StatusChip tone="success" className="self-start">
                    Claimed back
                  </StatusChip>
                </span>
                {r.spendingTxid && (
                  <span className="flex flex-none items-center gap-2">
                    <CopyButton text={r.spendingTxid} />
                    <ExternalLinkButton txid={r.spendingTxid} />
                  </span>
                )}
              </div>
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
            <LogViewer lines={logs} className="max-h-64" newestFirst />
          </div>
        </Disclosure>
      </div>
    </div>
  );
}
