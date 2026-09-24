/**
 * A router's side of a swap, laid out exactly like the wallet's own report — same header,
 * same hero, same UTXO section, same proof card. The two describe one swap from its two ends,
 * so a reader who has learned to read one already knows where to look in the other.
 */
import { useEffect, useState } from "react";
import { useParams } from "react-router-dom";

import { getRouterSwapReport, verifyRouterDeniability } from "../../api/commands";
import type { RouterSwapReportDetail } from "../../api/types";
import { EmptyState, SatsAmount } from "../../components/ui/display";
import {
  CoinRow,
  DeniabilityCard,
  HOP_ACCENTS,
  OUTGOING_ACCENT,
  ReportFailureBanner,
  ReportHeader,
  ReportHero,
  ReportLoading,
  Row,
  SectionCard,
  TxArtifact,
  satsToBtc,
} from "../../components/ui/report";
import { formatDuration } from "../../lib/wallet-format";
import { useToastStore } from "../../store/toast";

export function RouterSwapReportPage() {
  const { routerId = "", swapId = "" } = useParams();
  const id = decodeURIComponent(routerId);
  const reportId = decodeURIComponent(swapId);
  const pushToast = useToastStore((s) => s.push);
  const [report, setReport] = useState<RouterSwapReportDetail | null>(null);
  const [notFound, setNotFound] = useState(false);

  useEffect(() => {
    void getRouterSwapReport(id, reportId)
      .then(setReport)
      .catch((e) => {
        setNotFound(true);
        pushToast("error", e.message);
      });
  }, [id, reportId, pushToast]);

  if (notFound)
    return (
      <EmptyState
        size="lg"
        title="Report unavailable"
        description="The router report could not be loaded."
      />
    );
  if (!report) return <ReportLoading what="router report" />;

  const isFailure = report.status === "failed";
  // What the router kept beyond its own service fee: the mining costs the taker reimbursed.
  const spread = report.incomingAmountSats - report.outgoingAmountSats;
  const earnedPct =
    report.incomingAmountSats > 0
      ? (report.feeEarnedSats / report.incomingAmountSats) * 100
      : 0;

  return (
    <div className="flex h-full flex-col overflow-y-auto px-8 pb-8 pt-2">
      <ReportHeader
        backTo={`/router/${encodeURIComponent(id)}`}
        backLabel="Back to router"
        swapId={report.swapId}
        status={report.status}
      />

      {/* A router's report records no failure reason — the crate's `MakerReport` has no field
          for one — so the banner says only that it did not complete. */}
      {report.status !== "success" && <ReportFailureBanner />}

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1.3fr)_1fr]">
        <div className="flex flex-col gap-4">
          <ReportHero
            label={isFailure ? "Attempted Route" : "Amount Routed"}
            amountSats={report.incomingAmountSats}
            secondary={
              !isFailure && report.feeEarnedSats > 0 ? (
                <>
                  <SatsAmount sats={report.feeEarnedSats} className="text-success" /> earned
                </>
              ) : undefined
            }
            network={report.network}
            durationSeconds={report.swapDurationSeconds}
            startTimestamp={report.startTimestamp}
            endTimestamp={report.endTimestamp}
          />

          <SectionCard title="UTXOs">
            {report.incomingContractOutpoint && (
              <TxArtifact
                label="Incoming UTXO"
                caption="The contract the previous hop paid into this router"
                txid={report.incomingContractOutpoint.txid}
                vout={report.incomingContractOutpoint.vout}
                accent={HOP_ACCENTS[0]}
                arrow="↙"
              />
            )}
            {report.incomingUtxos.length > 0 && (
              <CoinRow
                label="Incoming UTXOs"
                caption="The coins this router swept out of the incoming contract"
                coins={report.incomingUtxos}
                accent={HOP_ACCENTS[0]}
                arrow="↙"
              />
            )}
            {report.outgoingContractOutpoint && (
              <TxArtifact
                label="Outgoing UTXO"
                caption="The contract this router funded for the next hop"
                txid={report.outgoingContractOutpoint.txid}
                vout={report.outgoingContractOutpoint.vout}
                accent={OUTGOING_ACCENT}
                arrow="↗"
              />
            )}
            {report.outgoingUtxos.length > 0 && (
              <CoinRow
                label="Outgoing UTXOs"
                caption="The router's own coins spent to fund the outgoing contract"
                coins={report.outgoingUtxos}
                accent={OUTGOING_ACCENT}
                arrow="↗"
              />
            )}
            {!report.incomingContractOutpoint &&
              !report.outgoingContractOutpoint &&
              report.incomingUtxos.length === 0 &&
              report.outgoingUtxos.length === 0 && (
                <p className="text-[12px] text-subtle">No UTXO data recorded for this swap.</p>
              )}
          </SectionCard>
        </div>

        <div className="flex flex-col gap-4">
          <SectionCard title="Fee Details">
            <Row label="Received">
              <SatsAmount sats={report.incomingAmountSats} />
            </Row>
            <Row label="Forwarded">
              <SatsAmount sats={report.outgoingAmountSats} />
            </Row>
            {/* Distinct from the fee: the difference also carries the mining costs the taker
                reimbursed, which the router spends again on the outgoing contract. */}
            <Row label="Spread">
              <SatsAmount sats={spread} className={spread >= 0 ? "text-success" : "text-danger"} />
            </Row>
            <div className="mt-1 flex items-baseline justify-between gap-3 border-t border-dashed border-line pt-3.5">
              <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Fee earned</span>
              <div className="text-right">
                <SatsAmount
                  sats={report.feeEarnedSats}
                  className="font-mono text-[26px] leading-none text-success"
                />
                <p className="mt-2 font-mono text-[12px] text-muted">{satsToBtc(report.feeEarnedSats)} BTC</p>
              </div>
            </div>
            <div className="flex items-center justify-between gap-3 border-t border-dashed border-line pt-3.5">
              <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">% Earned</span>
              <strong className="font-mono text-[15px] font-bold text-foreground">{earnedPct.toFixed(2)}%</strong>
            </div>
          </SectionCard>

          <SectionCard title="Contract Terms">
            <Row label="Timelock">{report.timelock.toLocaleString()} blocks</Row>
            <Row label="Duration">{formatDuration(report.swapDurationSeconds)}</Row>
            <Row label="Network">{report.network}</Row>
          </SectionCard>

          <DeniabilityCard
            swapId={report.swapId}
            proof={report.deniabilityProof}
            verify={() => verifyRouterDeniability(id, reportId)}
          />
        </div>
      </div>
    </div>
  );
}
