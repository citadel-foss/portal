import type { ReactNode } from "react";

import { explorerTxUrl, formatNumber } from "../../../lib/wallet-format";
import { EDGE_STAGE_LABEL, type CircuitView } from "./useSwapCircuit";

/**
 * Answers "how much longer", as a strip under the circuit. `Stage` is the circuit's own
 * headline rather than a second name for the same moment, so the two cannot drift apart.
 */
export function Vitals({
  view,
  elapsed,
}: {
  view: CircuitView;
  /** Owns its own tick, so the circuit doesn't re-render once a second. */
  elapsed: ReactNode;
}) {
  const total = view.routerCount + 1;
  return (
    <div className="grid grid-cols-2 gap-px overflow-hidden rounded-control border border-line bg-line sm:grid-cols-4">
      <VitalCell label="Elapsed" value={elapsed} />
      <VitalCell label="Confirmed" value={`${view.hopsConfirmed} of ${total} hops`} />
      <VitalCell label="ETA" value={view.etaLabel} />
      <VitalCell label="Stage" value={view.title} />
    </div>
  );
}

function VitalCell({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="flex flex-col items-center gap-1 bg-surface px-3 py-2.5">
      <span className="font-mono text-[9px] uppercase tracking-[0.18em] text-subtle">{label}</span>
      <span className="font-numeric text-[13px] text-foreground">{value}</span>
    </div>
  );
}

/**
 * The money question, under the circuit: which contract the live leg is, and what would happen
 * to the funds if the swap stopped here. What the swap is *doing* is the circuit's centre —
 * repeating it here would just be the same sentence twice on one screen.
 */
export function NowPanel({ view }: { view: CircuitView }) {
  const edge = view.liveEdgeIndex === null ? null : view.edges[view.liveEdgeIndex];

  const safety = view.failed
    ? "Your funds are recoverable through the contract's timelock or hashlock path."
    : view.focusIndex === null
      ? view.paymentAddress
        ? "Nothing is left in contract — the swap is settled and the receiver has been paid."
        : "Nothing is left in contract — the swap is settled and the coins are spendable."
      : view.committed
        ? "Your funds are locked in a contract you can reclaim after its refund window if this fails."
        : "Nothing is on-chain yet — this swap can still be cancelled with no loss.";

  return (
    <div
      className="rounded-control border bg-surface p-3"
      style={{ borderColor: view.failed ? "var(--color-danger)" : "var(--color-line)" }}
    >
      {edge && (
        <p className="font-mono text-[11px] text-foreground">
          {edge.index === 0
            ? `Your funding transaction${edge.contractCount > 1 ? "s" : ""}`
            : edge.index === view.routerCount
              ? `Router ${edge.index}'s contract${
                  edge.contractCount > 1 ? "s" : ""
                } — the one${edge.contractCount > 1 ? "s" : ""} that pay${
                  edge.contractCount > 1 ? "" : "s"
                } you`
              : `Router ${edge.index} → Router ${edge.index + 1}`}
          {edge.contractCount > 1 && ` · ${edge.contractCount} Splits`}
          {" · "}
          {EDGE_STAGE_LABEL[edge.stage]}
          {edge.amountSats !== undefined && ` · ${formatNumber(edge.amountSats)} sats`}
        </p>
      )}
      {/* One line per contract: a leg carries several, and run together they read as one
          unbroken string of hex. */}
      {edge?.txids.map((txid) => (
        <a
          key={txid}
          href={explorerTxUrl(txid)}
          target="_blank"
          rel="noreferrer"
          className="mt-1 block break-all font-mono text-[10px] text-primary underline decoration-dotted"
        >
          {txid}
        </a>
      ))}
      <p className="mt-1.5 font-mono text-[10px] text-subtle">{safety}</p>
      {view.failed && view.failureReason && (
        <p className="mt-1.5 break-all font-mono text-[9px] text-danger">{view.failureReason}</p>
      )}
    </div>
  );
}
