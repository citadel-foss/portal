import { AlertTriangle, ChevronRight, LifeBuoy, RefreshCw, ShieldCheck } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { getRecoveryStatus, listRecoveries } from "../../api/commands";
import type { RecoveryStatus, RecoverySummary } from "../../api/types";
import {
  BackButton,
  Card,
  EmptyState,
  Identifier,
  Notice,
  SatsAmount,
  StatStrip,
  StatusChip,
} from "../../components/ui/display";
import { Button } from "../../components/ui/inputs";
import { formatRelativeTime } from "../../lib/wallet-format";
import { useToastStore } from "../../store/toast";

// Matches the detail page: the crate's own recovery loop retries once a minute.
const POLL_MS = 12_000;

const PHASE_LABEL: Record<string, string> = {
  not_started: "Waiting to start",
  incoming_recovered: "Incoming leg reclaimed",
  outgoing_recovered: "Outgoing leg reclaimed",
  cleaned_up: "Finished",
};

export function RecoveriesPage() {
  const pushFailure = useToastStore((s) => s.pushFailure);
  const [rows, setRows] = useState<RecoverySummary[] | null>(null);
  // The contract pool belongs to the recovery as a whole rather than to any one swap, so the
  // headline figures come from the status read and the per-swap detail comes from the list.
  const [pool, setPool] = useState<RecoveryStatus | null>(null);

  const [failed, setFailed] = useState(false);

  const load = useCallback(async () => {
    try {
      const [list, status] = await Promise.all([listRecoveries(), getRecoveryStatus()]);
      setRows(list);
      setPool(status);
      setFailed(false);
    } catch (e) {
      // Deliberately keeps whatever was last read. Emptying the list here would render
      // "Nothing to recover" over a recovery that is still running, off nothing more than a
      // failed disk read — the one claim this page must never make wrongly.
      setFailed(true);
      pushFailure(e, "Failed to load recoveries.");
    }
  }, [pushFailure]);

  useEffect(() => {
    void load();
    const id = setInterval(() => void load(), POLL_MS);
    return () => clearInterval(id);
  }, [load]);

  return (
    <div className="h-full overflow-y-auto px-8 pb-8 pt-2">
      <div className="flex shrink-0 items-center gap-3 pb-4">
        <BackButton to="/swap" label="Back to Swap" />
        <div>
          <h1 className="font-header text-[26px] font-bold text-foreground">Recovery</h1>
          <p className="mt-1 text-[13.5px] text-muted">
            Swaps that stopped with funds still in a contract.
          </p>
        </div>
      </div>

      {rows === null && failed ? (
        <EmptyState
          icon={<AlertTriangle size={30} strokeWidth={1.6} />}
          title="Couldn't read the recovery state"
          description="This says nothing about your funds — the contracts are on-chain either way."
          action={
            <Button variant="secondary" size="sm" onClick={() => void load()}>
              Try again
            </Button>
          }
        />
      ) : rows === null ? (
        <div className="grid flex-1 place-items-center gap-2.5 text-center text-[13px] text-subtle">
          <RefreshCw size={28} strokeWidth={1.6} className="animate-spin text-primary" />
          <span>Loading recoveries…</span>
        </div>
      ) : rows.length === 0 ? (
        <EmptyState
          icon={<ShieldCheck size={30} strokeWidth={1.6} />}
          title="Nothing to recover"
          description="Every swap this wallet started either finished or had its funds reclaimed."
        />
      ) : (
        <>
          <StatStrip
            className="shrink-0"
            items={[
              { label: "Swaps recovering", value: String(rows.filter((r) => r.active).length) },
              {
                label: "Still in contracts",
                value: <SatsAmount sats={pool?.lockedSats ?? 0} />,
                tone: (pool?.lockedSats ?? 0) > 0 ? "warning" : "foreground",
              },
              { label: "Contracts waiting", value: String(pool?.pending.length ?? 0) },
            ]}
          />

          {rows.length > 1 && (
            <Notice tone="primary" className="mt-4">
              These are reclaimed together, not one at a time — the protocol runs a single recovery
              over every stopped swap at once. Opening one shows what it was doing when it stopped.
            </Notice>
          )}

          <Card className="mt-4 flex flex-col border-line-strong">
            <div className="grid grid-cols-[auto_1.3fr_0.9fr_1fr_0.6fr_0.9fr_auto] gap-3 border-b border-line px-4.5 py-3 font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
              <span />
              <span>Swap ID</span>
              <span>Stopped</span>
              <span>Recovery</span>
              <span>Routers</span>
              <span>Amount</span>
              <span />
            </div>
            <div className="flex flex-col divide-y divide-line">
              {rows.map((row) => (
                <Link
                  key={row.swapId}
                  to={`/swap/recovery/${encodeURIComponent(row.swapId)}`}
                  className="grid grid-cols-[auto_1.3fr_0.9fr_1fr_0.6fr_0.9fr_auto] items-center gap-3 px-4.5 py-3.5 text-[12.5px] outline-none hover:bg-hover focus-visible:shadow-ring"
                >
                  <LifeBuoy
                    size={15}
                    strokeWidth={1.9}
                    className={row.active ? "text-warning" : "text-success"}
                  />
                  <Identifier value={row.swapId} className="leading-[1.45] text-muted" />
                  <span className="text-subtle">{formatRelativeTime(row.updatedAt)}</span>
                  <span>
                    <StatusChip tone={row.active ? "warning" : "success"}>
                      {row.active
                        ? (PHASE_LABEL[row.phase] ?? row.phase)
                        : `Finished · ${row.resolvedCount} reclaimed`}
                    </StatusChip>
                  </span>
                  <span className="font-numeric">{row.routerCount}</span>
                  <span className="font-numeric">
                    <SatsAmount sats={row.sendAmountSats} />
                  </span>
                  <ChevronRight size={15} strokeWidth={1.9} className="text-subtle" />
                </Link>
              ))}
            </div>
          </Card>
        </>
      )}
    </div>
  );
}
