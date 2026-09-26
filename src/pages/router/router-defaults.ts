/** Bitcoin targets a block every 10 minutes, so 144 a day. Approximate by construction. */
export function timelockDays(blocks: number): number {
  return Math.round(blocks / 144);
}

// Mirrors the backend's `valid_id`, so a rejected name is caught before the round trip.
export const ROUTER_ID_PATTERN = /^[A-Za-z0-9_-]+$/;
