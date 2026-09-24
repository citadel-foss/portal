// Ported from taker-app/src/js/coinswapHelpers.js (formatTorEndpoint,
// estimateRouterFee) so the Market page matches the old app's real fee math
// and address display exactly.

import { truncateMiddle } from "./wallet-format";

/** The bare host, scheme and port stripped — the part that actually names a router. */
export function torEndpointHost(value: string, stripOnion = false): string {
  const text = (value ?? "").trim();
  if (!text) return "unknown";
  const noScheme = text.replace(/^https?:\/\//i, "").replace(/^tcp:\/\//i, "").split("/")[0];
  const separatorIndex = noScheme.lastIndexOf(":");
  const host = separatorIndex !== -1 ? noScheme.slice(0, separatorIndex) : noScheme;
  return stripOnion ? host.replace(/\.onion$/i, "") : host;
}

/** Shortened to a first and last few characters, with the middle elided. */
export function formatTorEndpoint(value: string, start = 12, end = 8, stripOnion = false): string {
  return truncateMiddle(torEndpointHost(value, stripOnion), start, end);
}

/**
 * A router's name, as every list, row, label and hover card in the app shows it.
 *
 * Unlike a txid or a bitcoin address, nobody needs to read an onion host end to end: it is 56
 * characters of base32 that no flow here asks anyone to type or paste — routers arrive over
 * Nostr and are acted on by row — and at full length it crowds out the fees and bonds beside
 * it. The head and tail identify one uniquely among the handful a route ever involves.
 */
export function routerName(address: string): string {
  return formatTorEndpoint(address, 12, 8, true);
}

export interface RouterFeeEstimate {
  baseFee: number;
  liquidityFee: number;
  timeFee: number;
  totalFee: number;
  refundLocktime: number;
}

// totalFee = baseFee + amount*volumeRate + refundLocktime*amount*timeRate.
// refundLocktime = 20 * (totalRouters - position + 1).
export function estimateRouterFee(opts: {
  baseFee: number;
  amountRelativeFeePct: number;
  timeRelativeFeePct: number;
  amountSats: number;
  routerPosition: number;
  totalRouters: number;
}): RouterFeeEstimate {
  const refundLocktime = 20 * (opts.totalRouters - opts.routerPosition + 1);
  const liquidityFee = opts.amountSats * (opts.amountRelativeFeePct / 100);
  const timeFee = refundLocktime * opts.amountSats * (opts.timeRelativeFeePct / 100);
  return {
    baseFee: opts.baseFee,
    liquidityFee,
    timeFee,
    totalFee: opts.baseFee + liquidityFee + timeFee,
    refundLocktime,
  };
}

/** Total router fees for a whole route, in sats. Mirrors Taker::prepare_swap: each hop
 * prices the amount remaining after the previous hop, and each individual router fee is
 * rounded up to sats. */
export function estimateRouteRouterFees(
  routers: { baseFee: number; amountRelativeFeePct: number; timeRelativeFeePct: number }[],
  amountSats: number,
): number {
  let remaining = amountSats;
  let totalFeeSats = 0;
  for (let i = 0; i < routers.length; i += 1) {
    const router = routers[i];
    const estimate = estimateRouterFee({
      ...router,
      amountSats: remaining,
      routerPosition: i + 1,
      totalRouters: routers.length,
    });
    totalFeeSats += Math.ceil(estimate.totalFee);
    // The crate carries the unrounded f64 amount into the next hop.
    remaining = Math.max(0, remaining - estimate.totalFee);
  }
  return totalFeeSats;
}
