/** One bitcoin, in satoshis. */
export const SATS_PER_BTC = 100_000_000;

/**
 * Digit grouping for every number the app prints, pinned rather than taken from the host.
 *
 * A bare `toLocaleString()` follows the machine's locale, and on an en-IN one that is lakh
 * grouping: 212,238 sats renders as "2,12,238". Bitcoin amounts are read in thousands the
 * world over, and a swap figure that regroups itself depending on whose laptop it is on is
 * unreadable rather than localized. Dates are not covered by this — those stay local.
 */
export function formatEnUsNumber(value: number, maximumFractionDigits = 0): string {
  return value.toLocaleString("en-US", { maximumFractionDigits });
}

/** ₿ immediately followed by the sats amount, grouped every three digits. The ₿ means satoshis. */
export function formatBalanceSats(sats: number): string {
  return `₿${formatEnUsNumber(Math.round(sats))}`;
}

/**
 * Dollar equivalent of `sats` at `btcPriceUsd` per bitcoin.
 * Always two fractional digits. Null when there is no usable quote, so a balance
 * can still render without inventing a price.
 */
export function formatBalanceUsd(sats: number, btcPriceUsd: number | null): string | null {
  if (btcPriceUsd === null || !Number.isFinite(btcPriceUsd) || btcPriceUsd <= 0) return null;
  if (!Number.isFinite(sats)) return null;
  const usd = (Math.round(sats) / SATS_PER_BTC) * btcPriceUsd;
  if (!Number.isFinite(usd)) return null;
  return usd.toLocaleString("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
}
