import { Braces, RefreshCw, ShieldCheck } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { getIncomingSwapUtxo, getOffers, getSwapReport, verifyDeniability } from "../../api/commands";
import type { ReportRouterFee, Offer, SwapReportDetail, SwapUtxo } from "../../api/types";
import { CopyButton, Modal, SatsAmount } from "../../components/ui/display";
import { routerName } from "../../lib/market-format";
import { Button, LinkButton } from "../../components/ui/inputs";
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
  TxidRow,
  identifiedAddress,
  protocolTags,
  satsToBtc,
} from "../../components/ui/report";
import { formatNumber } from "../../lib/wallet-format";

export function SwapReportPage() {
  const { swapId } = useParams<{ swapId: string }>();
  const navigate = useNavigate();
  const [report, setReport] = useState<SwapReportDetail | null>(null);
  const [notFound, setNotFound] = useState(false);
  const [selectedRouter, setSelectedRouter] = useState<{ index: number; address: string; fee?: ReportRouterFee } | null>(null);
  const [jsonOpen, setJsonOpen] = useState(false);
  const [offerByAddress, setOfferByAddress] = useState<Record<string, Offer>>({});
  const [incomingUtxo, setIncomingUtxo] = useState<SwapUtxo | null>(null);
  const [utxoState, setUtxoState] = useState<"idle" | "loading" | "done" | "failed">("idle");

  useEffect(() => {
    if (!swapId) return;
    void getSwapReport(swapId).then(setReport).catch(() => setNotFound(true));
  }, [swapId]);

  // Reports written before upstream PR #1006 don't record the sweep outputs, so for those the
  // received coin has to be read off the chain, at a round-trip per candidate transaction.
  const loadIncomingUtxo = useCallback(() => {
    if (!swapId || utxoState === "loading") return;
    setUtxoState("loading");
    void getIncomingSwapUtxo(swapId)
      .then((utxo) => {
        setIncomingUtxo(utxo);
        setUtxoState("done");
      })
      .catch(() => setUtxoState("failed"));
  }, [swapId, utxoState]);

  // Only entries we can actually name. An unnamed one must not satisfy this, or the chain
  // lookup below never runs and the page settles for showing nothing useful.
  const reportedIncoming = (report?.incomingUtxos ?? []).filter(identifiedAddress);
  useEffect(() => {
    if (swapId && report && reportedIncoming.length === 0 && utxoState === "idle") {
      loadIncomingUtxo();
    }
  }, [swapId, report, reportedIncoming.length, utxoState, loadIncomingUtxo]);

  // The report records neither router names nor bonds; both come from the routers' current
  // offers. Best-effort: a router that stopped posting offers keeps its "Router N" label. Read
  // again when a router's details open, since an offerbook still loading at mount has none yet.
  const offersRequest = useRef(0);
  const loadOffers = useCallback(() => {
    const request = ++offersRequest.current;
    void getOffers()
      .then((book) => {
        // An earlier lookup answering last must not replace a newer offerbook.
        if (request !== offersRequest.current) return;
        const map: Record<string, Offer> = {};
        for (const m of [...book.good, ...book.bad, ...book.unresponsive]) {
          if (m.offer) map[m.address] = m.offer;
        }
        setOfferByAddress(map);
      })
      .catch(() => {});
  }, []);
  useEffect(loadOffers, [loadOffers]);

  const routerLabel = (address: string, index: number) =>
    offerByAddress[address]?.name?.trim() || `Router ${index + 1}`;

  if (notFound) {
    return (
      <div className="grid h-full place-items-center gap-3 text-center">
        <p className="text-[13px] text-subtle">No report found for this swap.</p>
        <Button variant="secondary" onClick={() => navigate("/swap/reports")}>
          Back to Swap Reports
        </Button>
      </div>
    );
  }

  if (!report) return <ReportLoading what="swap report" />;

  const isFailure = report.status === "failed";

  return (
    <div className="flex h-full flex-col overflow-y-auto px-8 pb-8 pt-2">
      <ReportHeader
        backTo="/swap/reports"
        backLabel="Back to Swap Reports"
        swapId={report.swapId}
        status={report.status}
        action={
          <Button size="sm" variant="secondary" onClick={() => setJsonOpen(true)}>
            <Braces size={14} strokeWidth={1.8} />
            Report JSON
          </Button>
        }
      />

      {/* Everything that is not a completed swap, matching how the reports list counts them.
          A swap can stop in several ways and all of them can leave funds committed on-chain. */}
      {report.status !== "success" && (
        <ReportFailureBanner
          errorMessage={report.errorMessage}
          action={
            // What the reader actually wants next. Funds committed to a contract come back
            // through recovery, and this page cannot tell them whether that finished — so it
            // hands them straight there instead of leaving them to find it in the nav.
            <LinkButton
              to={`/swap/recovery/${encodeURIComponent(report.swapId)}`}
              size="sm"
              variant="secondary"
              className="flex-none"
            >
              <ShieldCheck size={14} strokeWidth={1.8} />
              Track recovery
            </LinkButton>
          }
        />
      )}

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1.3fr)_1fr]">
        <div className="flex flex-col gap-4">
          <ReportHero
            label={isFailure ? "Attempted Amount" : "Amount Swapped"}
            amountSats={report.outgoingAmountSats}
            // The settled figure, measured on-chain. The swap screen can only ever quote the
            // pre-swap ceiling, so this is the first place the real number exists.
            secondary={
              !isFailure && report.receivedAmountSats > 0 ? (
                <>
                  <SatsAmount sats={report.receivedAmountSats} className="text-success" /> received
                </>
              ) : undefined
            }
            tags={[...protocolTags(report.deniabilityProof), report.paySwap ? "PaySwap" : "Self swap"]}
            durationSeconds={report.swapDurationSeconds}
            startTimestamp={report.startTimestamp}
            endTimestamp={report.endTimestamp}
          />

          <SectionCard title="UTXOs">
            {report.outgoingUtxos.length > 0 && (
              <CoinRow
                label="Outgoing UTXOs"
                caption="The coins we spent for this swap."
                coins={report.outgoingUtxos}
                accent={OUTGOING_ACCENT}
                arrow="↗"
              />
            )}
            {reportedIncoming.length > 0 ? (
              <CoinRow
                label="Incoming UTXOs"
                caption="The coins we got back from this swap."
                coins={reportedIncoming}
                accent={HOP_ACCENTS[0]}
                arrow="↙"
              />
            ) : incomingUtxo ? (
              <TxArtifact
                label="Incoming UTXO"
                caption="The coins we got back from this swap."
                txid={incomingUtxo.txid}
                vout={incomingUtxo.vout}
                amountSats={incomingUtxo.amountSats}
                accent={HOP_ACCENTS[0]}
                arrow="↙"
              />
            ) : (
              // Not in the report file: the sweep that lands this coin happens after the report
              // is written, so it has to be read off the chain.
              <div className="flex flex-col gap-2 rounded-control border border-dashed border-line bg-surface-raised p-5">
                <h4 className="text-[15px] font-extrabold text-foreground">Incoming UTXO</h4>
                <p className="text-[11.5px] leading-5 text-subtle">
                  The coins we got back from this swap.
                </p>
                {/* The amount is recorded even when the outpoint is not, and it is the part
                    worth reading — so the card always carries it rather than being nothing
                    but an apology for what could not be resolved. */}
                {report.receivedAmountSats > 0 && (
                  <p className="font-numeric text-[13px] text-foreground">
                    <SatsAmount sats={report.receivedAmountSats} />
                    <span className="ml-1.5 text-[11.5px] text-subtle">received</span>
                  </p>
                )}
                {utxoState === "failed" ? (
                  <p className="text-[11.5px] text-danger">
                    Could not reach the chain backend to find it.
                  </p>
                ) : utxoState === "done" ? (
                  <p className="text-[11.5px] text-subtle">
                    The exact coin can't be pinned down yet — the sweep that lands it may not
                    have confirmed.
                  </p>
                ) : (
                  <p className="text-[11.5px] text-subtle">Reading the chain…</p>
                )}
                {utxoState !== "loading" && (
                  <Button size="sm" variant="secondary" onClick={loadIncomingUtxo}>
                    <RefreshCw size={14} strokeWidth={1.8} />
                    Try again
                  </Button>
                )}
              </div>
            )}
            {!incomingUtxo &&
              report.outgoingUtxos.length === 0 &&
              reportedIncoming.length === 0 &&
              utxoState === "done" && (
                <p className="text-[12px] text-subtle">No UTXO data recorded for this swap.</p>
              )}
          </SectionCard>
        </div>

        <div className="flex flex-col gap-4">
          <SectionCard title="Fee Details">
            <Row label="Received">
              <SatsAmount sats={report.receivedAmountSats} />
            </Row>
            <Row label="Router fees">
              <SatsAmount sats={report.totalRouterFeesSats} />
            </Row>
            <Row label="Mining fees">
              <SatsAmount sats={report.miningFeeSats} />
            </Row>
            <div className="mt-1 flex items-baseline justify-between gap-3 border-t border-dashed border-line pt-3.5">
              <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Total fee</span>
              <div className="text-right">
                <SatsAmount sats={report.feePaidSats} className="font-mono text-[26px] leading-none text-foreground" />
                <p className="mt-2 font-mono text-[12px] text-muted">{satsToBtc(report.feePaidSats)} BTC</p>
              </div>
            </div>
            <div className="flex items-center justify-between gap-3 border-t border-dashed border-line pt-3.5">
              <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">% Fees</span>
              <strong className="font-mono text-[15px] font-bold text-foreground">{report.feePercentage.toFixed(2)}%</strong>
            </div>
          </SectionCard>

          <SectionCard title={`Swap Partners (${report.routersCount})`}>
            {report.routerAddresses.length === 0 && <p className="text-[12px] text-subtle">No routers recorded.</p>}
            {report.routerAddresses.map((address, i) => {
              const fee = report.routerFeeInfo.find((m) => m.routerIndex === i) ?? report.routerFeeInfo[i];
              return (
                <button
                  key={address}
                  type="button"
                  onClick={() => {
                    setSelectedRouter({ index: i, address, fee });
                    loadOffers();
                  }}
                  className="lift flex items-center justify-between gap-3 rounded-card border border-line bg-surface-raised px-3.5 py-3 text-left outline-none hover:border-line-strong hover:bg-[var(--color-hover)] focus-visible:shadow-ring"
                >
                  <span className="flex min-w-0 flex-col gap-0.5">
                    <span className="truncate font-mono text-[11px] text-foreground">{routerLabel(address, i)}</span>
                    <span className="font-mono text-[10.5px] leading-[1.45] text-subtle">{routerName(address)}</span>
                  </span>
                  {fee && <SatsAmount sats={fee.totalFeeSats} className="flex-none text-[12px] font-semibold text-warning" />}
                </button>
              );
            })}
          </SectionCard>

          <DeniabilityCard
            swapId={report.swapId}
            proof={report.deniabilityProof}
            verify={() => verifyDeniability(report.swapId)}
          />
        </div>
      </div>

      {jsonOpen && (
        <Modal title="Swap report JSON" wide onClose={() => setJsonOpen(false)}>
          <div className="relative">
            <pre className="max-h-[60vh] overflow-auto rounded-control border border-line bg-surface-raised p-3.5 pr-14 font-mono text-[11px] leading-[1.6] text-muted">
              {report.raw}
            </pre>
            <div className="absolute right-2.5 top-2.5">
              <CopyButton text={report.raw} title="Copy JSON" />
            </div>
          </div>
        </Modal>
      )}

      {selectedRouter && (
        <Modal title={routerLabel(selectedRouter.address, selectedRouter.index)} onClose={() => setSelectedRouter(null)}>
          <Row label="Address">{routerName(selectedRouter.address)}</Row>
          <Row label="Route position">{selectedRouter.index + 1}</Row>
          {selectedRouter.fee ? (
            <>
              <Row label="Base fee">
                <SatsAmount sats={selectedRouter.fee.baseFeeSats} />
              </Row>
              <Row label="Amount-relative fee">
                <SatsAmount sats={selectedRouter.fee.amountRelativeFeeSats} />
              </Row>
              <Row label="Time-relative fee">
                <SatsAmount sats={selectedRouter.fee.timeRelativeFeeSats} />
              </Row>
              <Row label="Total fee">
                <SatsAmount sats={selectedRouter.fee.totalFeeSats} className="font-bold text-warning" />
              </Row>
            </>
          ) : (
            <p className="text-[12px] text-subtle">No fee breakdown recorded for this router.</p>
          )}

          <div className="mt-1 border-t border-dashed border-line pt-3">
            <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Fidelity Bond</span>
            {(() => {
              const bond = offerByAddress[selectedRouter.address];
              return bond ? (
                <div className="mt-2 flex flex-col gap-1.5">
                  <Row label="Bond amount">
                    <SatsAmount sats={bond.bondAmountSats} />
                  </Row>
                  <Row label="Locktime height">Block {formatNumber(bond.bondLocktimeHeight)}</Row>
                  <Row label="Status">{bond.bondIsSpent ? "Spent" : "Unspent"}</Row>
                  <TxidRow label="Bond Transaction" txid={bond.bondTxid} />
                </div>
              ) : (
                <p className="mt-2 text-[12px] text-subtle">
                  This router isn't in the current offerbook, so its fidelity bond can't be looked up.
                </p>
              );
            })()}
          </div>

          <div className="mt-1 border-t border-dashed border-line pt-3">
            <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Transactions</span>
            <div className="mt-2 flex flex-col gap-1.5">
              {(report.fundingTxids[selectedRouter.index] ?? []).map((txid, i) => (
                <TxidRow key={i} label={`Funding ${i + 1}`} txid={txid} />
              ))}
              {(report.fundingTxids[selectedRouter.index] ?? []).length === 0 && (
                <p className="text-[12px] text-subtle">No transactions recorded for this router.</p>
              )}
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}
