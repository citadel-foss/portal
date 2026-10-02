import { useCallback, useEffect, useState } from "react";
import { estimateFees } from "../api/commands";
import type { FeeEstimate } from "../api/types";

export type FeeTier = "fast" | "medium" | "slow";

/** Portal's typo guard, the same one the backend applies to every swap and send. */
export const MAX_FEE_RATE = 500;
export type FeeChoice = FeeTier | "custom";

export const FEE_TIERS: { key: FeeTier; label: string }[] = [
  { key: "slow", label: "Slow" },
  { key: "medium", label: "Medium" },
  { key: "fast", label: "Fast" },
];

/** The session's chain server's estimate for each tier. */
export function useFeeEstimate() {
  const [fees, setFees] = useState<FeeEstimate | null>(null);
  const [failed, setFailed] = useState(false);
  const load = useCallback(() => {
    setFailed(false);
    void estimateFees()
      .then(setFees)
      .catch(() => {
        setFees(null);
        setFailed(true);
      });
  }, []);
  useEffect(load, [load]);
  return { fees, failed, retry: load };
}

/** 0 when the choice has no usable rate yet; callers treat that as not ready. Fee rates are
 *  always rounded up to whole sats/vB so Portal never pays below the selected estimate. */
export function chosenFeeRate(
  fees: FeeEstimate | null,
  choice: FeeChoice,
  custom: string,
): number {
  if (choice === "custom") {
    const text = custom.trim();
    const rate = Number(text);
    return text !== "" &&
      Number.isFinite(rate) &&
      rate >= 1 &&
      rate <= MAX_FEE_RATE
      ? Math.ceil(rate)
      : 0;
  }
  const rate = fees?.[choice];
  if (rate == null || !Number.isFinite(rate) || rate < 1 || rate > MAX_FEE_RATE) return 0;
  return Math.max(1, Math.ceil(rate));
}
