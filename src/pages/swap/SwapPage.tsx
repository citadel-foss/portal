import { subscribe } from "../../api/transport";
import { UnresolvedPayments } from "../../components/app/UnresolvedPayments";
import { spendingBlocked, useUnresolvedStore } from "../../store/unresolved";
import {
  AlertTriangle,
  ArrowLeftRight,
  CheckCircle2,
  FileText,
  LifeBuoy,
  RefreshCw,
  ShieldAlert,
  XCircle,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import {
  estimateSwapFunding,
  getBtcPrice,
  getLogs,
  getOffers,
  getSwapPreparation,
  getRecoveryStatus,
  getSwapProgress,
  getSwapTracker,
  cancelSwap,
  startSwap,
} from "../../api/commands";
import { isAppError } from "../../api/types";
import { chosenFeeRate, type FeeChoice, useFeeEstimate } from "../../lib/fee-rate";
import { useSwapPrepare } from "../../store/swap-prepare";
import type {
  SwapPreparation,
  AppError,
  LogLine,
  Router,
  Outpoint,
  ProtocolVersion,
  SwapLiquidity,
  SwapFundingEstimate,
  SwapRequest,
  SwapSummary,
  SwapTrackerProgress,
  UtxoEntry,
} from "../../api/types";
import {
  AmountTile,
  Card,
  Disclosure,
  Identifier,
  LogViewer,
  Notice,
  SatsAmount,
} from "../../components/ui/display";
import {
  Button,
  FeeRateField,
  PresetTile,
  SegmentedToggle,
  TextField,
} from "../../components/ui/inputs";
import { Checklist } from "../../components/ui/Checklist";
import { SwapCircuit } from "./circuit/SwapCircuit";
import { NowPanel, Vitals } from "./circuit/panels";
import { useSwapCircuit } from "./circuit/useSwapCircuit";
import {
  estimateRouteRouterFees,
  routerName,
} from "../../lib/market-format";
import { classifySpendType, formatDuration, formatNumber, formatUnitAmount, SATS_PER_BTC, type Unit, useUnitAmount } from "../../lib/wallet-format";
import { RECOVERY_UI_ENABLED, useRecoveryStore } from "../../store/recovery";
import { useToastStore } from "../../store/toast";
import { useWalletCacheStore } from "../../store/wallet-cache";

type UtxoFilter = "regular" | "swap";
type Lifecycle = "configure" | "running" | "finished" | "failed";
function EstimatedSats({
  sats,
  className,
}: {
  sats: number | null;
  className: string;
}) {
  return sats === null ? (
    <strong className="font-mono text-subtle">—</strong>
  ) : (
    <span className="inline-flex items-baseline gap-1">
      <span className="font-mono text-subtle" aria-label="approximately">≈</span>
      <SatsAmount sats={sats} className={className} />
    </span>
  );
}

// A blocking prepare takes tens of seconds; this only reads a local file.
const PREPARATION_POLL_MS = 1_200;

const ROUTER_COUNT_PRESETS = [1, 2, 3] as const;
/** The Custom tile's own value; its count comes from the text field. */
const CUSTOM_ROUTER_COUNT = 4;

/** `MAX_TX_COUNT`; the backend rejects anything outside 1..=10. */
const DEFAULT_TX_COUNT = 1;
const MAX_TX_COUNT = 10;


const FUNDING_RETRY_DELAYS_MS = [2_000, 5_000, 12_000];

// Routers drop a negotiated swap left idle for 15 minutes (the crate's IDLE_CONNECTION_TIMEOUT);
// starting one after that fails part-way through, so the review stops offering it first.
const REVIEW_TTL_MS = 10 * 60_000;

function elapsedLabel(startedAt: number | null): string {
  if (!startedAt) return "0s";
  return formatDuration(Date.now() / 1000 - startedAt);
}

// Owns its own 1s tick so the rest of the progress screen (circuit diagram, Motion props
// and all) doesn't re-render every second just to update this one label.
function Elapsed({
  startedAt,
  active,
}: {
  startedAt: number | null;
  active: boolean;
}) {
  const [, tick] = useState(0);
  useEffect(() => {
    if (!active) return;
    const id = setInterval(() => tick((t) => t + 1), 1000);
    return () => clearInterval(id);
  }, [active]);
  return <>{elapsedLabel(startedAt)}</>;
}

export function SwapPage() {
  const navigate = useNavigate();
  const pushToast = useToastStore((s) => s.push);
  const walletSyncStatus = useWalletCacheStore((s) => s.syncStatus);
  const walletSyncError = useWalletCacheStore((s) => s.syncError);
  const balances = useWalletCacheStore((s) => s.balances);
  const utxos = useWalletCacheStore((s) => s.utxos);

  const liquidity = useMemo<SwapLiquidity | null>(() => {
    if (!balances) return null;
    return {
      spendable: balances.spendable,
      regular: balances.regular,
      swap: balances.swap,
      // One swap is funded from one kind of coin, never both together.
      maxSwappable: Math.max(balances.regular, balances.swap),
    };
  }, [balances]);
  const [btcPrice, setBtcPrice] = useState<number | null>(null);
  const [btcPriceCached, setBtcPriceCached] = useState(false);
  const {
    unit,
    input: amountInput,
    setInput: setAmountInput,
    changeUnit,
    sats: amountSats,
  } = useUnitAmount(btcPrice);
  const [routers, setRouters] = useState<Router[]>([]);
  const [fundingEstimate, setFundingEstimate] =
    useState<SwapFundingEstimate | null>(null);
  const [fundingEstimateStatus, setFundingEstimateStatus] = useState<
    "idle" | "loading" | "retrying" | "ready" | "error"
  >("idle");
  // What the funding plan needs and has, when the wallet cannot cover it: waiting never fixes
  // that, so it is shown at once instead of retried.
  const [shortfall, setShortfall] = useState<{ available: number; required: number } | null>(null);
  // Bumped by the summary's Retry to re-arm the quote with a fresh attempt budget.
  const [fundingAttempt, setFundingAttempt] = useState(0);
  // Compared against the quote's own rate rather than a literal 2: the protocol's fixed rate
  // lives in the crate, and hardcoding it here would go stale silently.

  const [utxoFilter, setUtxoFilter] = useState<UtxoFilter>("regular");
  const [selectedOutpoints, setSelectedOutpoints] = useState<Outpoint[]>([]);
  const [protocol, setProtocol] = useState<ProtocolVersion>("taproot");
  const [routerCount, setRouterCount] = useState(2);
  const [customRouterCount, setCustomRouterCount] = useState(String(CUSTOM_ROUTER_COUNT));
  const [selectedRouters, setSelectedRouters] = useState<string[]>([]);
  const [txCount, setTxCount] = useState(DEFAULT_TX_COUNT);
  const [feeKey, setFeeKey] = useState<FeeChoice>("medium");
  const [customFeeRate, setCustomFeeRate] = useState("");
  const { fees, failed: feesFailed, retry: retryFees } = useFeeEstimate();
  const feeRate = chosenFeeRate(fees, feeKey, customFeeRate, true);
  const [destination, setDestination] = useState<"wallet" | "address">("wallet");
  const [paymentAddress, setPaymentAddress] = useState("");

  const [phase, setPhase] = useState<Lifecycle>("configure");
  const [summary, setSummary] = useState<SwapSummary | null>(null);
  const [swapId, setSwapId] = useState<string | null>(null);
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [failure, setFailure] = useState<AppError | null>(null);
  const [tracker, setTracker] = useState<SwapTrackerProgress | null>(null);
  const prepare = useSwapPrepare();
  const preparingSince = prepare.since;
  const submitting = preparingSince !== null;
  const review = prepare.review;
  const [preparation, setPreparation] = useState<SwapPreparation | null>(null);
  const [reviewExpired, setReviewExpired] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [swapLogs, setSwapLogs] = useState<LogLine[]>([]);
  const [logsOpen, setLogsOpen] = useState(false);

  const recoveryActive = useRecoveryStore((s) => s.active);

  const routerNames = useMemo(
    () =>
      new Map(
        routers.flatMap((router) => {
          const name = router.offer?.name.trim();
          return name ? [[router.address, name] as const] : [];
        }),
      ),
    [routers],
  );

  const circuit = useSwapCircuit(
    tracker,
    summary,
    phase === "failed",
    phase === "finished",
    routerNames,
  );

  const loadReference = useCallback(async () => {
    const nextOffers = await getOffers();
    setRouters(nextOffers.good);
  }, []);

  useEffect(() => {
    // A warning, not an error: arriving here before the offerbook has synced is the normal
    // first-launch state, the page still works, and the router list fills in on its own.
    void loadReference().catch((e) => {
      pushToast(
        "warning",
        isAppError(e) ? e.message : "Router list is not available yet.",
      );
    });
    // BTC/USD price is best-effort, same as Send — leave the USD unit disabled rather than toast.
    void getBtcPrice()
      .then((p) => {
        setBtcPrice(p.usd);
        setBtcPriceCached(p.cached);
      })
      .catch(() => {
        setBtcPrice(null);
        setBtcPriceCached(false);
      });

    // Reconcile a swap already in flight (app restart mid-swap, or navigating back here). Only a
    // running swap is ever reported: a terminal phase is stale by definition, and recovery lives
    // on its own page, so an answer here means "still running" and nothing else.
    void getSwapProgress().then((progress) => {
      if (!progress) return;
      setSwapId(progress.swapId);
      setStartedAt(progress.startedAt ?? null);
      // Router fees live only in the prepared quote, so without replaying it the circuit's
      // per-hop amounts silently stop descending after a remount.
      setSummary(progress.summary);
      setPhase("running");
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The promises are what cleanup awaits, never a variable each one assigns on resolving: a
  // remount before `subscribe` resolves would find that variable still unset, leak the first
  // listener and leave every event handled twice. StrictMode remounts exactly that fast.
  useEffect(() => {
    const unlisteners = [
      subscribe<string>("swap://finished", () => setPhase("finished")),
      // Only fires for a failure with nothing on-chain, which is a plain error with nothing to
      // recover. Anything past the funding broadcast arrives as swap://recovering instead.
      subscribe<AppError>("swap://failed", (e) => {
        setFailure(e);
        setPhase("failed");
      }),
      subscribe("swap://recovering", () => handOffToRecovery()),
    ];
    return () => {
      void Promise.all(unlisteners).then((fns) => fns.forEach((fn) => fn()));
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Live per-router detail straight off swap_tracker.cbor — same 2s poll cadence the old Electron
  // app used for its disk-read poll of the same file.
  //
  // The same poll is what ends the swap. `swap://finished` is a notification, not the source of
  // truth, and a browser that misses one — a dropped SSE stream, a session that idled out during
  // a swap that runs for half an hour — would otherwise sit on a running screen forever with the
  // timer still counting. The active slot emptying is `start_swap` having returned, report
  // written and all, so it is the one signal that cannot be missed.
  useEffect(() => {
    if (phase !== "running") return;
    let cancelled = false;
    // A slow answer must not stack a new request on it every tick: the browser allows only a
    // handful of connections per origin, and a backlog of these starves every other call.
    let inFlight = false;
    const poll = () => {
      if (inFlight) return;
      inFlight = true;
      void Promise.all([getSwapTracker(), getSwapProgress()])
        .then(async ([next, progress]) => {
          if (cancelled) return;
          // A handoff to recovery releases the swap slot, and the tracker read with it; keeping
          // the last one leaves the circuit where the swap actually stopped.
          if (next) setTracker(next);
          if (progress !== null) return;
          if (next?.phase === "completed") {
            setPhase("finished");
            return;
          }
          // The slot also empties when the swap hands off to recovery. `swap://recovering` says
          // so, but a missed event must not read as a failure while recovery is already
          // claiming the funds back.
          // An unreadable status is not an inactive one: settle on nothing and ask again next poll.
          const recovery = swapId
            ? await getRecoveryStatus(swapId).catch(() => undefined)
            : null;
          if (cancelled || recovery === undefined) return;
          // Settles once: a poll still in flight must not repeat the handoff and its toast.
          cancelled = true;
          if (recovery?.active) handOffToRecovery();
          else setPhase("failed");
        })
        .catch(() => {})
        .finally(() => {
          inFlight = false;
        });
    };
    poll();
    const id = setInterval(poll, 2000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase, swapId]);

  // One more read on the terminal transition — the last 2s-cadence poll can be a beat stale by
  // the time swap://finished|failed fires, so refresh once more for the final per-router state.
  useEffect(() => {
    if (phase !== "finished" && phase !== "failed") return;
    void getSwapTracker()
      .then(setTracker)
      .catch(() => {});
  }, [phase]);

  // Backs the collapsed-by-default log panel below — same debug.log tail as the Logs page.
  // Gated on logsOpen too: no point polling a fetch nobody can see.
  useEffect(() => {
    if (!logsOpen) return;
    if (phase !== "running" && phase !== "finished" && phase !== "failed")
      return;
    let cancelled = false;
    let inFlight = false;
    const poll = () => {
      if (inFlight) return;
      inFlight = true;
      void getLogs(100)
        .then((next) => {
          if (!cancelled) setSwapLogs(next);
        })
        .catch(() => {})
        .finally(() => {
          inFlight = false;
        });
    };
    poll();
    const id = phase === "running" ? setInterval(poll, 2000) : undefined;
    return () => {
      cancelled = true;
      if (id) clearInterval(id);
    };
  }, [phase, logsOpen]);

  const otherUnits = useMemo(
    () => (["sats", "btc", "usd"] as Unit[]).filter((u) => u !== unit),
    [unit],
  );

  const spendableUtxos = useMemo(
    () => utxos.filter((u) => u.spendable && u.solvable),
    [utxos],
  );
  const filteredUtxos = useMemo(
    () =>
      spendableUtxos.filter(
        (u) =>
          classifySpendType(u.spendType) ===
          (utxoFilter === "regular" ? "Regular" : "Swap"),
      ),
    [spendableUtxos, utxoFilter],
  );
  const selectedUtxos = useMemo(() => {
    const set = new Set(selectedOutpoints.map((o) => `${o.txid}:${o.vout}`));
    return spendableUtxos.filter((u) => set.has(`${u.txid}:${u.vout}`));
  }, [selectedOutpoints, spendableUtxos]);
  const selectedTotal = useMemo(
    () => selectedUtxos.reduce((sum, u) => sum + u.amountSats, 0),
    [selectedUtxos],
  );

  function toggleOutpoint(u: UtxoEntry) {
    const key = `${u.txid}:${u.vout}`;
    setSelectedOutpoints((prev) => {
      const exists = prev.some((o) => `${o.txid}:${o.vout}` === key);
      if (exists) return prev.filter((o) => `${o.txid}:${o.vout}` !== key);
      return [...prev, { txid: u.txid, vout: u.vout }];
    });
  }

  // Never mix Regular/Swap UTXO kinds — switching the filter clears the other kind's selection.
  function changeUtxoFilter(next: UtxoFilter) {
    setUtxoFilter(next);
    setSelectedOutpoints([]);
  }

  const compatibleRouters = useMemo(
    () =>
      routers.filter((m) => {
        if (!m.offer) return false;
        const routerProtocol = m.protocol?.toLowerCase();
        // "unified" routers (the crate's current default) speak both — only a router pinned to
        // the other protocol is actually incompatible.
        return (
          !routerProtocol ||
          routerProtocol === "unified" ||
          routerProtocol === protocol
        );
      }),
    [routers, protocol],
  );

  function pickRouterCount(count: number) {
    setRouterCount(count);
    setSelectedRouters([]);
  }

  function toggleRouter(address: string) {
    setSelectedRouters((prev) =>
      prev.includes(address)
        ? prev.filter((a) => a !== address)
        : [...prev, address],
    );
  }

  // Nothing ticked in the advanced panel means automatic — no separate mode flag to keep in sync.
  const manualCoins = selectedOutpoints.length > 0;
  const manualRouters = selectedRouters.length > 0;

  const effectiveRouterCount = manualRouters
    ? selectedRouters.length
    : routerCount === CUSTOM_ROUTER_COUNT
      ? Math.max(1, Number(customRouterCount) || CUSTOM_ROUTER_COUNT)
      : routerCount;

  const estimateRouters = useMemo(() => {
    if (manualRouters)
      return compatibleRouters.filter((m) => selectedRouters.includes(m.address));
    return compatibleRouters.slice(0, Math.max(0, effectiveRouterCount));
  }, [compatibleRouters, manualRouters, selectedRouters, effectiveRouterCount]);

  // Deliberately not gated on the wallet sync: `estimate_swap_funding` is a `coin_select`
  // over the wallet's stored UTXOs with no network I/O, so it can quote before a sync lands.
  // Sync still gates *starting* a swap — see `warnings` and `handleStartSwap`.
  useEffect(() => {
    let cancelled = false;
    setFundingEstimate(null);
    if (amountSats <= 0 || feeRate < 1) {
      setFundingEstimateStatus("idle");
      return () => {
        cancelled = true;
      };
    }
    let attempt = 0;
    let timer: ReturnType<typeof setTimeout>;
    const run = () => {
      setFundingEstimateStatus("loading");
      setShortfall(null);
      const outpoints =
        manualCoins && selectedOutpoints.length > 0
          ? selectedOutpoints
          : undefined;
      void estimateSwapFunding(amountSats, protocol, outpoints, txCount, feeRate)
        .then((estimate) => {
          if (cancelled) return;
          setFundingEstimate(estimate);
          setFundingEstimateStatus("ready");
        })
        .catch((e: unknown) => {
          if (cancelled) return;
          setFundingEstimate(null);
          if (isAppError(e) && e.code === "INSUFFICIENT_FUNDS") {
            const details = e.details as { available?: number; required?: number } | undefined;
            setShortfall(
              typeof details?.available === "number" && typeof details.required === "number"
                ? { available: details.available, required: details.required }
                : null,
            );
            setFundingEstimateStatus("error");
            return;
          }
          // A quote most often fails because a long sync is holding the wallet lock, which
          // clears on its own — so back off and try again rather than leaving the summary
          // blank with no way forward. Manual Retry re-arms this budget.
          const delay = FUNDING_RETRY_DELAYS_MS[attempt];
          attempt += 1;
          if (delay === undefined) {
            setFundingEstimateStatus("error");
            return;
          }
          setFundingEstimateStatus("retrying");
          timer = setTimeout(run, delay);
        });
    };
    // Debounced so typing an amount doesn't quote every keystroke.
    timer = setTimeout(run, 150);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [amountSats, protocol, manualCoins, selectedOutpoints, txCount, feeRate, fundingAttempt]);

  const feeSummary = useMemo(() => {
    const hasCompleteRoute =
      amountSats > 0 &&
      effectiveRouterCount > 0 &&
      estimateRouters.length === effectiveRouterCount;
    const routerFee = hasCompleteRoute
      ? estimateRouteRouterFees(
          estimateRouters.map((router) => ({
            baseFee: router.offer!.baseFee,
            amountRelativeFeePct: router.offer!.amountRelativeFeePct,
            timeRelativeFeePct: router.offer!.timeRelativeFeePct,
          })),
          amountSats,
        )
      : null;
    const fundingFee = fundingEstimate?.feeSats ?? null;
    const routeMiningFee = fundingEstimate
      ? fundingEstimate.routeMiningFeePerRouterSats * effectiveRouterCount
      : null;
    const sweepFee = fundingEstimate?.sweepFeeSats ?? null;
    // Everything the route takes out of the amount itself, as opposed to the funding fee,
    // which the wallet pays on top of it — hence `amount - deductions` for the receive
    // figure and `+ fundingFee` only for the total.
    const routeDeductions =
      routerFee !== null && routeMiningFee !== null && sweepFee !== null
        ? routerFee + routeMiningFee + sweepFee
        : null;
    const totalFee =
      routeDeductions !== null && fundingFee !== null
        ? routeDeductions + fundingFee
        : null;
    const receiveAmount =
      routeDeductions !== null ? Math.max(0, amountSats - routeDeductions) : null;
    // Every on-chain cost as one figure: the funding transaction the wallet pays on top,
    // the per-hop route fee, and the claim sweep. The summary, the animation and the report
    // all take this split, so none of them can disagree about what a mining fee is.
    const miningFee =
      fundingFee !== null && routeMiningFee !== null && sweepFee !== null
        ? fundingFee + routeMiningFee + sweepFee
        : null;
    // What the whole swap costs as a share of what is being swapped — the number that makes
    // a fee comparable between a small swap and a large one.
    const feePct =
      totalFee !== null && amountSats > 0 ? (totalFee / amountSats) * 100 : null;
    return { routerFee, miningFee, feePct, totalFee, receiveAmount };
  }, [estimateRouters, effectiveRouterCount, amountSats, fundingEstimate]);

  const warnings = useMemo(() => {
    const list: string[] = [];
    if (walletSyncStatus !== "synced") {
      list.push(
        walletSyncStatus === "error"
          ? `Wallet sync failed: ${walletSyncError ?? "The chain backend is unavailable."}`
          : "Wait for the initial wallet sync before starting a swap.",
      );
    }
    if (amountInput.length > 0 && amountSats <= 0)
      list.push("Enter a valid amount.");
    if (feeRate < 1 && (feeKey === "custom" || feesFailed))
      list.push("Choose a fee rate: whole sats/vB, at least 1.");
    if (amountSats > 0 && liquidity && amountSats > liquidity.maxSwappable)
      list.push("Amount exceeds your swappable balance.");
    if (manualCoins && selectedTotal < amountSats)
      list.push("Selected UTXOs don't cover the swap amount.");
    // Each incoming contract is swept to its own output, and each has to clear dust.
    const dustFloor = fundingEstimate
      ? fundingEstimate.receiveDustSats * fundingEstimate.incomingUtxoCount
      : null;
    if (
      destination !== "address" &&
      amountSats > 0 &&
      feeSummary.receiveAmount !== null &&
      feeSummary.totalFee !== null &&
      fundingEstimate !== null &&
      dustFloor !== null &&
      feeSummary.receiveAmount < dustFloor
    ) {
      // What comes off the routed amount; the funding fee is paid on top of it. Rounded up to a
      // hundred so the routers' percentage fees, which grow with the amount, are still covered.
      const deductions = feeSummary.totalFee - fundingEstimate.feeSats;
      const minimum = Math.ceil((deductions + dustFloor) / 100) * 100;
      list.push(
        `After fees you would receive about ${formatNumber(feeSummary.receiveAmount)} sats, below the ${formatNumber(dustFloor)}-sat dust limit. Enter at least ${formatNumber(minimum)} sats${feeKey === "slow" ? "" : ", or choose a lower fee rate"}.`,
      );
    }
    if (destination === "address" && paymentAddress.trim().length === 0)
      list.push("Enter the address this swap should pay.");
    if (effectiveRouterCount < 1) list.push("Choose at least one router.");
    if (compatibleRouters.length === 0) {
      list.push(`No compatible ${protocol} routers found in the offerbook.`);
    } else if (!manualRouters && effectiveRouterCount > compatibleRouters.length) {
      list.push(
        `Only ${compatibleRouters.length} compatible router${compatibleRouters.length === 1 ? "" : "s"} available for ${effectiveRouterCount} hops.`,
      );
    }
    return list;
  }, [
    amountInput,
    amountSats,
    feeRate,
    feeKey,
    feesFailed,
    destination,
    paymentAddress,
    liquidity,
    manualCoins,
    selectedOutpoints,
    selectedTotal,
    feeSummary.receiveAmount,
    feeSummary.totalFee,
    fundingEstimate,
    manualRouters,
    selectedRouters,
    compatibleRouters,
    effectiveRouterCount,
    protocol,
    walletSyncStatus,
    walletSyncError,
    fundingEstimateStatus,
  ]);

  // Null once the quote is ready: the numbers underneath already are the quote. Reports the
  // quote's own state, not the sync's — the quote no longer waits on one.
  const quoteStatus =
    fundingEstimateStatus === "loading"
      ? "Calculating wallet quote"
      : fundingEstimateStatus === "retrying"
        ? "Retrying quote"
        : fundingEstimateStatus === "error"
          ? "Quote unavailable"
          : fundingEstimate
            ? null
            : "";

  // Pinned routers already show up in the Routers section; a UTXO pick has no other home.
  const advancedSummary = manualCoins
    ? `${selectedOutpoints.length} UTXO${selectedOutpoints.length === 1 ? "" : "s"} selected`
    : null;

  // Same rule as Send: an unconfirmed earlier payment could be paid twice by starting more
  // on-chain work.
  // See SendPage: an unreadable journal holds this as firmly as a populated one.
  const paymentsHeld = useUnresolvedStore(spendingBlocked);
  const canStart =
    amountSats > 0 &&
    fundingEstimate !== null &&
    warnings.length === 0 &&
    !submitting &&
    !paymentsHeld;

  // Only while `submitting`: this is the one window where the taker mutex is held by a call that
  // reports nothing, so the tracker file is the only progress signal there is.
  useEffect(() => {
    if (!submitting || preparingSince === null) {
      setPreparation(null);
      return;
    }
    let cancelled = false;
    const poll = () => {
      void getSwapPreparation(preparingSince)
        .then((next) => {
          if (!cancelled) setPreparation(next);
        })
        .catch(() => {});
    };
    poll();
    const id = setInterval(poll, PREPARATION_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [submitting, preparingSince]);

  async function handleStartSwap() {
    if (useWalletCacheStore.getState().syncStatus !== "synced") {
      // Not a fault: the sync is expected to be in progress on arrival, and it clears on its own.
      pushToast(
        "warning",
        "Wait for the wallet sync to finish before starting a swap.",
      );
      return;
    }
    const request: SwapRequest = {
      protocol,
      amountSats,
      routerCount: effectiveRouterCount,
      outpoints:
        manualCoins && selectedOutpoints.length > 0
          ? selectedOutpoints
          : undefined,
      preferredRouters:
        manualRouters && selectedRouters.length > 0
          ? selectedRouters
          : undefined,
      txCount,
      feeRate,
      paymentAddress:
        destination === "address" ? paymentAddress.trim() : undefined,
    };
    prepare.run(request);
  }

  // A preparation that failed while this page was away still owes its message.
  const { error: prepareError, clearError: clearPrepareError } = prepare;
  useEffect(() => {
    if (!prepareError) return;
    pushToast("error", prepareError);
    clearPrepareError();
  }, [prepareError, clearPrepareError, pushToast]);

  // Counted from when the review arrived, which may have been while this page was away.
  const reviewAt = prepare.reviewAt;
  useEffect(() => {
    setReviewExpired(false);
    if (reviewAt === null) return;
    const remaining = REVIEW_TTL_MS - (Date.now() - reviewAt);
    if (remaining <= 0) {
      setReviewExpired(true);
      return;
    }
    const id = setTimeout(() => setReviewExpired(true), remaining);
    return () => clearTimeout(id);
  }, [reviewAt]);

  function cancelReview() {
    if (review) void cancelSwap(review.swapId).catch(() => {});
    prepare.clearReview();
  }

  async function confirmSwap() {
    if (!review) return;
    setConfirming(true);
    try {
      await startSwap(review.swapId);
      setSummary(review);
      setSwapId(review.swapId);
      setStartedAt(Math.floor(Date.now() / 1000));
      setPhase("running");
      prepare.clearReview();
    } catch (e) {
      const err = isAppError(e) ? e : null;
      pushToast("error", err?.message ?? "Failed to start swap.");
      cancelReview();
    } finally {
      setConfirming(false);
    }
  }

  // The funds are in contracts and the crate is already claiming them back, so this page
  // hands itself back for the next swap and the recovery page takes over.
  function handOffToRecovery() {
    resetWizard();
    pushToast("warning", "The swap stopped. Recovering your funds — see Recovery.");
    navigate("/swap/recovery");
  }

  function resetWizard() {
    setPhase("configure");
    setSummary(null);
    setSwapId(null);
    setStartedAt(null);
    setFailure(null);
    setTracker(null);
    setAmountInput("");
    setSelectedOutpoints([]);
    setSelectedRouters([]);
    void loadReference().catch(() => {});
  }

  if (submitting) {
    // Every step here is real: no record on disk yet means the crate is still refreshing the
    // marketplace, a record means routers are picked, and the per-router `negotiated` flags are
    // where the count comes from. Nothing is faked forward on a timer.
    const discovered = preparation !== null;
    const negotiated = discovered && preparation.negotiatedCount >= preparation.routerCount;
    const negotiating = discovered && !negotiated;
    return (
      <div className="grid h-full place-items-center px-8 py-10">
        <Card className="flex w-full max-w-md flex-col gap-5 border-line-strong p-7">
          <div>
            <h1 className="font-header text-[19px] font-bold text-foreground">
              Preparing your swap
            </h1>
            <p className="mt-1 text-[12.5px] leading-5 text-muted">
              No funds have moved yet. Every router has to agree terms over its own Tor circuit
              first, one after another, which is what takes the time.
            </p>
          </div>
          <Checklist
            steps={[
              {
                label: "Refreshing the marketplace",
                state: discovered ? "passed" : "running",
              },
              {
                label: discovered
                  ? `Agreeing terms · ${preparation.negotiatedCount} of ${preparation.routerCount} routers`
                  : "Agreeing terms with the routers",
                state: negotiated ? "passed" : negotiating ? "running" : "idle",
              },
            ]}
          />
        </Card>
      </div>
    );
  }

  if (review) {
    const paying = review.payment !== undefined;
    const feePct =
      review.sendAmountSats > 0
        ? (review.totalEstimatedFeeSats / review.sendAmountSats) * 100
        : null;
    // Compared with the offer the form quoted from: the crate re-fetches each router's offer
    // while negotiating and accepts any new price below 100%.
    const raised = review.routers.filter((hop) => {
      const quoted = routers.find((r) => r.address === hop.address)?.offer;
      return (
        quoted !== undefined &&
        (hop.baseFee > quoted.baseFee ||
          hop.amountRelativeFeePct > quoted.amountRelativeFeePct ||
          hop.timeRelativeFeePct > quoted.timeRelativeFeePct)
      );
    });
    return (
      <div className="flex h-full flex-col items-center overflow-y-auto px-8 py-10">
        {/* shrink-0: in this flex column the card would shrink to the viewport, and its
            overflow-hidden then clips the bottom instead of letting the page scroll. */}
        <Card className="flex w-full max-w-lg shrink-0 flex-col gap-5 border-line-strong p-7">
          <div>
            <h1 className="font-header text-[19px] font-bold text-foreground">
              Review your swap
            </h1>
            <p className="mt-1 text-[12.5px] leading-5 text-muted">
              These are the terms the routers agreed to. No funds have moved yet.
            </p>
          </div>

          {raised.length > 0 && (
            <Notice tone="warning" icon={<AlertTriangle size={16} strokeWidth={2} />}>
              {raised.length === 1
                ? "A router raised its fee"
                : `${raised.length} routers raised their fees`}{" "}
              since the quote. Check the fees below before you start.
            </Notice>
          )}

          <div className="flex flex-col gap-2">
            {review.routers.map((hop, i) => {
              // The negotiated summary carries no name; the offerbook entry the quote came from does.
              const name = routers.find((r) => r.address === hop.address)?.offer?.name;
              return (
              <div
                key={hop.address}
                className={`flex flex-col gap-1 rounded-control border px-3.5 py-2.5 ${
                  raised.includes(hop)
                    ? "border-warning/50 bg-warning/[0.06]"
                    : "border-line bg-surface-raised"
                }`}
              >
                <div className="flex items-center justify-between gap-3">
                  <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
                    Router {i + 1}
                  </span>
                  <SatsAmount
                    sats={hop.estimatedFeeSats}
                    className={`text-[12px] font-semibold ${raised.includes(hop) ? "text-warning" : "text-foreground"}`}
                  />
                </div>
                {name && (
                  <span className="truncate text-[13px] font-semibold text-foreground">{name}</span>
                )}
                <Identifier value={hop.address} className="text-[11px] text-muted" />
                <span className="font-mono text-[10.5px] text-subtle">
                  base {formatNumber(hop.baseFee)} sats · {hop.amountRelativeFeePct}% ·
                  time {hop.timeRelativeFeePct}%
                </span>
              </div>
              );
            })}
          </div>

          <div className="flex flex-col gap-1.5 border-t border-dashed border-line pt-3 text-[12px]">
            <div className="flex items-center justify-between">
              <span className="text-subtle">Swap amount</span>
              <SatsAmount sats={review.sendAmountSats} className="font-semibold text-foreground" />
            </div>
            <div className="flex items-center justify-between">
              <span className="text-subtle">Router fees</span>
              <SatsAmount sats={review.routerFeeSats} className="font-semibold text-foreground" />
            </div>
            <div className="flex items-center justify-between">
              <span className="text-subtle">Total mining fees</span>
              <SatsAmount sats={review.miningFeeSats} className="font-semibold text-foreground" />
            </div>
            <div className="flex items-center justify-between border-t border-line pt-1.5">
              <span className="text-subtle">Max total fees</span>
              <SatsAmount sats={review.totalEstimatedFeeSats} className="font-bold text-primary" />
            </div>
            <div className="flex items-center justify-between">
              <span className="text-subtle">% Fees</span>
              <strong className="font-numeric tabular-nums font-semibold text-foreground">
                {feePct === null ? "—" : `${feePct.toFixed(2)}%`}
              </strong>
            </div>
          </div>

          {paying ? (
            <AmountTile label="Receiver gets exactly">
              <SatsAmount sats={review.payment!.amountSats} className="text-success" />
            </AmountTile>
          ) : (
            <AmountTile label="You receive at least">
              <SatsAmount sats={review.estimatedReceiveAmountSats} className="text-success" />
            </AmountTile>
          )}

          {reviewExpired ? (
            <>
              <Notice tone="warning" icon={<AlertTriangle size={16} strokeWidth={2} />}>
                These terms are too old to start. Get a fresh quote from the routers.
              </Notice>
              <div className="flex gap-3">
                <Button size="md" variant="secondary" className="flex-1" onClick={cancelReview}>
                  Cancel
                </Button>
                <Button
                  size="md"
                  className="flex-1"
                  onClick={() => {
                    cancelReview();
                    void handleStartSwap();
                  }}
                >
                  <RefreshCw size={14} strokeWidth={2} /> Get a fresh quote
                </Button>
              </div>
            </>
          ) : (
            <div className="flex gap-3">
              <Button
                size="md"
                variant="secondary"
                className="flex-1"
                disabled={confirming}
                onClick={cancelReview}
              >
                Cancel
              </Button>
              <Button
                size="md"
                className="flex-1"
                loading={confirming}
                onClick={() => void confirmSwap()}
              >
                Start swap
              </Button>
            </div>
          )}
        </Card>
      </div>
    );
  }

  if (phase === "running" || phase === "finished" || phase === "failed") {
    const contractsBroadcasted = failure?.code === "CONTRACTS_BROADCASTED";

    // Navigating away and back remounts this component, resetting `summary` to null — it only
    // ever comes from prepareSwap's return value, which can't be re-fetched for an already-running
    // swap. Fall back to the live tracker (which survives remounts fine, since it's a fresh read
    // off disk each time) so the screen doesn't get stuck "reconnecting" forever.
    const routeKnown =
      (summary?.routers.length ?? tracker?.routers.length ?? 0) > 0 ||
      tracker !== null;
    const displaySendAmountSats =
      summary?.sendAmountSats ?? tracker?.sendAmountSats;

    return (
      <div className="flex h-full flex-col items-center overflow-y-auto px-8 py-10">
        <div className="w-full max-w-4xl">
          <div className="flex items-center justify-center gap-3">
            {phase === "running" && (
              <RefreshCw
                size={30}
                strokeWidth={1.8}
                className="animate-spin text-primary"
              />
            )}
            {phase === "finished" && (
              <CheckCircle2
                size={30}
                strokeWidth={1.8}
                className="text-success"
              />
            )}
            {phase === "failed" && (
              <XCircle size={30} strokeWidth={1.8} className="text-danger" />
            )}
            <h1 className="font-header text-[26px] font-bold text-foreground">
              {phase === "running" && "Swap in progress"}
              {phase === "finished" && "Swap Complete"}
              {phase === "failed" && "Swap Failed"}
            </h1>
          </div>

          {phase === "running" && (
            <Notice
              tone="warning"
              icon={<AlertTriangle size={20} strokeWidth={2} />}
              className="mt-5"
            >
              <p className="text-[14px] font-bold text-warning">
                Do not stop the app during the swap.
              </p>
              <p className="mt-1 text-muted">
                If it stops or crashes, start it again as soon as you can.
              </p>
            </Notice>
          )}

          <Card className="mt-5 flex flex-col gap-4 border-line-strong p-6">
            {routeKnown ? (
              <div className="flex flex-col gap-4">
                <SwapCircuit view={circuit} />
                <Vitals
                  view={circuit}
                  elapsed={
                    startedAt === null ? (
                      "—"
                    ) : (
                      <Elapsed startedAt={startedAt} active={phase === "running"} />
                    )
                  }
                />
                <NowPanel view={circuit} />
              </div>
            ) : (
              <div className="grid min-h-[220px] place-items-center gap-2.5 text-center text-[13px] text-subtle">
                <RefreshCw
                  size={32}
                  strokeWidth={1.6}
                  className="animate-spin text-primary"
                />
                <span>Loading swap progress…</span>
              </div>
            )}

            <div className="rounded-control border border-line-strong bg-surface-raised px-3.5 py-3 text-center">
              <div className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
                Amount
              </div>
              <div className="mt-1 font-mono text-[13px] font-semibold text-foreground">
                {displaySendAmountSats !== undefined
                  ? formatNumber(displaySendAmountSats)
                  : "—"}
              </div>
            </div>

            {phase === "failed" && (
              <div className="flex flex-col gap-2.5 rounded-control border border-danger/35 bg-danger/[0.06] px-3.5 py-3">
                <div className="flex items-start gap-2 text-[12px] text-danger">
                  <ShieldAlert
                    size={15}
                    strokeWidth={2}
                    className="mt-0.5 flex-none"
                  />
                  <span>
                    {contractsBroadcasted
                      ? "Funds are on-chain and safe — recovery has already started automatically."
                      : (tracker?.failureReason ??
                        failure?.message ??
                        "Something went wrong.")}
                  </span>
                </div>
              </div>
            )}

            {(phase === "finished" || phase === "failed") && (
              <div className="flex gap-2.5">
                <Button variant="secondary" className="flex-1" onClick={resetWizard}>
                  Back to Swap Page
                </Button>
                {swapId && (
                  <Button
                    className="flex-1"
                    onClick={() =>
                      navigate(`/swap/reports/${encodeURIComponent(swapId)}`)
                    }
                  >
                    View Report
                  </Button>
                )}
              </div>
            )}

            <Disclosure label="Swap Log" onOpenChange={setLogsOpen}>
              <div className="rounded-control border border-line bg-surface-raised">
                <LogViewer lines={swapLogs} className="max-h-64" />
              </div>
            </Disclosure>
          </Card>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col overflow-y-auto px-8 pb-8 pt-2">
      <div className="flex shrink-0 items-start justify-between gap-3 pb-4">
        <div>
          <h1 className="font-header text-[26px] font-bold text-foreground">
            Initiate Swap
          </h1>
          <p className="mt-1 text-[13.5px] text-muted">
            Route a private Bitcoin swap through multiple routers over Tor.
          </p>
        </div>
        <div className="flex items-center gap-2">
          {RECOVERY_UI_ENABLED && (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => navigate("/swap/recovery")}
              className={recoveryActive ? "border-warning/50 text-warning" : ""}
            >
              <LifeBuoy size={14} strokeWidth={2} />
              Recovery
            </Button>
          )}
          <Button
            variant="secondary"
            size="sm"
            onClick={() => navigate("/swap/reports")}
          >
            <FileText size={14} strokeWidth={2} />
            Swap Reports
          </Button>
        </div>
      </div>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_340px]">
        <Card className="flex flex-col gap-5 border-line-strong p-6">
          <div className="flex items-center gap-2.5">
            <span className="flex h-[30px] w-[30px] items-center justify-center rounded-full border border-primary/40 bg-primary/[0.08] text-primary">
              <ArrowLeftRight size={15} strokeWidth={2} />
            </span>
            <h2 className="font-header text-[15px] font-bold text-foreground">
              Amount To Swap
            </h2>
          </div>

          <div className="flex flex-col gap-2">
            <div className="grid grid-cols-[1fr_auto] items-end gap-3">
              <TextField
                label="Amount"
                inputMode="decimal"
                placeholder="0"
                value={amountInput}
                onChange={(e) => setAmountInput(e.target.value)}
              />
              <SegmentedToggle
                groupId="swap-unit"
                value={unit}
                onChange={changeUnit}
                options={[
                  { value: "sats", label: "sats" },
                  { value: "btc", label: "BTC" },
                  {
                    value: "usd",
                    label: "USD",
                    disabled: btcPrice === null,
                    title:
                      btcPrice === null
                        ? "BTC price unavailable"
                        : btcPriceCached
                          ? "Using the last saved BTC price because the live update failed"
                          : undefined,
                  },
                ]}
              />
            </div>
            <div className="flex items-center justify-between px-1 text-[11px] text-subtle">
              <div className="flex items-center gap-4">
                <span>
                  {formatUnitAmount(amountSats, otherUnits[0], btcPrice) ?? "—"}
                </span>
                <span>
                  {formatUnitAmount(amountSats, otherUnits[1], btcPrice) ?? "—"}
                </span>
              </div>
              <button
                type="button"
                onClick={() => {
                  changeUnit("sats");
                  setAmountInput(String(liquidity?.maxSwappable ?? 0));
                }}
                className="flex items-center gap-1 font-semibold text-primary hover:text-primary-hover"
              >
                Use max swappable:{" "}
                <SatsAmount sats={liquidity?.maxSwappable ?? 0} />
              </button>
            </div>
          </div>

          <div className="flex flex-col gap-2.5 border-t border-line pt-5">
            <div className="flex items-center justify-between gap-3">
              <h2 className="font-header text-[13.5px] font-bold text-foreground">
                Destination
              </h2>
              <SegmentedToggle
                groupId="swap-destination"
                value={destination}
                onChange={setDestination}
                options={[
                  { value: "wallet", label: "My wallet" },
                  { value: "address", label: "Payswap" },
                ]}
              />
            </div>
            {destination === "address" && (
              <TextField
                label="Pay to address"
                placeholder="bc1…"
                value={paymentAddress}
                onChange={(e) => setPaymentAddress(e.target.value)}
              />
            )}
            <p className="text-[11.5px] text-subtle">
              {destination === "address"
                ? "The last hop settles straight to this address, so the receiver gets exactly the amount above and your wallet covers the fees on top. The route cost is solved when the swap is prepared."
                : "The swapped coins come back to this wallet as a fresh UTXO."}
            </p>
          </div>

          <div className="flex items-center justify-between gap-3 border-t border-line pt-5">
            <h2 className="font-header text-[13.5px] font-bold text-foreground">
              Protocol
            </h2>
            <SegmentedToggle
              groupId="swap-protocol"
              value={protocol}
              onChange={setProtocol}
              options={[
                { value: "taproot", label: "Taproot" },
                { value: "legacy", label: "Legacy" },
              ]}
            />
          </div>

          <div className="flex flex-col gap-2.5 border-t border-line pt-5">
            <div className="flex items-center justify-between">
              <h2 className="font-header text-[13.5px] font-bold text-foreground">
                Routers
              </h2>
              <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
                {effectiveRouterCount} from {compatibleRouters.length} available
              </span>
            </div>
            <div className="grid grid-cols-4 gap-2">
              {ROUTER_COUNT_PRESETS.map((n) => (
                <PresetTile
                  key={n}
                  onClick={() => pickRouterCount(n)}
                  selected={!manualRouters && routerCount === n}
                  label="Routers"
                  value={n}
                />
              ))}
              <PresetTile
                onClick={() => pickRouterCount(CUSTOM_ROUTER_COUNT)}
                selected={!manualRouters && routerCount === CUSTOM_ROUTER_COUNT}
                label="Custom"
                value={`${CUSTOM_ROUTER_COUNT}+`}
              />
            </div>
            {!manualRouters && routerCount === CUSTOM_ROUTER_COUNT && (
              <TextField
                label="Number of routers"
                inputMode="numeric"
                value={customRouterCount}
                onChange={(e) => setCustomRouterCount(e.target.value)}
              />
            )}
            <p className="text-[11.5px] text-subtle">
              {manualRouters
                ? `Route pinned to ${selectedRouters.length} specific router${selectedRouters.length === 1 ? "" : "s"} in advanced options — pick a count to go back to automatic.`
                : "More routers means stronger privacy and higher fees."}
            </p>
          </div>

          <div className="flex flex-col gap-2.5 border-t border-line pt-5">
            <h2 className="font-header text-[13.5px] font-bold text-foreground">
              Fee Rate
            </h2>
            <FeeRateField
              fees={fees}
              failed={feesFailed}
              onRetry={retryFees}
              choice={feeKey}
              onChoice={setFeeKey}
              custom={customFeeRate}
              onCustom={setCustomFeeRate}
              whole
            />
          </div>

          <div className="flex flex-col gap-2 border-t border-line pt-5">
            <Disclosure label="Advanced options" onOpenChange={setAdvancedOpen}>
              <div className="flex flex-col gap-5 pt-1">
                <div className="flex flex-col gap-2.5">
                  <div className="flex items-center justify-between">
                    <h3 className="font-header text-[12.5px] font-bold text-foreground">
                      Funding Splits
                    </h3>
                    <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
                      {txCount} per hop
                    </span>
                  </div>
                  <input
                    type="range"
                    min={1}
                    max={MAX_TX_COUNT}
                    step={1}
                    value={txCount}
                    onChange={(e) => setTxCount(Number(e.target.value))}
                    className="accent-[var(--color-primary)]"
                    aria-label="Funding transactions per hop"
                  />
                  <p className="text-[11.5px] text-subtle">
                    Every hop is funded by this many contracts instead of one, so an observer
                    sees several smaller amounts rather than the whole swap in one transaction.
                    More splits cost more in mining fees. A router short of liquidity may forward
                    fewer than asked.
                  </p>
                </div>

                <div className="flex flex-col gap-2.5">
                  <div className="flex items-center justify-between">
                    <h3 className="font-header text-[12.5px] font-bold text-foreground">
                      Pin Specific Routers
                    </h3>
                    <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
                      {manualRouters
                        ? `${selectedRouters.length} pinned`
                        : "Automatic"}
                    </span>
                  </div>
                  <p className="text-[11.5px] text-subtle">
                    Tick routers to route through them specifically, or leave
                    them all unticked to auto-select.
                  </p>
                  <div className="flex max-h-45 flex-col gap-1.5 overflow-y-auto">
                    {compatibleRouters.length === 0 && (
                      <p className="text-[11.5px] text-subtle">
                        No compatible {protocol} routers in the offerbook.
                      </p>
                    )}
                    {compatibleRouters.map((m) => (
                      <label
                        key={m.address}
                        className="flex cursor-pointer items-center justify-between gap-3 rounded-control border border-line bg-surface-raised px-3 py-2"
                      >
                        <span className="flex min-w-0 items-center gap-2 font-mono text-[11px] text-muted">
                          <input
                            type="checkbox"
                            checked={selectedRouters.includes(m.address)}
                            onChange={() => toggleRouter(m.address)}
                            className="accent-primary"
                          />
                          {m.offer?.name ? (
                            <span className="flex min-w-0 flex-col leading-[1.35]">
                              <span className="truncate text-[12px] font-semibold text-foreground">{m.offer.name}</span>
                              <span className="truncate font-mono text-[10.5px] text-subtle">{routerName(m.address)}</span>
                            </span>
                          ) : (
                            <span className="font-mono text-[11px] leading-[1.45] text-muted">{routerName(m.address)}</span>
                          )}
                        </span>
                        <span className="flex flex-none items-center gap-2 font-mono text-[11px] text-subtle">
                          {m.offer?.amountRelativeFeePct.toFixed(3)}%
                          <SatsAmount
                            sats={m.offer?.bondAmountSats ?? 0}
                            className="text-foreground"
                          />
                        </span>
                      </label>
                    ))}
                  </div>
                </div>

                <div className="flex flex-col gap-2.5 border-t border-line pt-4">
                  <div className="flex items-center justify-between">
                    <h3 className="font-header text-[12.5px] font-bold text-foreground">
                      Coin Selection
                    </h3>
                    <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
                      {manualCoins
                        ? `${selectedOutpoints.length} selected`
                        : "Automatic"}
                    </span>
                  </div>
                  <p className="text-[11.5px] text-subtle">
                    Tick the UTXOs to fund this swap, or leave them all unticked
                    to let the wallet choose.
                  </p>
                  <SegmentedToggle
                    groupId="swap-utxo-filter"
                    value={utxoFilter}
                    onChange={changeUtxoFilter}
                    options={[
                      { value: "regular", label: "Regular" },
                      { value: "swap", label: "Swap" },
                    ]}
                  />
                  <div className="flex max-h-45 flex-col gap-1.5 overflow-y-auto">
                    {filteredUtxos.length === 0 && (
                      <p className="text-[11.5px] text-subtle">
                        No spendable {utxoFilter} UTXOs.
                      </p>
                    )}
                    {filteredUtxos.map((u) => {
                      const key = `${u.txid}:${u.vout}`;
                      const checked = selectedOutpoints.some(
                        (o) => `${o.txid}:${o.vout}` === key,
                      );
                      return (
                        <label
                          key={key}
                          // The wallet's own UTXO columns, in the same order: address, script,
                          // type, amount. Picking a coin here and reading it there have to be
                          // the same act of recognition.
                          className="grid cursor-pointer grid-cols-[18px_minmax(0,1fr)_74px_74px_auto] items-center gap-2 rounded-control border border-line bg-surface-raised px-3 py-2 font-mono text-[11px] text-muted"
                        >
                          <input
                            type="checkbox"
                            checked={checked}
                            onChange={() => toggleOutpoint(u)}
                            className="accent-primary"
                          />
                          {u.address ? (
                            <Identifier value={u.address} className="text-[11px] leading-[1.45]" />
                          ) : (
                            <Identifier
                              value={`${u.txid}:${u.vout}`}
                              className="text-[11px] leading-[1.45] text-subtle"
                            />
                          )}
                          <span className="rounded-control border border-line px-1.5 py-0.5 text-center text-[9px] text-subtle">
                            {classifySpendType(u.spendType)}
                          </span>
                          <span className="text-right font-numeric text-[10px] text-subtle">
                            {formatNumber(u.confirmations)} conf
                          </span>
                          <SatsAmount
                            sats={u.amountSats}
                            className="flex-none text-[11px] font-semibold text-foreground"
                          />
                        </label>
                      );
                    })}
                  </div>
                  {manualCoins && (
                    <p className="text-[11.5px] text-subtle">
                      Selected:{" "}
                      <SatsAmount
                        sats={selectedTotal}
                        className="text-foreground"
                      />
                    </p>
                  )}
                </div>
              </div>
            </Disclosure>
            {/* Advanced picks survive collapsing the panel, so they'd otherwise be invisible. */}
            {!advancedOpen && advancedSummary && (
              <p className="px-1 text-[11.5px] text-subtle">
                {advancedSummary}
              </p>
            )}
          </div>

          {warnings.length > 0 && (
            <div className="flex flex-col gap-1.5 rounded-control border border-warning/35 bg-warning/[0.08] px-3.5 py-2.5">
              {warnings.map((w) => (
                <div
                  key={w}
                  className="flex items-start gap-2 text-[12px] text-warning"
                >
                  <AlertTriangle
                    size={13}
                    strokeWidth={2}
                    className="mt-0.5 flex-none"
                  />
                  <span>{w}</span>
                </div>
              ))}
            </div>
          )}

          <UnresolvedPayments verb="swapping" />
          <Button
            size="md"
            disabled={!canStart}
            loading={submitting}
            onClick={() => void handleStartSwap()}
          >
            Start Swap
          </Button>
        </Card>

        <div className="flex flex-col gap-4">
          <Card className="border-line-strong p-5">
            <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
              Swappable Balance
            </span>
            <div className="mt-1.5">
              <SatsAmount
                sats={liquidity?.maxSwappable ?? 0}
                className="text-[26px] font-bold text-primary"
              />
            </div>
            <p className="mt-1 text-[12px] text-muted">
              {liquidity
                ? `${(liquidity.maxSwappable / SATS_PER_BTC).toFixed(8)} BTC`
                : "…"}
            </p>
          </Card>

          <Card className="flex flex-col gap-3 border-line-strong p-5">
            <div className="flex items-center justify-between">
              <h3 className="font-header text-[14px] font-bold text-foreground">
                Swap Summary
              </h3>
              {quoteStatus && (
                <span className="rounded-pill border border-primary/35 bg-primary/[0.08] px-2.5 py-1 font-mono text-[10px] uppercase tracking-widest text-primary">
                  {quoteStatus}
                </span>
              )}
            </div>

            {fundingEstimateStatus === "error" && (
              <div className="flex flex-col gap-2 rounded-control border border-warning/35 bg-warning/[0.06] px-3 py-2.5">
                <p className="text-[11.5px] leading-4 text-warning">
                  {shortfall
                    ? `Not enough funds: this amount plus its funding transaction's mining fee needs ${shortfall.required.toLocaleString("en-US")} sats, and the wallet can spend ${shortfall.available.toLocaleString("en-US")}.`
                    : "The wallet couldn't quote a funding transaction for this amount and coin selection, so the mining-fee rows are blank. Router fees come from the offerbook and are unaffected."}
                </p>
                <div>
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => setFundingAttempt((n) => n + 1)}
                  >
                    <RefreshCw size={13} strokeWidth={1.9} />
                    Retry quote
                  </Button>
                </div>
              </div>
            )}

            <div className="flex flex-col gap-1.5 text-[12px]">
              <div className="flex items-center justify-between">
                <span className="text-subtle">Swap amount</span>
                <SatsAmount
                  sats={amountSats}
                  className="font-semibold text-foreground"
                />
              </div>
              <div className="flex items-center justify-between">
                <span className="text-subtle">Routers</span>
                <strong className="font-mono text-foreground">
                  {effectiveRouterCount}
                </strong>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-subtle">Outgoing UTXOs</span>
                <strong className="font-mono text-foreground">
                  {fundingEstimate?.outgoingUtxoCount ?? "—"}
                </strong>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-subtle">Incoming UTXOs</span>
                <strong className="font-mono text-foreground">
                  {destination === "address"
                    ? 0
                    : fundingEstimate
                      ? // A ceiling, so say so: a router short of liquidity forwards fewer
                        // splits and every hop after it inherits the smaller count.
                        `${fundingEstimate.incomingUtxoCount > 1 ? "≤ " : ""}${fundingEstimate.incomingUtxoCount}`
                      : "—"}
                </strong>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-subtle">Funding tx size</span>
                <strong className="font-mono text-foreground">
                  {fundingEstimate ? `${fundingEstimate.vbytes} vB` : "—"}
                </strong>
              </div>
            </div>

            {/* Every figure here is the ceiling the crate quotes — routers billed at the full
                input budget on every split — so the settled cost can only come in under it.
                That is what the ≈ on each of them says. */}
            <div className="flex flex-col gap-1.5 border-t border-dashed border-line pt-3 text-[12px]">
              <div className="flex items-center justify-between">
                <span className="text-subtle">Router fees</span>
                <EstimatedSats
                  sats={feeSummary.routerFee}
                  className="font-semibold text-foreground"
                />
              </div>
              <div className="flex items-center justify-between">
                <span className="text-subtle">Total mining fees</span>
                <EstimatedSats
                  sats={feeSummary.miningFee}
                  className="font-semibold text-foreground"
                />
              </div>
              <div className="flex items-center justify-between border-t border-line pt-1.5">
                <span className="text-subtle">Max total fees</span>
                <EstimatedSats
                  sats={feeSummary.totalFee}
                  className="font-bold text-primary"
                />
              </div>
              <div className="flex items-center justify-between">
                <span className="text-subtle">% Fees</span>
                {feeSummary.feePct === null ? (
                  <strong className="font-mono text-subtle">—</strong>
                ) : (
                  <span className="inline-flex items-baseline gap-1">
                    <span className="font-mono text-subtle" aria-label="approximately">≈</span>
                    <strong className="font-numeric tabular-nums font-semibold text-foreground">
                      {feeSummary.feePct.toFixed(2)}%
                    </strong>
                  </span>
                )}
              </div>
            </div>

            {destination === "address" ? (
              <AmountTile label="Receiver gets exactly">
                <SatsAmount sats={amountSats} className="text-success" />
              </AmountTile>
            ) : (
              <AmountTile label="You receive at least">
                <EstimatedSats
                  sats={feeSummary.receiveAmount}
                  className="text-success"
                />
              </AmountTile>
            )}
          </Card>
        </div>
      </div>
    </div>
  );
}
