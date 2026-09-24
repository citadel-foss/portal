import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import { Globe, KeyRound, Wallet } from "lucide-react";

import { routerName } from "../../../lib/market-format";
import { explorerTxUrl } from "../../../lib/wallet-format";
import { buildCircuit, edgeStrands, labelAnchor, type CircuitGeometry } from "./geometry";
import {
  EDGE_STAGE_LABEL,
  STAGE_LABEL,
  type CircuitView,
  type EdgeView,
  type HopView,
  type Tone,
} from "./useSwapCircuit";

const STROKE: Record<Tone, string> = {
  idle: "var(--color-line-strong)",
  active: "var(--color-primary)",
  success: "var(--color-success)",
  danger: "var(--color-danger)",
};

const ROUTER_STROKE: Record<Tone, string> = {
  ...STROKE,
  active: "var(--color-router)",
};

type Inspect = { kind: "node" | "edge" | "wallet"; index: number };

/**
 * The swap circuit: one wallet node owning both ends, one node per router, and one edge per
 * contract transaction. The loop is the point — the wallet that funds the first contract is the
 * wallet that receives the last one, so it is drawn once with an out-port and an in-port rather
 * than as two nodes at opposite ends of a line.
 */
export function SwapCircuit({
  view,
  maxSize = 700,
}: {
  view: CircuitView;
  maxSize?: number;
}) {
  const reduceMotion = useReducedMotion();
  // Hovering shows detail; clicking pins it, so the reader can move the mouse away without
  // losing what they were looking at.
  const [hovered, setHovered] = useState<Inspect | null>(null);
  const [pinned, setPinned] = useState<Inspect | null>(null);
  const inspect = hovered ?? pinned;
  const onHover = (next: Inspect | null) => setHovered(next);
  const onSelect = (next: Inspect) =>
    setPinned((prev) => (prev && prev.kind === next.kind && prev.index === next.index ? null : next));
  const host = useRef<HTMLDivElement>(null);
  const [available, setAvailable] = useState(0);

  // The circuit is a circle, so its height tracks its width — it fills the row it is given up to
  // maxSize rather than taking a fixed size, which kept it small inside a wide card.
  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => setAvailable(entry.contentRect.width));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // geo.size may exceed the requested size: buildCircuit grows the ring when a long route
  // would otherwise collide nodes.
  const geo = buildCircuit(view.routerCount, Math.min(available || maxSize, maxSize));
  const canvas = geo.size;

  return (
    <div ref={host} className="w-full">
    <div
      className={`relative mx-auto ${
        view.act === "finalizing" && !reduceMotion ? "circuit-heartbeat" : ""
      }`}
      style={{ width: canvas, height: canvas }}
    >
      <svg
        width={canvas}
        height={canvas}
        className="absolute left-0 top-0 overflow-visible"
        role="img"
        aria-label={circuitLabel(view)}
      >
        {geo.edges.map((edge) => (
          <CircuitEdge
            key={edge.index}
            geo={geo}
            edge={edge}
            view={view.edges[edge.index]}
            reduceMotion={!!reduceMotion}
            onHover={onHover}
            onSelect={onSelect}
          />
        ))}
      </svg>

      {view.hops.map((hop) => (
        <RouterNode
          key={hop.index}
          geo={geo}
          hop={hop}
          reduceMotion={!!reduceMotion}
          onHover={onHover}
          onSelect={onSelect}
        />
      ))}

      {view.hops.map((hop) => (
        <RouterLabel key={`label-${hop.index}`} geo={geo} hop={hop} />
      ))}

      <CenterReadout geo={geo} view={view} inspect={inspect} />

      {/* Painted last: the wallet is the one node that must never be occluded, and on a long
          route its card reaches close to the ticks either side of the seam. */}
      <WalletNode geo={geo} view={view} onHover={onHover} />
    </div>
    </div>
  );
}

function circuitLabel(view: CircuitView) {
  const total = view.routerCount + 1;
  if (view.failed) return `Swap circuit: failed at hop ${(view.focusIndex ?? 0) + 1} of ${total}`;
  if (view.focusIndex === null) return `Swap circuit: complete, ${total} of ${total} hops confirmed`;
  return `Swap circuit: ${view.hopsConfirmed} of ${total} hops confirmed, working on hop ${
    view.focusIndex + 1
  }`;
}

function CircuitEdge({
  geo,
  edge,
  view,
  reduceMotion,
  onHover,
  onSelect,
}: {
  geo: CircuitGeometry;
  edge: CircuitGeometry["edges"][number];
  view: EdgeView;
  reduceMotion: boolean;
  onHover?: (i: Inspect | null) => void;
  onSelect?: (i: Inspect) => void;
}) {
  const flowing = view.stage === "broadcast";
  const confirming = view.stage === "confirming";
  const label = labelAnchor(edge, geo.tier === "A" ? 30 : 18);
  const strands = edgeStrands(geo, edge, view.contractCount);

  return (
    <g
      tabIndex={0}
      role="button"
      aria-label={`Contract leg ${edge.index + 1}: ${EDGE_STAGE_LABEL[view.stage]}${
        strands.length > 1 ? `, ${strands.length} transactions` : ""
      }`}
      onMouseEnter={() => onHover?.({ kind: "edge", index: edge.index })}
      onMouseLeave={() => onHover?.(null)}
      onFocus={() => onHover?.({ kind: "edge", index: edge.index })}
      onBlur={() => onHover?.(null)}
      onClick={() => onSelect?.({ kind: "edge", index: edge.index })}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect?.({ kind: "edge", index: edge.index });
        }
      }}
      style={{ pointerEvents: "stroke", cursor: "pointer" }}
    >
      {/* Invisible fat stroke so the thin arc is actually hoverable. */}
      <path d={edge.d} fill="none" stroke="transparent" strokeWidth={18 + (strands.length - 1) * 5} />
      {strands.map((strand, i) => (
        <motion.path
          key={i}
          d={strand.d}
          fill="none"
          stroke={STROKE[view.tone]}
          // Split strands thin out so a pair reads as one leg carrying two txs rather than
          // as two legs.
          strokeWidth={strands.length > 1 ? 1.75 : 2.5}
          strokeLinecap="round"
          // Analytic length from geometry.ts — never pathLength, which measures in JS and emits
          // px dash values that WKWebView scales wrong.
          strokeDasharray={flowing || confirming ? undefined : strand.length}
          initial={reduceMotion ? false : { strokeDashoffset: strand.length }}
          animate={{ strokeDashoffset: 0, opacity: view.stage === "pending" ? 0.35 : 1 }}
          // Strands are staggered so the eye counts them as they draw; without it a pair of
          // parallel arcs animating in lockstep just looks like one thick stroke.
          transition={
            reduceMotion
              ? { duration: 0 }
              : { duration: 0.55, delay: edge.index * 0.05 + i * 0.12, ease: [0.16, 1, 0.3, 1] }
          }
          className={
            flowing ? "circuit-edge-flowing" : confirming ? "circuit-edge-confirming" : undefined
          }
        />
      ))}
      {geo.tier === "A" && (
        <EdgeLabel x={label.x} y={label.y} view={view} />
      )}
      {view.stage === "confirmed" &&
        !reduceMotion &&
        strands.map((strand, i) => (
          <CoinToken key={i} d={strand.d} tone={view.tone} delay={i * 0.35} />
        ))}
    </g>
  );
}

function EdgeLabel({ x, y, view }: { x: number; y: number; view: EdgeView }) {
  const badge =
    view.stage === "confirmed"
      ? "✓ confirmed"
      : view.stage === "pending"
        ? ""
        : `◌ ${EDGE_STAGE_LABEL[view.stage].toLowerCase()}`;
  // One leg can carry several contracts; the first is enough to open the right explorer, and
  // the centre readout lists every one of them.
  const txid = view.txids[0];
  return (
    <g style={{ pointerEvents: "none" }}>
      {view.amountSats !== undefined && (
        <text
          x={x}
          y={y - 3}
          textAnchor="middle"
          className="fill-muted"
          style={{ font: "500 9px var(--font-numeric)" }}
        >
          {view.amountSats.toLocaleString()}
        </text>
      )}
      {/* The strands themselves are deliberately thin, so the count is spelled out rather
          than left to be read off the stroke. */}
      {view.contractCount > 1 && (
        <text
          x={x}
          y={y + 6}
          textAnchor="middle"
          className="fill-subtle"
          style={{ font: "500 8px var(--font-mono)" }}
        >
          {view.contractCount} Splits
        </text>
      )}
      {badge &&
        (txid ? (
          <a
            href={explorerTxUrl(txid)}
            target="_blank"
            rel="noreferrer"
            style={{ pointerEvents: "auto", cursor: "pointer" }}
          >
            <title>Open this contract on mempool</title>
            <text
              x={x}
              y={y + (view.contractCount > 1 ? 16 : 8)}
              textAnchor="middle"
              fill={STROKE[view.tone]}
              style={{ font: "500 8px var(--font-mono)", textDecoration: "underline" }}
            >
              {badge}
            </text>
          </a>
        ) : (
          <text
            x={x}
            y={y + (view.contractCount > 1 ? 16 : 8)}
            textAnchor="middle"
            fill={STROKE[view.tone]}
            style={{ font: "500 8px var(--font-mono)" }}
          >
            {badge}
          </text>
        ))}
    </g>
  );
}

/** One shot on confirmation — the value actually moving to the next hop. */
function CoinToken({ d, tone, delay = 0 }: { d: string; tone: Tone; delay?: number }) {
  return (
    <motion.circle
      r={4}
      fill={STROKE[tone]}
      style={{ offsetPath: `path("${d}")`, offsetRotate: "0deg" }}
      initial={{ offsetDistance: "0%", opacity: 0 }}
      animate={{ offsetDistance: "100%", opacity: [0, 1, 1, 0] }}
      transition={{ duration: 0.9, delay, ease: "easeInOut" }}
    />
  );
}

/**
 * Act 5 relays private keys forward: the wallet's key goes to router 1, router 1's key comes
 * back and goes to router 2, and so on. It is the conceptual heart of the protocol, so it gets
 * its own glyph and accent rather than reusing the coin token.
 */

function WalletNode({
  geo,
  view,
  onHover,
}: {
  geo: CircuitGeometry;
  view: CircuitView;
  onHover?: (i: Inspect | null) => void;
}) {
  const slot = geo.slots[0];
  const width = geo.walletWidth;
  const height = geo.walletHeight;
  const done = view.complete && !view.failed;

  return (
    <motion.div
      className="circuit-node absolute flex flex-col items-center"
      style={{ left: slot.center.x - width / 2, top: slot.center.y - height / 2, width }}
      onMouseEnter={() => onHover?.({ kind: "wallet", index: -1 })}
      onMouseLeave={() => onHover?.(null)}
      animate={done ? { scale: [1, 1.05, 1] } : { scale: 1 }}
      transition={{ duration: 0.5 }}
    >
      <div
        className={`relative grid w-full place-items-center rounded-card border-2 bg-surface-raised ${
          view.committed ? "border-solid" : "border-dashed"
        }`}
        style={{
          height,
          borderColor: view.failed ? "var(--color-danger)" : "var(--color-primary)",
          boxShadow: done ? "var(--shadow-glow)" : undefined,
        }}
      >
        <Wallet size={20} strokeWidth={1.8} className="text-primary" />
        <span className="mt-0.5 font-header text-[9px] font-bold uppercase tracking-wide text-foreground">
          Your Wallet
        </span>
        {/* Two ports on one node: money leaves one side and returns to the other. Flow runs
            clockwise, so out is the right-hand port and in is the left-hand one. */}
        <span className="absolute -bottom-2 left-3 rounded-pill border border-line bg-surface px-1.5 font-mono text-[8px] text-subtle">
          ◂ in
        </span>
        <span className="absolute -bottom-2 right-3 rounded-pill border border-line bg-surface px-1.5 font-mono text-[8px] text-subtle">
          out ▸
        </span>
      </div>
    </motion.div>
  );
}

function RouterNode({
  geo,
  hop,
  reduceMotion,
  onHover,
  onSelect,
}: {
  geo: CircuitGeometry;
  hop: HopView;
  reduceMotion: boolean;
  onHover?: (i: Inspect | null) => void;
  onSelect?: (i: Inspect) => void;
}) {
  const slot = geo.slots[hop.index + 1];
  const s = geo.nodeSize;
  const active = hop.tone === "active";

  return (
    <motion.button
      type="button"
      className="circuit-node absolute flex flex-col items-center gap-1 focus:outline-none"
      style={{ left: slot.center.x - s / 2, top: slot.center.y - s / 2, width: s }}
      initial={reduceMotion ? false : { opacity: 0, scale: 0.8 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={{ duration: 0.35, delay: reduceMotion ? 0 : hop.index * 0.05 }}
      onMouseEnter={() => onHover?.({ kind: "node", index: hop.index })}
      onMouseLeave={() => onHover?.(null)}
      onFocus={() => onHover?.({ kind: "node", index: hop.index })}
      onBlur={() => onHover?.(null)}
      onClick={() => onSelect?.({ kind: "node", index: hop.index })}
      aria-label={`${hop.label}: ${STAGE_LABEL[hop.stage]}`}
    >
      <div
        className="relative grid place-items-center rounded-full border-2 bg-surface-raised"
        style={{
          width: s,
          height: s,
          borderColor: ROUTER_STROKE[hop.tone],
          borderStyle: hop.stage === "waiting" ? "dashed" : "solid",
          boxShadow: active ? "0 0 16px color-mix(in oklab, var(--color-router) 45%, transparent)" : undefined,
        }}
      >
        {geo.tier !== "C" &&
          (hop.stage === "settled" ? (
            <KeyRound size={geo.tier === "A" ? 22 : 14} strokeWidth={1.8} className="text-success" />
          ) : (
            <Globe
              size={geo.tier === "A" ? 22 : 14}
              strokeWidth={1.8}
              style={{ color: ROUTER_STROKE[hop.tone] }}
            />
          ))}
      </div>
    </motion.button>
  );
}

/**
 * Router identity and state sit inside the ring, on the radius through the node, so the route
 * reads as one object instead of a diagram with a legend hanging off the outside of it.
 */
function RouterLabel({ geo, hop }: { geo: CircuitGeometry; hop: HopView }) {
  if (geo.tier === "C") return null;
  const slot = geo.slots[hop.index + 1];
  // Only name and state go inside: a radial inset alone does not clear a wide label when the
  // node sits on a diagonal, and the address is a hover detail rather than route structure.
  const inset = geo.nodeSize / 2 + (geo.tier === "A" ? 44 : 26);
  const x = geo.center.x + (geo.radius - inset) * Math.cos(slot.angle);
  const y = geo.center.y + (geo.radius - inset) * Math.sin(slot.angle);
  return (
    <div
      className="pointer-events-none absolute flex -translate-x-1/2 -translate-y-1/2 flex-col items-center"
      style={{ left: x, top: y }}
    >
      <span
        className="whitespace-nowrap font-header text-[9px] font-bold uppercase tracking-wide"
        style={{ color: ROUTER_STROKE[hop.tone] }}
      >
        {hop.label}
      </span>
      {geo.tier === "A" && (
        <span className="whitespace-nowrap font-mono text-[8.5px] text-subtle">
          {STAGE_LABEL[hop.stage]}
        </span>
      )}
    </div>
  );
}

/**
 * Everything the circuit has to say renders here, in the middle of the ring: the stage by
 * default, and the detail for whatever node or edge is being inspected. A floating card was the
 * obvious alternative, but it has to be positioned against the viewport and fell off-screen for
 * anything in the lower half of a ring this size.
 */
function CenterReadout({
  geo,
  view,
  inspect,
}: {
  geo: CircuitGeometry;
  view: CircuitView;
  inspect: Inspect | null;
}) {
  const width = Math.min(340, geo.radius * 1.35);
  return (
    <div
      className="pointer-events-none absolute -translate-x-1/2 -translate-y-1/2"
      style={{ left: geo.center.x, top: geo.center.y, width }}
    >
      <AnimatePresence mode="wait">
        <motion.div
          key={inspect ? `${inspect.kind}-${inspect.index}` : "stage"}
          initial={{ opacity: 0, y: 6 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -6 }}
          transition={{ duration: 0.18 }}
          className="flex flex-col items-center"
        >
          {inspect === null && <StageBody view={view} />}
          {inspect?.kind === "node" && <NodeBody hop={view.hops[inspect.index]} view={view} />}
          {inspect?.kind === "edge" && <EdgeBody edge={view.edges[inspect.index]} view={view} />}
          {inspect?.kind === "wallet" && <WalletBody view={view} />}
        </motion.div>
      </AnimatePresence>
    </div>
  );
}

function Heading({ children, tone }: { children: ReactNode; tone?: string }) {
  return (
    <span
      className="font-header text-[17px] font-bold uppercase tracking-[0.1em]"
      style={{ color: tone ?? "var(--color-foreground)" }}
    >
      {children}
    </span>
  );
}

function Sub({ children }: { children: ReactNode }) {
  return <span className="mt-0.5 text-center font-mono text-[10px] text-subtle">{children}</span>;
}

function StageBody({ view }: { view: CircuitView }) {
  const done = view.complete && !view.failed;
  const tone = view.failed
    ? "var(--color-danger)"
    : done
      ? "var(--color-success)"
      : "var(--color-primary)";

  const line = view.failed
    ? `Stopped at hop ${(view.focusIndex ?? 0) + 1} of ${view.routerCount + 1}`
    : done
      ? `${view.routerCount + 1} hops through ${view.routerCount} routers`
      : view.focusIndex === null
        // Every hop has settled but the swap has not finished — the incoming contract is still
        // being swept — so there is no hop to count.
        ? `${view.routerCount + 1} hops settled`
        : `Hop ${(view.focusIndex ?? 0) + 1} of ${view.routerCount + 1}`;

  return (
    <>
      <Heading tone={tone}>{view.title}</Heading>
      <Sub>{line}</Sub>

      <p
        className="mt-2 text-center font-mono text-[11.5px] leading-snug"
        style={{ color: tone }}
        aria-live="polite"
      >
        {!done && !view.failed && <span className="mr-1 animate-pulse">▸</span>}
        {view.activity}
      </p>

      <div className="mt-3 w-full border-t border-line pt-2">
        {view.sendAmountSats !== undefined && (
          <Row label="Sent" value={`${view.sendAmountSats.toLocaleString()} sats`} />
        )}
        {view.paymentAmountSats !== undefined ? (
          <Row label="Receiver gets" value={`${view.paymentAmountSats.toLocaleString()} sats`} />
        ) : (
          view.receiveAmountSats !== undefined && (
            <Row label="Receiving" value={`${view.receiveAmountSats.toLocaleString()} sats`} />
          )
        )}
        {view.routerFeeSats !== undefined && (
          <Row label="Router fees" value={`${view.routerFeeSats.toLocaleString()} sats`} />
        )}
        {view.miningFeeSats !== undefined && (
          <Row label="Mining fees" value={`${view.miningFeeSats.toLocaleString()} sats`} />
        )}
      </div>

      <Sub>Hover a router or a leg for detail</Sub>
    </>
  );
}

function NodeBody({ hop, view }: { hop: HopView; view: CircuitView }) {
  // The per-maker protocol flags are deliberately not listed. There are eight of them under
  // taproot and fourteen under legacy, the panel grew tall enough to sit over the router labels
  // inside the ring, and the count could not move honestly anyway: taproot writes all five of a
  // maker's exchange flags in one go after its confirmation wait, so "1 of 8" stood still for
  // the longest stretch of the hop. The stage line above says the same thing in one phrase.
  return (
    <>
      <Heading tone={ROUTER_STROKE[hop.tone]}>{hop.label}</Heading>
      <Sub>
        hop {hop.index + 1} of {view.routerCount + 1} · {STAGE_LABEL[hop.stage]}
      </Sub>
      <span className="mt-1 max-w-full text-center font-mono text-[9px] text-muted">
        {routerName(hop.address)}
      </span>
      {hop.fee && (
        <div className="mt-1 w-full border-t border-line pt-2">
          <Row label="Fee" value={`${hop.fee.estimatedFeeSats.toLocaleString()} sats`} />
          <Row label="Locktime" value={`${hop.fee.locktime} blocks`} />
          <Row
            label="Offer"
            value={`${hop.fee.baseFee} + ${hop.fee.amountRelativeFeePct}% + ${hop.fee.timeRelativeFeePct}%`}
          />
        </div>
      )}
    </>
  );
}

function EdgeBody({ edge, view }: { edge: EdgeView; view: CircuitView }) {
  const from = edge.index === 0 ? "Your wallet" : `Router ${edge.index}`;
  const to = edge.index === view.routerCount ? "Your wallet" : `Router ${edge.index + 1}`;
  return (
    <>
      <Heading tone={STROKE[edge.tone]}>Contract</Heading>
      <Sub>
        {from} → {to}
      </Sub>
      <div className="mt-2 w-full border-t border-line pt-2">
        {edge.amountSats !== undefined && (
          <Row label="Amount" value={`${edge.amountSats.toLocaleString()} sats`} />
        )}
        <Row label="Status" value={EDGE_STAGE_LABEL[edge.stage]} />
        {edge.contractCount > 1 && <Row label="Splits" value={`${edge.contractCount}`} />}
        {edge.locktimeBlocks !== undefined && (
          <Row label="Refund after" value={`${edge.locktimeBlocks} blocks`} />
        )}
      </div>
      {edge.txids.length > 0 ? (
        // Pointer events are off for the readout as a whole so the ring underneath stays
        // hoverable; the links are the one part that has to take a click.
        <div className="mt-1.5 flex w-full flex-col items-center gap-0.5" style={{ pointerEvents: "auto" }}>
          {edge.txids.map((txid) => (
            <a
              key={txid}
              href={explorerTxUrl(txid)}
              target="_blank"
              rel="noreferrer"
              className="max-w-full break-all text-center font-mono text-[9px] text-primary underline decoration-dotted"
            >
              {txid}
            </a>
          ))}
        </div>
      ) : (
        <Sub>
          {edge.contractCount > 1
            ? "Transaction ids appear once the swap records them"
            : "Transaction id appears once the swap records it"}
        </Sub>
      )}
    </>
  );
}

function WalletBody({ view }: { view: CircuitView }) {
  const net =
    view.sendAmountSats !== undefined && view.receiveAmountSats !== undefined
      ? view.receiveAmountSats - view.sendAmountSats
      : undefined;
  return (
    <>
      <Heading tone="var(--color-primary)">Your Wallet</Heading>
      <Sub>Funds leave the out-port and return to the in-port of this same wallet</Sub>
      <div className="mt-2 w-full border-t border-line pt-2">
        {view.sendAmountSats !== undefined && (
          <Row label="Out" value={`${view.sendAmountSats.toLocaleString()} sats`} />
        )}
        {view.receiveAmountSats !== undefined && (
          <Row label="In" value={`${view.receiveAmountSats.toLocaleString()} sats`} />
        )}
        {view.totalFeeSats !== undefined && (
          <Row label="Fees" value={`${view.totalFeeSats.toLocaleString()} sats`} />
        )}
        {net !== undefined && <Row label="Net" value={`${net.toLocaleString()} sats`} />}
      </div>
    </>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3 py-0.5">
      <span className="font-mono text-[9px] uppercase tracking-wide text-subtle">{label}</span>
      <span className="font-numeric text-[10px] text-foreground">{value}</span>
    </div>
  );
}
