import { AlertTriangle, ChevronRight, LifeBuoy } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { Link } from "react-router-dom";
import { listRecoveries, listSwapReports } from "../../api/commands";
import type { RecoverySummary, SwapReportSummary } from "../../api/types";
import { Card, SatsAmount } from "../../components/ui/display";
import { STATUS_LABEL, swapReportLink } from "../../components/ui/report";
import { formatRelativeTime, swapStatusPresentation } from "../../lib/wallet-format";
import { useRecoveryStore } from "../../store/recovery";

/** Rows each card shows; the rest is one click away on its own page. */
const RECENT = 3;

function SideCard({
  title,
  meta,
  to,
  className = "",
  children,
}: {
  title: string;
  meta?: ReactNode;
  to: string;
  className?: string;
  children: ReactNode;
}) {
  return (
    <Card className={`flex flex-col gap-3 p-5 ${className}`}>
      <div className="flex items-center justify-between gap-3">
        <h3 className="font-header text-[14px] font-bold text-foreground">{title}</h3>
        <Link
          to={to}
          className="flex items-center gap-0.5 text-[11.5px] font-semibold text-primary outline-none hover:text-primary-hover focus-visible:underline"
        >
          See all
          <ChevronRight size={13} strokeWidth={2.2} />
        </Link>
      </div>
      {meta && <p className="-mt-1.5 text-[11.5px] text-subtle">{meta}</p>}
      {children}
    </Card>
  );
}

function Row({ to, icon, when, amountSats, label }: {
  to: string | null;
  icon: ReactNode;
  when: string;
  amountSats: number;
  label: string;
}) {
  const body = (
    <>
      {icon}
      <span className="flex min-w-0 flex-col">
        <span className="truncate text-[12px] text-foreground">{label}</span>
        <span className="font-mono text-[10.5px] text-subtle">{when}</span>
      </span>
      <SatsAmount sats={amountSats} className="text-[12px] font-semibold text-foreground" />
    </>
  );
  const grid = "grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2.5 rounded-control px-2 py-1.5";
  return to ? (
    <Link to={to} className={`${grid} outline-none hover:bg-[var(--color-hover)] focus-visible:shadow-ring`}>
      {body}
    </Link>
  ) : (
    <div className={grid}>{body}</div>
  );
}

export function SwapReportsCard() {
  const [reports, setReports] = useState<SwapReportSummary[] | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    void listSwapReports()
      .then(setReports)
      .catch(() => setFailed(true));
  }, []);

  const recent = useMemo(
    () => [...(reports ?? [])].sort((a, b) => b.startTimestamp - a.startTimestamp).slice(0, RECENT),
    [reports],
  );
  const unfinished = (reports ?? []).filter((r) => r.status !== "success").length;

  return (
    <SideCard
      title="Swap reports"
      to="/swap/reports"
      className="border-line-strong"
      meta={
        reports &&
        reports.length > 0 &&
        `${reports.length} swap${reports.length === 1 ? "" : "s"}${unfinished ? ` · ${unfinished} didn't complete` : ""}`
      }
    >
      {failed ? (
        <p className="text-[12px] text-subtle">Couldn't read the swap reports.</p>
      ) : reports === null ? (
        <p className="text-[12px] text-subtle">Loading…</p>
      ) : reports.length === 0 ? (
        <p className="text-[12px] text-subtle">No swaps yet.</p>
      ) : (
        <div className="flex flex-col">
          {recent.map((r) => {
            const { Icon, tone, label } = swapStatusPresentation(r.status);
            return (
              <Row
                key={r.swapId}
                to={swapReportLink(r)}
                icon={<Icon size={15} strokeWidth={2} className={tone} />}
                label={STATUS_LABEL[r.status] ?? label}
                when={formatRelativeTime(r.endTimestamp ?? r.startTimestamp)}
                amountSats={r.outgoingAmountSats}
              />
            );
          })}
        </div>
      )}
    </SideCard>
  );
}

export function RecoveryCard() {
  const [rows, setRows] = useState<RecoverySummary[] | null>(null);
  const [failed, setFailed] = useState(false);
  const storeActive = useRecoveryStore((s) => s.active);

  useEffect(() => {
    void listRecoveries()
      .then(setRows)
      .catch(() => setFailed(true));
  }, []);

  const running = (rows ?? []).filter((r) => r.active);
  // Funds are in contracts while this is on; it has to read as loudly as the old header button.
  const active = storeActive || running.length > 0;

  return (
    <SideCard
      title="Recovery"
      to="/swap/recovery"
      className={active ? "border-warning/50" : "border-line-strong"}
    >
      {active && (
        <Link
          to={running.length === 1 ? `/swap/recovery/${encodeURIComponent(running[0].swapId)}` : "/swap/recovery"}
          className="flex items-center gap-2 rounded-control border border-warning/40 bg-warning/[0.08] px-3 py-2 text-[12px] font-semibold text-warning outline-none hover:bg-warning/[0.12] focus-visible:shadow-ring"
        >
          <AlertTriangle size={14} strokeWidth={2} className="flex-none" />
          {running.length > 1 ? `${running.length} recoveries in progress` : "Recovery in progress"}
          <ChevronRight size={13} strokeWidth={2.2} className="ml-auto flex-none" />
        </Link>
      )}
      {failed ? (
        <p className="text-[12px] text-subtle">Couldn't read the recovery history.</p>
      ) : rows === null ? (
        <p className="text-[12px] text-subtle">Loading…</p>
      ) : rows.length === 0 ? (
        <p className="text-[12px] text-subtle">No recoveries.</p>
      ) : (
        <div className="flex flex-col">
          {rows.slice(0, RECENT).map((r) => (
            <Row
              key={r.swapId}
              to={`/swap/recovery/${encodeURIComponent(r.swapId)}`}
              icon={
                <LifeBuoy size={15} strokeWidth={2} className={r.active ? "text-warning" : "text-success"} />
              }
              label={
                r.active
                  ? "Recovering"
                  : `Recovered${r.recoveryTypes.length ? ` · ${r.recoveryTypes.join(" + ")}` : ""}`
              }
              when={formatRelativeTime(r.updatedAt)}
              amountSats={r.sendAmountSats}
            />
          ))}
        </div>
      )}
    </SideCard>
  );
}
