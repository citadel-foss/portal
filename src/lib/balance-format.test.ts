import assert from "node:assert/strict";
import { test } from "node:test";
import { SATS_PER_BTC, formatBalanceSats, formatBalanceUsd } from "./balance-format.ts";

test("sats use a ₿ prefix and groups of three digits", () => {
  assert.equal(formatBalanceSats(472_850), "₿472,850");
  assert.equal(formatBalanceSats(0), "₿0");
  assert.equal(formatBalanceSats(999), "₿999");
  assert.equal(formatBalanceSats(1_000), "₿1,000");
  assert.equal(formatBalanceSats(1_000_000), "₿1,000,000");
  // en-IN would print this as 2,12,238. Amounts stay in thousands everywhere.
  assert.equal(formatBalanceSats(212_238), "₿212,238");
});

test("fractional sats round to the nearest sat before grouping", () => {
  assert.equal(formatBalanceSats(472_850.4), "₿472,850");
  assert.equal(formatBalanceSats(472_850.5), "₿472,851");
});

test("dollar values keep two fractional digits", () => {
  assert.equal(formatBalanceUsd(SATS_PER_BTC, 49_818.2), "$49,818.20");
  assert.equal(formatBalanceUsd(SATS_PER_BTC, 1.2), "$1.20");
  assert.equal(formatBalanceUsd(SATS_PER_BTC / 2, 60_000), "$30,000.00");
  assert.equal(formatBalanceUsd(0, 50_000), "$0.00");
  // A single sat at a six-figure bitcoin price is under one cent.
  assert.equal(formatBalanceUsd(1, 100_000), "$0.00");
});

test("dollar value is omitted when the quote cannot be used", () => {
  assert.equal(formatBalanceUsd(472_850, null), null);
  assert.equal(formatBalanceUsd(472_850, 0), null);
  assert.equal(formatBalanceUsd(472_850, -1), null);
  assert.equal(formatBalanceUsd(472_850, Number.NaN), null);
  assert.equal(formatBalanceUsd(Number.NaN, 50_000), null);
});
