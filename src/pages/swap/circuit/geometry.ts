/**
 * Circuit geometry — pure math, no React, so the analytic stroke lengths below can be unit
 * tested without a DOM.
 *
 * Every stroke in the circuit is a circle or a circular arc, so its length is closed-form.
 * That matters: Motion's `pathLength` prop measures paths with getTotalLength() and emits
 * absolute-px stroke-dasharray, which WKWebView scales differently from every other engine
 * (motiondivision/motion#3301) — strokes shorten and clip, and the measurement can't run until
 * the node is in the DOM, so the first frame flashes. Feeding an analytic length into a
 * unitless strokeDashoffset avoids both.
 */

type Tier = "A" | "B" | "C";

interface Point {
  x: number;
  y: number;
}

interface CircuitEdgeGeometry {
  index: number;
  d: string;
  /** Analytic arc length. Never call getTotalLength() on this path. */
  length: number;
  /** Midpoint on the arc, for hanging labels off the outside of the ring. */
  label: Point;
  /** Outward unit normal at the midpoint, so labels push away from the centre. */
  normal: Point;
  /** Kept so `edgeStrands` can redraw the same span on a wider or narrower circle. */
  fromAngle: number;
  toAngle: number;
}

interface CircuitSlotGeometry {
  index: number;
  angle: number;
  center: Point;
}

export interface CircuitGeometry {
  size: number;
  radius: number;
  center: Point;
  ringCircumference: number;
  slots: CircuitSlotGeometry[];
  edges: CircuitEdgeGeometry[];
  outPort: Point;
  inPort: Point;
  tier: Tier;
  nodeSize: number;
  walletWidth: number;
  walletHeight: number;
}

const TWO_PI = Math.PI * 2;

function tierFor(routerCount: number): Tier {
  if (routerCount <= 5) return "A";
  if (routerCount <= 12) return "B";
  return "C";
}

function nodeSizeFor(tier: Tier): number {
  return tier === "A" ? 56 : tier === "B" ? 34 : 14;
}

function walletSizeFor(tier: Tier): { width: number; height: number } {
  return tier === "A" ? { width: 132, height: 58 } : { width: 104, height: 46 };
}

function pointOnCircle(center: Point, radius: number, angle: number): Point {
  return {
    x: center.x + radius * Math.cos(angle),
    y: center.y + radius * Math.sin(angle),
  };
}

function arc(a: Point, b: Point, radius: number): string {
  // sweep-flag 1 = clockwise. A true arc of the same circle the nodes sit on, not a bezier
  // bulge — with few nodes a curve reads as a rounded polygon instead of a circle.
  return `M ${a.x.toFixed(2)} ${a.y.toFixed(2)} A ${radius.toFixed(2)} ${radius.toFixed(
    2,
  )} 0 0 1 ${b.x.toFixed(2)} ${b.y.toFixed(2)}`;
}

/**
 * `routerCount` routers plus one wallet, and `routerCount + 1` edges — the circuit closes,
 * because the wallet that funds the first contract is the wallet that receives the last one.
 *
 * The wallet is not just another evenly spaced slot: it is a much wider card, so it gets its own
 * reserved angular sector at 12 o'clock and the routers share what is left. Spacing them all
 * evenly instead makes the wallet card sit on top of its neighbours as soon as the route grows
 * past a handful of hops.
 */
export function buildCircuit(routerCount: number, size: number): CircuitGeometry {
  const tier = tierFor(routerCount);
  const nodeSize = nodeSizeFor(tier);
  const wallet = walletSizeFor(tier);

  // Room a node needs along the ring: the circle itself plus its label, which at tier A is
  // wider than the node.
  const footprint = nodeSize + (tier === "A" ? 74 : tier === "B" ? 16 : 8);
  const arcsAroundRing = routerCount + 1;

  const walletSectorAt = (r: number) =>
    Math.min(TWO_PI * 0.45, 2 * Math.asin(Math.min(0.95, (wallet.width / 2 + 26) / r)));

  // The wallet's angular sector depends on the radius, and the radius needed to keep routers
  // from colliding depends on the sector left over — so solve it by iteration rather than
  // assuming evenly spaced slots, which underestimates and packs the routers together.
  let radius = Math.max(90, size / 2 - nodeSize / 2 - (tier === "A" ? 46 : 22));
  for (let i = 0; i < 8; i += 1) {
    const step = (TWO_PI - walletSectorAt(radius)) / arcsAroundRing;
    const needed = footprint / (2 * Math.sin(step / 2));
    if (needed <= radius + 0.5) break;
    radius = needed;
  }

  const walletSector = walletSectorAt(radius);
  const available = TWO_PI - walletSector;
  const step = available / arcsAroundRing;

  // Centre on the canvas the circuit actually needs, which may be larger than the requested size.
  const canvas = Math.max(size, radius * 2 + nodeSize + (tier === "A" ? 96 : 46));
  const center = { x: canvas / 2, y: canvas / 2 };

  const top = -Math.PI / 2;
  // Angles increase clockwise in SVG (y grows downward), so the out-port is the clockwise side
  // of the wallet and the in-port is the counter-clockwise side. Flow runs clockwise.
  const outAngle = top + walletSector / 2;
  const inAngle = outAngle + available;

  const outPort = pointOnCircle(center, radius, outAngle);
  const inPort = pointOnCircle(center, radius, inAngle);

  const slots: CircuitSlotGeometry[] = [
    { index: 0, angle: top, center: pointOnCircle(center, radius, top) },
    ...Array.from({ length: routerCount }, (_, i) => {
      const angle = outAngle + (i + 1) * step;
      return { index: i + 1, angle, center: pointOnCircle(center, radius, angle) };
    }),
  ];

  const edges: CircuitEdgeGeometry[] = Array.from({ length: arcsAroundRing }, (_, index) => {
    const fromAngle = index === 0 ? outAngle : slots[index].angle;
    const toAngle = index === arcsAroundRing - 1 ? inAngle : slots[index + 1].angle;
    const from = index === 0 ? outPort : slots[index].center;
    const to = index === arcsAroundRing - 1 ? inPort : slots[index + 1].center;
    const midAngle = fromAngle + (toAngle - fromAngle) / 2;

    return {
      index,
      d: arc(from, to, radius),
      length: radius * (toAngle - fromAngle),
      label: pointOnCircle(center, radius, midAngle),
      normal: { x: Math.cos(midAngle), y: Math.sin(midAngle) },
      fromAngle,
      toAngle,
    };
  });

  return {
    size: canvas,
    radius,
    center,
    ringCircumference: TWO_PI * radius,
    slots,
    edges,
    outPort,
    inPort,
    tier,
    nodeSize,
    walletWidth: wallet.width,
    walletHeight: wallet.height,
  };
}

/** Gap between two strands of a split leg, in px of radius. */
const STRAND_GAP = 5;

/**
 * One arc per contract transaction on a leg, drawn as concentric strands centred on the ring.
 * A hop funded by two splits is two transactions, and one stroke would say it was one.
 *
 * Each strand is a true arc of its own circle rather than an offset copy of the base path, so
 * its length stays analytic — see this file's header for why that matters in WKWebView.
 */
export function edgeStrands(
  geo: CircuitGeometry,
  edge: CircuitEdgeGeometry,
  count: number,
): { d: string; length: number }[] {
  const strands = Math.max(1, count);
  return Array.from({ length: strands }, (_, i) => {
    const radius = geo.radius + (i - (strands - 1) / 2) * STRAND_GAP;
    return {
      d: arc(
        pointOnCircle(geo.center, radius, edge.fromAngle),
        pointOnCircle(geo.center, radius, edge.toAngle),
        radius,
      ),
      length: radius * (edge.toAngle - edge.fromAngle),
    };
  });
}

/** Where a label should sit so it clears the ring rather than overlapping the stroke. */
export function labelAnchor(edge: CircuitEdgeGeometry, distance: number): Point {
  return {
    x: edge.label.x + edge.normal.x * distance,
    y: edge.label.y + edge.normal.y * distance,
  };
}
