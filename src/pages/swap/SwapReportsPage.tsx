import { RefreshCw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { getRecoveryStatus, listSwapReports } from "../../api/commands";
import type { SwapReportSummary, SwapStatus } from "../../api/types";
import { BackButton, Card, Identifier, SatsAmount, StatStrip, StatusChip } from "../../components/ui/display";
import { SegmentedToggle, SortToggle } from "../../components/ui/inputs";
import {
  formatBlockWait,
  formatDuration,
  formatRelativeTime,
  swapStatusPresentation,
} from "../../lib/wallet-format";
import { useToastStore } from "../../store/toast";

type StatusFilter = "all" | "success" | "failed";
type SortField = "time" | "amount";

const STATUS_LABEL: Record<SwapStatus, string> = {
  success: "Success",
  recovery_hashlock: "Recovered (hashlock)",
  recovery_timelock: "Recovered (timelock)",
  // No report was written for these — the swap tracker is all there is. See `SwapStatus`.
  recovered: "Interrupted · recovered",
  interrupted: "Interrupted · recovering",
  unfinished: "Never finished",
  failed: "Failed",
};
const STATUS_TONE: Record<SwapStatus, "success" | "warning" | "danger"> = {
  success: "success",
  recovery_hashlock: "warning",
  recovery_timelock: "warning",
  recovered: "warning",
  interrupted: "warning",
  unfinished: "warning",
  failed: "danger",
};

export function SwapReportsPage() {
  const pushFailure = useToastStore((s) => s.pushFailure);
  const [reports, setReports] = useState<SwapReportSummary[] | null>(null);
  const [blocksLeft, setBlocksLeft] = useState<number | undefined>(undefined);
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [sortField, setSortField] = useState<SortField>("time");
  const [sortDir, setSortDir] = useState<Record<SortField, "asc" | "desc">>({ time: "desc", amount: "desc" });

  function toggleSort(field: SortField) {
    if (field === sortField) setSortDir((prev) => ({ ...prev, [field]: prev[field] === "desc" ? "asc" : "desc" }));
    else setSortField(field);
  }

  useEffect(() => {
    void listSwapReports()
      .then(setReports)
      .catch((e) => {
        setReports([]);
        pushFailure(e, "Failed to load swap reports.");
      });
    // The wait is a property of the contract pool rather than of any one swap, so it comes from
    // the recovery read rather than from the rows. Best-effort: a row without it simply shows no
    // countdown, which is how it read before.
    void getRecoveryStatus()
      .then((status) => setBlocksLeft(status.blocksRemaining))
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const filtered = useMemo(() => {
    const rows = (reports ?? []).filter((r) =>
      statusFilter === "all"
        ? true
        : statusFilter === "success"
          ? r.status === "success"
          : r.status !== "success",
    );
    const sorted = [...rows];
    const dir = sortDir[sortField] === "asc" ? 1 : -1;
    if (sortField === "time") sorted.sort((a, b) => (a.startTimestamp - b.startTimestamp) * dir);
    else sorted.sort((a, b) => (a.outgoingAmountSats - b.outgoingAmountSats) * dir);
    return sorted;
  }, [reports, statusFilter, sortField, sortDir]);

  const stats = useMemo(() => {
    const all = reports ?? [];
    // Anything that isn't a success. An interrupted swap has no report of its own, so counting
    // only the report file's `failed` reported zero while money was still in a contract.
    const unsuccessful = all.filter((r) => r.status !== "success").length;
    const totalVolume = all.reduce((sum, r) => sum + r.outgoingAmountSats, 0);
    const totalFees = all.reduce((sum, r) => sum + r.feePaidSats, 0);
    return { total: all.length, unsuccessful, totalVolume, totalFees };
  }, [reports]);

  return (
    <div className="h-full overflow-y-auto px-8 pb-8 pt-2">
      <div className="flex shrink-0 items-center gap-3 pb-4">
        <BackButton to="/swap" label="Back to Swap" />
        <div>
          <h1 className="font-header text-[26px] font-bold text-foreground">Swap Reports</h1>
          <p className="mt-1 text-[13.5px] text-muted">History of past swaps for this wallet.</p>
        </div>
      </div>

      {reports === null ? (
        <div className="grid flex-1 place-items-center gap-2.5 text-center text-[13px] text-subtle">
          <RefreshCw size={28} strokeWidth={1.6} className="animate-spin text-primary" />
          <span>Loading swap reports…</span>
        </div>
      ) : (
        <>
          <StatStrip
            className="shrink-0"
            items={[
              { label: "Total reports", value: String(stats.total) },
              {
                label: "Didn't complete",
                value: String(stats.unsuccessful),
                tone: stats.unsuccessful > 0 ? "warning" : "foreground",
              },
              { label: "Total volume", value: <SatsAmount sats={stats.totalVolume} />, tone: "primary" },
              { label: "Total fees", value: <SatsAmount sats={stats.totalFees} /> },
            ]}
          />

          <Card className="mt-4 flex shrink-0 flex-wrap items-center justify-between gap-3 border-line-strong px-4 py-3">
            <SegmentedToggle
              groupId="reports-status-filter"
              value={statusFilter}
              onChange={setStatusFilter}
              options={[
                { value: "all", label: "All" },
                { value: "success", label: "Success" },
                { value: "failed", label: "Didn't complete" },
              ]}
            />
            <SortToggle
              groupId="reports-sort"
              sortKey={sortField}
              sortDir={sortDir}
              onChange={toggleSort}
              options={[
                { key: "time", label: "Newest" },
                { key: "amount", label: "Amount" },
              ]}
            />
          </Card>

          <Card className="mt-4 flex min-h-[min(52vh,470px)] max-h-[min(68vh,680px)] flex-col border-line-strong">
            <div className="grid grid-cols-[auto_1.3fr_0.9fr_0.7fr_0.9fr_0.6fr_0.9fr] gap-3 border-b border-line px-4.5 py-3 font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
              <span />
              <span>Swap ID</span>
              <span>When</span>
              <span>Duration</span>
              <span>Amount</span>
              <span>Routers</span>
              <span>Fee</span>
            </div>
            <div className="flex flex-1 flex-col divide-y divide-line overflow-y-auto">
              {filtered.length === 0 && (
                <p className="px-4.5 py-8 text-center text-[13px] text-subtle">
                  {reports.length === 0 ? "No swap reports yet." : "No reports match this filter."}
                </p>
              )}
              {filtered.map((r) => {
                const { Icon, label: rawStatusLabel } = swapStatusPresentation(r.status);
                const cells = (
                  <>
                    <StatusChip tone={STATUS_TONE[r.status]} shape="tile" className="h-[34px] w-[34px] justify-center px-0"><Icon size={17} strokeWidth={2} /></StatusChip>
                    <span className="flex min-w-0 flex-col gap-1">
                      <Identifier value={r.swapId} className="text-[12px] leading-[1.45] text-muted" />
                      <StatusChip tone={STATUS_TONE[r.status] ?? "warning"} className="self-start">{STATUS_LABEL[r.status] ?? rawStatusLabel}</StatusChip>
                    </span>
                    <span className="font-mono text-[11.5px] text-subtle">
                      {r.endTimestamp === undefined
                        ? `started ${formatRelativeTime(r.startTimestamp)}`
                        : formatRelativeTime(r.endTimestamp)}
                    </span>
                    <span className="font-mono text-[11.5px] text-subtle">
                      {r.endTimestamp !== undefined ? (
                        formatDuration(r.endTimestamp - r.startTimestamp)
                      ) : r.status === "interrupted" && blocksLeft !== undefined ? (
                        // A swap still in recovery has no duration to report, but it does have a
                        // wait — which is the thing a reader actually wants off this row.
                        <span className="text-warning" title={formatBlockWait(blocksLeft)}>
                          ~{blocksLeft} blocks left
                        </span>
                      ) : (
                        "—"
                      )}
                    </span>
                    <SatsAmount sats={r.outgoingAmountSats} className="text-[12.5px] font-semibold text-foreground" />
                    <span className="font-mono text-[12px] text-foreground">{r.routersCount}</span>
                    {r.reported ? (
                      <SatsAmount sats={r.feePaidSats} className="text-[12px] text-warning" />
                    ) : (
                      <span className="font-mono text-[12px] text-subtle">—</span>
                    )}
                  </>
                );
                const grid =
                  "grid grid-cols-[auto_1.3fr_0.9fr_0.7fr_0.9fr_0.6fr_0.9fr] items-center gap-3 px-4.5 py-3 text-left outline-none transition-colors duration-200";
                const interactive =
                  "cursor-pointer hover:bg-[var(--color-hover)] focus-visible:shadow-[inset_0_0_0_1px_color-mix(in_oklab,var(--color-primary)_45%,transparent)]";
                // A tracker-only row has no report of its own, but it is a recovery — so it opens
                // the recovery for that swap instead, finished ones included. Only a swap that
                // never got as far as a recovery has nothing behind it at all.
                const target = r.reported
                  ? `/swap/reports/${encodeURIComponent(r.swapId)}`
                  : r.status === "interrupted" || r.status === "recovered"
                    ? `/swap/recovery/${encodeURIComponent(r.swapId)}`
                    : null;
                return target ? (
                  <Link key={r.swapId} to={target} className={`${grid} ${interactive}`}>
                    {cells}
                  </Link>
                ) : (
                  <div
                    key={r.swapId}
                    title="No report was written for this swap — these are the figures the swap tracker kept"
                    className={`${grid} opacity-70`}
                  >
                    {cells}
                  </div>
                );
              })}
            </div>
          </Card>
        </>
      )}
    </div>
  );
}
