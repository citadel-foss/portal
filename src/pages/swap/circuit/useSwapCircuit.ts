import { useMemo } from "react";

import type {
  RouterFeeInfo,
  RouterStage,
  SwapSummary,
  SwapTrackerProgress,
  TrackerPhase,
} from "../../../api/types";

/** Six acts, one headline each. Acts before `route` have nothing on-chain — see `committed`. */
export type Act = "find" | "deal" | "fund" | "route" | "finalizing" | "settle";

export const ACT_LABEL: Record<Act, string> = {
  find: "Find",
  deal: "Deal",
  fund: "Fund",
  route: "Route",
  finalizing: "Finalizing",
  settle: "Settle",
};

/**
 * The per-router protocol stages, as prose. `STAGE_DOING` narrates the live hop; `STAGE_LABEL`
 * is the word under a router on the ring.
 */
const STAGE_DOING: Record<RouterStage, string> = {
  waiting: "Waiting its turn on the route",
  negotiated: "Terms agreed, waiting its turn",
  handshaking: "Handshaking over Tor and exchanging contracts",
  confirming: "Waiting for its contract to confirm on-chain",
  routed: "Contract confirmed, waiting for its private key",
  key_received: "Its key is yours, forwarding it to the next hop",
  settled: "Key forwarded to the next hop",
};

export const STAGE_LABEL: Record<RouterStage, string> = {
  waiting: "Waiting",
  negotiated: "Negotiated",
  handshaking: "Handshaking",
  confirming: "Confirming",
  routed: "Routed",
  key_received: "Key received",
  settled: "Settled",
};

/**
 * Past-tense name for each flag the crate sets, keyed by its field name. Both protocols' flag
 * sets are here — a router carries one set or the other, never both. This is hover-card detail
 * only: the flags are not evenly spaced in time, and taproot writes five of them at once, so
 * the narrative comes from `stage` instead.
 */
/**
 * What the swap is doing at each phase, for the stretches with no router of its own to name:
 * everything before the funds move, and the final sweep after the last key is forwarded.
 * Each phase names the *next* piece of work, since the crate stamps a phase on completing one.
 */
const PHASE_DOING: Record<TrackerPhase, string> = {
  routers_discovered: "Negotiating terms with the routers",
  negotiated: "Building your funding transactions",
  funding_created: "Broadcasting your funding transactions",
  funds_broadcast: "Routing your funds into the first contract",
  contracts_exchanged: "Waiting for the routed contracts to confirm",
  finalizing: "Exchanging private keys around the route",
  privkeys_forwarded: "Waiting for the sweep tx to confirm",
  completed: "Swept to your wallet",
  failed: "Stopped — recovery reclaims anything already on-chain",
};

/** An edge is a contract transaction, so it has its own lifecycle independent of its endpoints. */
export type EdgeStage = "pending" | "built" | "broadcast" | "confirming" | "confirmed";

export const EDGE_STAGE_LABEL: Record<EdgeStage, string> = {
  pending: "Not started",
  built: "Built",
  broadcast: "Waiting confirmations",
  confirming: "Confirming",
  confirmed: "Confirmed",
};

export type Tone = "idle" | "active" | "success" | "danger";

export interface Milestone {
  key: string;
  done: boolean;
}

export interface HopView {
  index: number;
  address: string;
  label: string;
  stage: RouterStage;
  tone: Tone;
  fee?: RouterFeeInfo;
  milestones: Milestone[];
}

export interface EdgeView {
  index: number;
  stage: EdgeStage;
  tone: Tone;
  /** Amount still travelling after this hop's fee is deducted. */
  amountSats?: number;
  /** From `RouterFeeInfo.locktime` — the refund window on this contract, in blocks. */
  locktimeBlocks?: number;
  /** Contract transactions on this leg. A hop may be funded by several splits, not one tx. */
  contractCount: number;
  /** Recorded contract txids, in the order the crate wrote them; empty until it does. */
  txids: string[];
}

export interface CircuitView {
  routerCount: number;
  act: Act;
  /** True once funds are on-chain. Before this the swap can still be abandoned. */
  committed: boolean;
  /**
   * Asserted by the page, not inferred from the route. "Every router settled" comes well before
   * the swap is over — the crate still has to sweep the incoming contract and write the report —
   * and the tracker's own `completed` lands before that report exists. Taking it from the page
   * keeps the diagram, the heading and the View Report button changing at one moment.
   */
  complete: boolean;
  failed: boolean;
  failureReason?: string;
  hops: HopView[];
  edges: EdgeView[];
  /** Index of the hop currently doing something, or null when nothing is live. */
  focusIndex: number | null;
  /** The contract the swap is blocked on: the first leg that isn't confirmed. */
  liveEdgeIndex: number | null;
  /** One sentence: what the swap is actually doing right now. */
  activity: string;
  /**
   * The headline: the act by default, and `Hop N Confirming` while a contract is on the chain
   * waiting. The status strip under the circuit shows this same string, so the two can never
   * name different stages of the same swap.
   */
  title: string;
  /** How much longer the live leg has: a block while a tx is confirming, otherwise unhedged. */
  etaLabel: string;
  hopsConfirmed: number;
  sendAmountSats?: number;
  receiveAmountSats?: number;
  totalFeeSats?: number;
  /** `totalFeeSats` split the way the summary panel splits it, so the two agree. */
  routerFeeSats?: number;
  miningFeeSats?: number;
  /** Set when this swap pays a third party rather than returning the coins to the wallet. */
  paymentAddress?: string;
  paymentAmountSats?: number;
}

const PHASE_ACT: Record<TrackerPhase, Act> = {
  routers_discovered: "find",
  negotiated: "deal",
  funding_created: "fund",
  funds_broadcast: "route",
  contracts_exchanged: "route",
  finalizing: "finalizing",
  privkeys_forwarded: "finalizing",
  completed: "settle",
  failed: "route",
};

const COMMITTED_PHASES: TrackerPhase[] = [
  "funds_broadcast",
  "contracts_exchanged",
  "finalizing",
  "privkeys_forwarded",
  "completed",
];

/** Stage order, for the "has this hop got at least this far" comparisons below. */
const STAGE_ORDER: RouterStage[] = [
  "waiting",
  "negotiated",
  "handshaking",
  "confirming",
  "routed",
  "key_received",
  "settled",
];

const atLeast = (stage: RouterStage, min: RouterStage) =>
  STAGE_ORDER.indexOf(stage) >= STAGE_ORDER.indexOf(min);

export function useSwapCircuit(
  tracker: SwapTrackerProgress | null,
  summary: SwapSummary | null,
  failure: boolean,
  finished = false,
): CircuitView {
  return useMemo<CircuitView>(() => {
    const routerCount = summary?.routers.length ?? tracker?.routerCount ?? 2;
    const phase: TrackerPhase = tracker?.phase ?? "routers_discovered";
    const failed = failure || phase === "failed";
    const act = PHASE_ACT[phase];
    const committed = COMMITTED_PHASES.includes(phase);

    const hops: HopView[] = Array.from({ length: routerCount }, (_, index) => {
      const fee = summary?.routers[index];
      const address = fee?.address ?? tracker?.routers[index]?.address ?? `router-${index}`;
      const live = tracker?.routers.find((r) => r.address === address) ?? tracker?.routers[index];
      return {
        index,
        address,
        label: `Router ${index + 1}`,
        stage: live?.stage ?? "waiting",
        tone: "idle" as Tone,
        fee,
        milestones: (live?.milestones ?? []).map((m) => ({
          key: m.key,
          done: m.done,
        })),
      };
    });

    // Routing walks the route once to get every contract confirmed; the key exchange then walks
    // it again to settle each hop. So which hop is live depends on which pass the swap is on —
    // during the second one, hops sitting at `routed` are still waiting their turn.
    const focusIndex = (() => {
      if (phase === "completed") return null;
      const target: RouterStage = act === "finalizing" || act === "settle" ? "settled" : "routed";
      const idx = hops.findIndex((h) => !atLeast(h.stage, target));
      return idx === -1 ? null : idx;
    })();

    hops.forEach((hop) => {
      hop.tone =
        failed && hop.index === focusIndex
          ? "danger"
          : hop.index === focusIndex
            ? "active"
            : atLeast(hop.stage, "routed")
              ? "success"
              : "idle";
    });

    // Which contract txs each leg carries. Only the two legs touching the wallet are recorded
    // per-leg; everything between two routers arrives as one flat list, so it can only be
    // attributed when it divides evenly across those legs. Until a leg's txids are recorded it
    // has none, and one strand stands in for the unknown.
    const betweenRouters = Math.max(0, routerCount - 1);
    const watchonly = tracker?.watchonlyContractTxids ?? [];
    const perIntermediate =
      betweenRouters > 0 && watchonly.length % betweenRouters === 0
        ? watchonly.length / betweenRouters
        : 0;
    const txidsFor = (index: number): string[] => {
      if (index === 0) return tracker?.outgoingContractTxids ?? [];
      if (index === routerCount) return tracker?.incomingContractTxids ?? [];
      if (perIntermediate === 0) return [];
      // Leg 1 is the first router-to-router hop, so it starts at the front of the flat list.
      return watchonly.slice((index - 1) * perIntermediate, index * perIntermediate);
    };

    // Edge k carries the value leaving slot k. Amounts descend as each router takes its fee.
    let running = summary?.sendAmountSats ?? tracker?.sendAmountSats;
    const edges: EdgeView[] = Array.from({ length: routerCount + 1 }, (_, index) => {
      const amountSats = running;
      const fee = summary?.routers[index];
      if (running !== undefined && fee) running = running - fee.estimatedFeeSats;

      let stage: EdgeStage = "pending";
      if (index === 0) {
        // Your own funding transaction. Legacy reports its confirmation directly; taproot never
        // does, but a maker won't fund its own contract until ours has confirmed, so its hop
        // reaching `routed` says so after the fact.
        const ours = hops[0]?.milestones.find((m) => m.key === "prev_funding_confirmed");
        if (ours?.done || (hops[0] && atLeast(hops[0].stage, "routed"))) stage = "confirmed";
        else if (committed) stage = "broadcast";
        else if (phase === "funding_created") stage = "built";
      } else {
        // Edge k is the contract router k funds, so it tracks that router's own stage.
        const source = hops[index - 1];
        if (source && atLeast(source.stage, "routed")) stage = "confirmed";
        else if (source?.stage === "confirming") stage = "confirming";
        else if (source?.stage === "handshaking") stage = "broadcast";
      }
      if (phase === "completed") stage = "confirmed";

      const tone: Tone =
        stage === "confirmed" ? "success" : stage === "pending" ? "idle" : "active";

      const txids = txidsFor(index);
      return {
        index,
        stage,
        tone,
        amountSats,
        locktimeBlocks: index === 0 ? undefined : summary?.routers[index - 1]?.locktime,
        contractCount: Math.max(1, txids.length),
        txids,
      };
    });

    // Router k funds edge k+1, so a hop index is never an edge index. Asking which leg is
    // unconfirmed answers "what is the swap waiting on" without that off-by-one.
    const liveEdge = edges.findIndex((e) => e.stage !== "confirmed");

    // The route is funded strictly one leg at a time, so only the live leg may be lit. Without
    // this, hop 1 showed two: the tracker marks router 1 `confirming` as soon as the route starts
    // moving, which lights its outgoing leg while our own funding leg is still broadcasting.
    if (liveEdge !== -1) {
      for (const edge of edges.slice(liveEdge + 1)) {
        edge.stage = "pending";
        edge.tone = "idle";
      }
    }
    if (failed && liveEdge !== -1) edges[liveEdge].tone = "danger";

    // Is a contract sitting on the chain waiting? That is the one state with a block-shaped
    // answer to "how much longer", and the one the headline names by hop.
    const liveStage = liveEdge === -1 ? null : edges[liveEdge].stage;
    const onChainWait = liveStage === "broadcast" || liveStage === "confirming";

    // A router that hasn't been reached yet isn't the story — the phase is. Once the swap is
    // working on one, that router's stage is the specific thing to say.
    const focusHop = focusIndex === null ? null : hops[focusIndex];
    const activity =
      failed || focusHop === null || focusHop.stage === "waiting"
        ? PHASE_DOING[failed ? "failed" : phase]
        : // Leg 0 is our own funding, not a router's contract. The first router reads as
          // `confirming` throughout that wait, so naming it here credits our transaction to it.
          liveEdge === 0 && onChainWait
          ? "Your wallet · Waiting for its contract to confirm on-chain"
          : `${focusHop.label} · ${STAGE_DOING[focusHop.stage]}`;

    const title = failed
      ? "Swap Failed"
      : finished
        ? "Swap Complete"
        : onChainWait
          ? `Hop ${liveEdge + 1} Confirming`
          : ACT_LABEL[act];

    return {
      routerCount,
      act,
      committed,
      complete: finished,
      failed,
      failureReason: tracker?.failureReason,
      hops,
      edges,
      focusIndex,
      liveEdgeIndex: liveEdge === -1 ? null : liveEdge,
      activity,
      title,
      // A confirmation wait is the only stretch with a real unit; everything else is protocol
      // chatter measured in seconds, and a countdown on it would be invented.
      etaLabel: failed || finished ? "—" : onChainWait ? "10 mins" : "Soon",
      hopsConfirmed: edges.filter((e) => e.stage === "confirmed").length,
      sendAmountSats: summary?.sendAmountSats ?? tracker?.sendAmountSats,
      receiveAmountSats: summary?.estimatedReceiveAmountSats,
      totalFeeSats: summary?.totalEstimatedFeeSats,
      routerFeeSats: summary?.routerFeeSats,
      miningFeeSats: summary?.miningFeeSats,
      paymentAddress: summary?.payment?.address ?? tracker?.paymentAddress,
      paymentAmountSats: summary?.payment?.amountSats ?? tracker?.paymentAmountSats,
    };
  }, [tracker, summary, failure, finished]);
}
