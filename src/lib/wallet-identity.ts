// A stable visual identity per wallet name, so a folder of wallets reads as distinct objects
// rather than a row of identical cards. Derived purely from the name — nothing is persisted, and
// the same wallet looks the same on every machine.

/** FNV-1a, 32-bit. `Math.imul` keeps the multiply in int32 instead of losing precision. */
function hash32(value: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < value.length; i++) {
    h ^= value.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

// A curated set rather than the full wheel: a free-running hue produces browns and puces that
// look like a bug against this background. Every entry here still reads as part of the palette.
const HUES = [222, 262, 190, 152, 42, 12, 288, 96];

interface WalletIdentity {
  /** Foreground ink for the monogram. */
  ink: string;
  /** Translucent fill behind it. */
  fill: string;
  /** Border tint. */
  edge: string;
  /** Ambient glow, for the larger unlock emblem. */
  glow: string;
  /** 2-3 characters standing in for the name. */
  monogram: string;
}

/**
 * Lightness and chroma are fixed while only hue varies: OKLCH is perceptually uniform, so every
 * wallet's accent carries the same visual weight. Chroma stays at 0.13 because past ~0.18 the
 * blues and greens clip out of sRGB and go flat.
 */
export function walletIdentity(name: string): WalletIdentity {
  const hue = HUES[hash32(name) % HUES.length];
  return {
    ink: `oklch(0.80 0.13 ${hue})`,
    fill: `oklch(0.62 0.15 ${hue} / 0.14)`,
    edge: `oklch(0.70 0.14 ${hue} / 0.42)`,
    glow: `oklch(0.62 0.15 ${hue} / 0.30)`,
    monogram: monogram(name),
  };
}

/**
 * Initials across name segments, so `cold-storage` reads "CS" rather than "CO" — the leading
 * characters of generated names (`taker-wallet-…`) are all identical, which is exactly the case
 * a monogram has to disambiguate.
 */
function monogram(name: string): string {
  const parts = name.split(/[^A-Za-z0-9]+/).filter(Boolean);
  if (parts.length >= 2) {
    return (parts[0][0] + parts[1][0]).toUpperCase();
  }
  return (parts[0] ?? name).slice(0, 2).toUpperCase() || "??";
}
