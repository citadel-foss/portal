import { openExternal } from "../../platform";
import { ArrowDown, ArrowUp, ArrowUpDown, ChevronDown, ExternalLink, Inbox, RefreshCw, Search } from "lucide-react";
import { motion } from "framer-motion";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getOffers, pollRouter, removeRouter, syncOfferbook } from "../../api/commands";
import { isAppError } from "../../api/types";
import type { Router } from "../../api/types";
import { Card, IndeterminateBar, Modal, SatsAmount, StatStrip, Tooltip } from "../../components/ui/display";
import { Button } from "../../components/ui/inputs";
import { estimateRouterFee, formatTorEndpoint } from "../../lib/market-format";
import { explorerTxUrl } from "../../lib/wallet-format";
import { useToastStore } from "../../store/toast";

type RouterStatus = "good" | "bad" | "unresponsive";
type SortKey = "baseFee" | "liquidityFee" | "timeRate" | "minSwap" | "maxSwap" | "bond";
type SortDir = "asc" | "desc";

// The crate re-syncs the offerbook on its own every 10 minutes, so without re-reading it the
// page would show whatever was there at mount for as long as it stays open. Polled well under
// that interval rather than matching it: the two timers are unaligned, so an equal period would
// leave results sitting unseen for most of a cycle. get_offers is an in-memory snapshot with no
// network I/O, which is what makes the tighter cadence free.
const OFFERBOOK_REREAD_MS = 60 * 1000;

// Muted (desaturated) variants of the success/warning/danger tokens, one per router status.
const STATUS_TAB_CLASS: Record<RouterStatus, { text: string; glow: string }> = {
  good: { text: "text-success", glow: "bg-success/15 shadow-[0_0_12px_color-mix(in_oklab,var(--color-success)_35%,transparent)]" },
  bad: { text: "text-danger", glow: "bg-danger/15 shadow-[0_0_12px_color-mix(in_oklab,var(--color-danger)_35%,transparent)]" },
  unresponsive: { text: "text-warning", glow: "bg-warning/15 shadow-[0_0_12px_color-mix(in_oklab,var(--color-warning)_35%,transparent)]" },
};

// Address/Swap range/Fidelity Bond/Actions gate whether a router is usable at all, so they stay
// visible; the raw Base/Liquidity/Time fee breakdown is a detail tucked behind the expand toggle.
const ROUTER_TABLE_GRID = {
  collapsed: "grid-cols-[minmax(150px,1.4fr)_repeat(2,minmax(90px,0.85fr))_minmax(122px,0.9fr)_minmax(250px,max-content)]",
  expanded:
    "grid-cols-[minmax(150px,1.35fr)_repeat(5,minmax(74px,0.78fr))_minmax(122px,0.9fr)_minmax(250px,max-content)]",
};

// Fewer columns (collapsed) means more room per column, so text can run larger; each range is also
// fluid via clamp()'s vw term, so resizing the window scales it smoothly instead of at breakpoints.
const ROUTER_HEADER_TEXT = {
  collapsed: "text-[clamp(9px,0.55vw,11px)]",
  expanded: "text-[clamp(8px,0.45vw,9.5px)]",
};
const ROUTER_ROW_TEXT = {
  collapsed: "text-[clamp(11px,1vw,14px)]",
  expanded: "text-[clamp(9px,0.75vw,11px)]",
};
// One height for both modes — sized for collapsed's larger text — so toggling the expand button
// doesn't jump row height, only the column layout and font size change.
const ROUTER_ROW_HEIGHT = "min-h-[clamp(46px,4vw,58px)]";

function SortHeader({
  label,
  sortKey,
  active,
  dir,
  onSort,
  className = "",
}: {
  label: React.ReactNode;
  sortKey: SortKey;
  active: SortKey | null;
  dir: SortDir;
  onSort: (key: SortKey) => void;
  className?: string;
}) {
  const isActive = active === sortKey;
  return (
    <button
      type="button"
      onClick={() => onSort(sortKey)}
      className={`group relative flex items-center justify-end gap-1 transition-colors ${isActive ? "text-primary" : "hover:text-foreground"} ${className}`}
    >
      {label}
      {isActive ? (
        dir === "asc" ? <ArrowUp size={11} strokeWidth={2.5} /> : <ArrowDown size={11} strokeWidth={2.5} />
      ) : (
        <ArrowUpDown size={11} strokeWidth={2} className="opacity-40" />
      )}
    </button>
  );
}

function TooltipButton({
  onClick,
  disabled,
  danger,
  tooltip,
  children,
}: {
  onClick: () => void;
  disabled?: boolean;
  danger?: boolean;
  tooltip: string;
  children: React.ReactNode;
}) {
  return (
    <Tooltip content={tooltip} align="right">
      <button
        type="button"
        onClick={onClick}
        disabled={disabled}
        className={`min-h-7 whitespace-nowrap rounded-full border px-2.5 text-[11.5px] font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-55 ${
          danger
            ? "border-line text-muted hover:border-danger/52 hover:bg-danger/10 hover:text-danger"
            : "border-line text-muted hover:border-line-strong hover:bg-[var(--color-hover)] hover:text-foreground"
        }`}
      >
        {children}
      </button>
    </Tooltip>
  );
}

function FidelityBondModal({ router, onClose }: { router: Router; onClose: () => void }) {
  const bond = router.offer!;
  return (
    <Modal title="Fidelity Bond Details" onClose={onClose} footer={<Button variant="secondary" onClick={onClose}>Close</Button>}>
      <div className="grid grid-cols-2 gap-3">
        <div className="rounded-xl border border-line p-3.5">
          <span className="mb-2 block text-[11px] text-subtle">Tor Address</span>
          <strong className="break-all font-mono text-[13px] text-foreground">{router.address}</strong>
        </div>
        <div className="rounded-xl border border-line p-3.5">
          <span className="mb-2 block text-[11px] text-subtle">Bond Amount</span>
          <strong className="font-mono text-[13px] text-foreground">
            <SatsAmount sats={bond.bondAmountSats} />
          </strong>
        </div>
        <div className="rounded-xl border border-line p-3.5">
          <span className="mb-2 block text-[11px] text-subtle">Bond Status</span>
          <strong className={`font-mono text-[13px] ${bond.bondIsSpent ? "text-danger" : "text-success"}`}>
            {bond.bondIsSpent ? "Spent" : "Active"}
          </strong>
        </div>
        <div className="rounded-xl border border-line p-3.5">
          <span className="mb-2 block text-[11px] text-subtle">Unlocks At</span>
          <strong className="font-mono text-[13px] text-foreground">Block {bond.bondLocktimeHeight.toLocaleString()}</strong>
        </div>
        <div className="col-span-2 rounded-xl border border-line p-3.5">
          <span className="mb-2 block text-[11px] text-subtle">Bond Txid</span>
          <button
            type="button"
            onClick={() => void openExternal(explorerTxUrl(bond.bondTxid))}
            className="break-all text-left font-mono text-[13px] text-primary hover:text-primary-hover"
          >
            {bond.bondTxid}:{bond.bondVout}
          </button>
        </div>
      </div>
    </Modal>
  );
}

function FeeCalculatorModal({ router, onClose }: { router: Router; onClose: () => void }) {
  const offer = router.offer!;
  const [amount, setAmount] = useState(offer.minSize > 0 ? offer.minSize : Math.min(10_000_000, offer.maxSize || 10_000_000));
  const [position, setPosition] = useState(1);
  const [totalRouters, setTotalRouters] = useState(2);

  const invalid = !Number.isInteger(position) || !Number.isInteger(totalRouters) || position < 1 || totalRouters < 1 || position > totalRouters;
  const estimate = invalid
    ? null
    : estimateRouterFee({
        baseFee: offer.baseFee,
        amountRelativeFeePct: offer.amountRelativeFeePct,
        timeRelativeFeePct: offer.timeRelativeFeePct,
        amountSats: amount,
        routerPosition: position,
        totalRouters,
      });
  const totalPercent = estimate && amount > 0 ? (estimate.totalFee / amount) * 100 : 0;

  return (
    <Modal
      title="Estimate swap cost"
      onClose={onClose}
      footer={<Button variant="secondary" onClick={onClose}>Close</Button>}
    >
      <p className="truncate font-mono text-[11px] text-muted" title={router.address}>
        {formatTorEndpoint(router.address, 22, 12, true)}
      </p>

      <label className="flex flex-col gap-2">
        <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Swap Amount</span>
        <input
          type="number"
          min={0}
          value={amount}
          onChange={(e) => setAmount(Math.max(0, Math.round(Number(e.target.value))))}
          className="h-[46px] rounded-card border border-line-strong bg-surface-raised px-3 font-mono text-[13.5px] font-medium text-foreground outline-none focus:border-primary/65 focus:shadow-ring"
        />
      </label>
      <div className="mt-1.5 flex items-center justify-between font-mono text-[11px] text-subtle">
        <span>Router range</span>
        <strong className="font-extrabold text-muted">
          <SatsAmount sats={offer.minSize} /> – <SatsAmount sats={offer.maxSize} />
        </strong>
      </div>

      <label className="mt-4 flex flex-col gap-2">
        <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Router Position in Circuit (n)</span>
        <input
          type="number"
          min={1}
          value={position}
          onChange={(e) => setPosition(Number(e.target.value))}
          className="h-[46px] rounded-card border border-line-strong bg-surface-raised px-3 font-mono text-[13.5px] font-medium text-foreground outline-none focus:border-primary/65 focus:shadow-ring"
        />
      </label>
      <label className="mt-4 flex flex-col gap-2">
        <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Total Routers in Swap (m)</span>
        <input
          type="number"
          min={1}
          value={totalRouters}
          onChange={(e) => setTotalRouters(Number(e.target.value))}
          className="h-[46px] rounded-card border border-line-strong bg-surface-raised px-3 font-mono text-[13.5px] font-medium text-foreground outline-none focus:border-primary/65 focus:shadow-ring"
        />
      </label>
      <div className="mt-1.5 flex items-center justify-between font-mono text-[11px] text-subtle">
        <span>Refund locktime = 20 x (m - n + 1)</span>
        <strong className="font-extrabold text-muted">{estimate ? `${estimate.refundLocktime} blocks` : "Enter position"}</strong>
      </div>
      {invalid && (
        <p className="mt-1.5 text-[11px] text-danger">Enter positive router counts where n is not greater than m.</p>
      )}

      <div className="mt-4 rounded-card border border-primary/28 bg-primary/[0.08] p-3.5 font-mono text-[12px] font-bold leading-relaxed text-foreground">
        <span className="mb-1 block text-primary">Formula</span>
        <strong>Total Fee</strong> = Base Fee + (Swap Amount x Liquidity Fee) + (Refund Locktime x Swap Amount x Time Rate)
      </div>

      <div className="mt-4 rounded-xl border border-line-strong bg-surface-raised p-3.5">
        <div className="grid grid-cols-[1fr_auto] gap-x-3.5 gap-y-1 border-b border-dashed border-white/10 py-2">
          <span className="font-mono text-[10px] text-subtle">Base Fee</span>
          <strong className="font-mono text-[14px] font-extrabold text-foreground">
            <SatsAmount sats={estimate?.baseFee ?? 0} />
          </strong>
        </div>
        <div className="grid grid-cols-[1fr_auto] gap-x-3.5 gap-y-1 border-b border-dashed border-white/10 py-2">
          <span className="font-mono text-[10px] text-subtle">Liquidity Fee</span>
          <strong className="font-mono text-[14px] font-extrabold text-foreground">
            <SatsAmount sats={estimate?.liquidityFee ?? 0} />
          </strong>
        </div>
        <div className="grid grid-cols-[1fr_auto] gap-x-3.5 gap-y-1 border-b border-dashed border-white/10 py-2">
          <span className="font-mono text-[10px] text-subtle">Time Fee</span>
          <strong className="font-mono text-[14px] font-extrabold text-foreground">
            <SatsAmount sats={estimate?.timeFee ?? 0} />
          </strong>
        </div>
        <div className="grid grid-cols-[1fr_auto] gap-x-3.5 gap-y-1 pt-3">
          <span className="font-mono text-[10px] text-subtle">Total Fee</span>
          <strong className="font-mono text-[19px] font-extrabold text-primary">
            <SatsAmount sats={estimate?.totalFee ?? 0} />
          </strong>
          <small className="col-span-2 font-mono text-[10px] text-subtle">
            {estimate ? `${totalPercent.toFixed(4)} % of swap amount` : "Enter position to calculate total fee"}
          </small>
        </div>
      </div>

      <p className="mt-3.5 font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
        Estimates exclude on-chain miner fees.
      </p>
    </Modal>
  );
}

function ConfirmRemoveModal({ address, onConfirm, onCancel, removing }: { address: string; onConfirm: () => void; onCancel: () => void; removing: boolean }) {
  return (
    <Modal
      title="Remove router?"
      onClose={onCancel}
      footer={
        <>
          <Button variant="secondary" onClick={onCancel} disabled={removing}>Cancel</Button>
          <Button onClick={onConfirm} loading={removing}>Remove</Button>
        </>
      }
    >
      <p className="break-all text-[13px] text-muted">
        Remove <span className="font-mono text-foreground">{address}</span> from the offerbook? It will no longer
        appear in market results until rediscovered.
      </p>
    </Modal>
  );
}

export function MarketPage() {
  const [good, setGood] = useState<Router[]>([]);
  const [bad, setBad] = useState<Router[]>([]);
  const [unresponsive, setUnresponsive] = useState<Router[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [tab, setTab] = useState<RouterStatus>("good");
  const [pollingAddress, setPollingAddress] = useState<string | null>(null);
  const [removeTarget, setRemoveTarget] = useState<string | null>(null);
  const [removing, setRemoving] = useState(false);
  const [feeCalcRouter, setFeeCalcRouter] = useState<Router | null>(null);
  const [bondRouter, setBondRouter] = useState<Router | null>(null);
  const [showAllColumns, setShowAllColumns] = useState(false);
  const [footerTick, setFooterTick] = useState(0);
  const [sortKey, setSortKey] = useState<SortKey | null>(null);
  const [sortDir, setSortDir] = useState<SortDir>("desc");
  const pollIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const pushToast = useToastStore((s) => s.push);
  const pushFailure = useToastStore((s) => s.pushFailure);

  const applyOfferBook = useCallback((view: { good: Router[]; bad: Router[]; unresponsive: Router[] }) => {
    setGood(view.good);
    setBad(view.bad);
    setUnresponsive(view.unresponsive);
  }, []);

  const load = useCallback(async () => {
    const view = await getOffers();
    applyOfferBook(view);
    return view;
  }, [applyOfferBook]);

  useEffect(() => {
    void (async () => {
      try {
        const view = await load();
        // A sync can already be running here — e.g. the one kicked off at app startup right
        // after init_taker — with no button click of ours to hang a loading state off of.
        // Reflect it anyway by polling until it clears, same cadence as our own refresh().
        if (view.syncing) {
          setRefreshing(true);
          pollIntervalRef.current = setInterval(() => {
            void load()
              .then((v) => {
                if (!v.syncing && pollIntervalRef.current) {
                  clearInterval(pollIntervalRef.current);
                  pollIntervalRef.current = null;
                  setRefreshing(false);
                  setFooterTick((t) => t + 1);
                }
              })
              .catch(() => {});
          }, 2000);
        }
      } catch (e) {
        // `get_offers` needs the taker, and a running swap holds it for hours. Arriving here
        // mid-swap is expected and there is nothing to act on, so it stays quiet — the same
        // rule the Swap page follows. A user-initiated Refresh below still reports it.
        if (!isAppError(e) || e.code !== "SWAP_IN_PROGRESS") {
          pushFailure(e, "Failed to load routers.");
        }
      } finally {
        setLoading(false);
      }
    })();
  }, [load, pushFailure]);

  // Separate from pollIntervalRef, which tracks a sync to completion and then stops. This one
  // runs for as long as the page is open. Failures are swallowed: get_offers takes the wallet
  // lock without blocking, so it simply fails for the duration of a running swap.
  useEffect(() => {
    const id = setInterval(() => void load().catch(() => {}), OFFERBOOK_REREAD_MS);
    return () => clearInterval(id);
  }, [load]);

  const refresh = useCallback(async () => {
    if (pollIntervalRef.current) clearInterval(pollIntervalRef.current);
    setRefreshing(true);
    pollIntervalRef.current = setInterval(() => {
      void load().catch(() => {});
    }, 2000);
    try {
      await syncOfferbook();
      await load();
      setFooterTick((t) => t + 1);
    } catch (e) {
      pushFailure(e, "Sync failed.");
    } finally {
      if (pollIntervalRef.current) clearInterval(pollIntervalRef.current);
      pollIntervalRef.current = null;
      setRefreshing(false);
    }
  }, [load, pushFailure]);

  useEffect(() => () => {
    if (pollIntervalRef.current) clearInterval(pollIntervalRef.current);
  }, []);

  const stats = useMemo(() => {
    const totalLiquidity = good.reduce((sum, m) => sum + (m.offer?.maxSize ?? 0), 0);
    const totalFidelity = good.reduce((sum, m) => sum + (m.offer?.bondAmountSats ?? 0), 0);
    return { totalLiquidity, totalFidelity };
  }, [good]);

  const buckets: Record<RouterStatus, Router[]> = { good, bad, unresponsive };
  const displayed = buckets[tab];

  function toggleSort(key: SortKey) {
    if (sortKey === key) {
      setSortDir((d) => (d === "asc" ? "desc" : "asc"));
    } else {
      setSortKey(key);
      setSortDir("desc");
    }
  }

  const rows = useMemo(() => {
    if (!sortKey) return displayed;

    const valueOf = ({ offer }: Router): number => {
      switch (sortKey) {
        case "baseFee":
          return offer?.baseFee ?? 0;
        case "liquidityFee":
          return offer?.amountRelativeFeePct ?? 0;
        case "timeRate":
          return offer?.timeRelativeFeePct ?? 0;
        case "minSwap":
          return offer?.minSize ?? 0;
        case "maxSwap":
          return offer?.maxSize ?? 0;
        case "bond":
          return offer?.bondAmountSats ?? 0;
      }
    };

    const sorted = [...displayed].sort((a, b) => valueOf(a) - valueOf(b));
    if (sortDir === "desc") sorted.reverse();
    return sorted;
  }, [displayed, sortKey, sortDir]);

  async function poll(address: string) {
    if (pollingAddress) return;
    setPollingAddress(address);
    try {
      const fresh = await pollRouter(address);
      setGood((g) => g.filter((m) => m.address !== address));
      setBad((b) => b.filter((m) => m.address !== address));
      setUnresponsive((u) => u.filter((m) => m.address !== address));
      if (fresh.state === "good") setGood((g) => [...g, fresh]);
      else if (fresh.state === "bad") setBad((b) => [...b, fresh]);
      else setUnresponsive((u) => [...u, fresh]);

      if (fresh.state === "good") {
        pushToast("success", "Router responded with a fresh offer. Offerbook updated.");
      } else {
        pushToast("error", "Router did not respond with a usable fresh offer.");
      }
    } catch (e) {
      pushFailure(e, "Poll failed.");
    } finally {
      setPollingAddress(null);
    }
  }

  async function confirmRemove() {
    if (!removeTarget) return;
    setRemoving(true);
    try {
      await removeRouter(removeTarget);
      await load();
      setRemoveTarget(null);
    } catch (e) {
      pushFailure(e, "Remove failed.");
    } finally {
      setRemoving(false);
    }
  }

  return (
    <div className="h-full overflow-y-auto p-8">
      <header className="flex shrink-0 items-start justify-between gap-4">
        <div>
          <h1 className="font-header text-[26px] font-bold leading-none text-foreground">Market</h1>
          <p className="mt-1.5 max-w-lg text-[13px] text-muted">
            Live view of Portal routers routing through your Tor circuit.
          </p>
        </div>
        <Button
          onClick={() => void refresh()}
          disabled={refreshing}
        >
          <RefreshCw size={15} strokeWidth={2} className={refreshing ? "animate-spin" : ""} />
          {refreshing ? "Refreshing..." : "Refresh"}
        </Button>
      </header>

      {(loading || refreshing) && (
        <div className="mt-5 shrink-0 rounded-card border border-primary/35 bg-primary/10 px-4.5 py-3.5">
          <div className="mb-3 flex items-center justify-between font-mono text-[11px] uppercase tracking-widest text-primary-hover">
            <span className="flex items-center gap-2">
              <RefreshCw size={16} strokeWidth={2} className="animate-spin" />
              Syncing market data...
            </span>
            <span className="text-subtle">Please wait</span>
          </div>
          <div className="mb-3"><IndeterminateBar /></div>
          <div className="flex items-center gap-2 font-mono text-[11px] uppercase tracking-widest text-primary-hover">
            <Search size={14} strokeWidth={2} />
            Discovering routers over Tor network...
          </div>
        </div>
      )}

      <StatStrip
        className="mt-5 shrink-0"
        items={[
          {
            label: "Fidelity locked",
            value: <SatsAmount sats={stats.totalFidelity} />,
            detail: `across ${good.length} active routers`,
          },
          {
            label: "Total liquidity",
            value: <SatsAmount sats={stats.totalLiquidity} />,
            detail: "spendable router depth",
            tone: "primary",
          },
          {
            label: "Active routers",
            value: String(good.length),
            detail: `${bad.length} bad · ${unresponsive.length} unresponsive`,
            tone: good.length > 0 ? "success" : "foreground",
          },
        ]}
      />

      <Card className="mt-3 flex min-h-[min(52vh,470px)] max-h-[min(68vh,680px)] flex-col border-line-strong">
        <div className="flex shrink-0 items-center justify-between gap-3.5 border-b border-line px-4.5 py-4">
          <div className="inline-flex items-center gap-1 rounded-full p-1">
            {(
              [
                ["good", "Good Routers", good.length],
                ["bad", "Bad Routers", bad.length],
                ["unresponsive", "Unresponsive", unresponsive.length],
              ] as [RouterStatus, string, number][]
            ).map(([value, label, count]) => (
              <button
                key={value}
                type="button"
                onClick={() => setTab(value)}
                className={`relative min-h-[30px] whitespace-nowrap rounded-full px-3.5 text-[11.5px] font-medium transition-colors ${
                  tab === value ? STATUS_TAB_CLASS[value].text : "text-muted hover:text-foreground"
                }`}
              >
                {tab === value && (
                  <motion.span
                    layoutId="tabglow-market-status"
                    transition={{ type: "spring", stiffness: 420, damping: 34, mass: 0.6 }}
                    className={`absolute inset-0 -z-10 rounded-full ${STATUS_TAB_CLASS[value].glow}`}
                  />
                )}
                {label}{" "}
                <span className={`ml-1 font-mono text-[10px] ${tab === value ? STATUS_TAB_CLASS[value].text : "text-subtle"}`}>
                  {count}
                </span>
              </button>
            ))}
          </div>
          <div className="flex items-center gap-3">
            <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">{displayed.length} {tab} offers</span>
            <button
              type="button"
              onClick={() => setShowAllColumns((v) => !v)}
              className="flex items-center gap-1 font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle transition-colors hover:text-foreground"
            >
              <ChevronDown size={12} strokeWidth={2.5} className={`transition-transform ${showAllColumns ? "rotate-180" : ""}`} />
              {showAllColumns ? "Show less" : "Show all columns"}
            </button>
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden">
          <div
            className={`sticky top-0 z-1 grid ${
              showAllColumns ? ROUTER_TABLE_GRID.expanded : ROUTER_TABLE_GRID.collapsed
            } gap-3 bg-surface-raised px-7.5 pb-2.5 pt-3.5 font-mono ${
              showAllColumns ? ROUTER_HEADER_TEXT.expanded : ROUTER_HEADER_TEXT.collapsed
            } uppercase tracking-widest text-subtle`}
          >
            <div>Tor Address</div>
            {showAllColumns && (
              <SortHeader label="Base Fee" sortKey="baseFee" active={sortKey} dir={sortDir} onSort={toggleSort} />
            )}
            {showAllColumns && (
              <SortHeader label="Liquidity Fee" sortKey="liquidityFee" active={sortKey} dir={sortDir} onSort={toggleSort} />
            )}
            {showAllColumns && (
              <SortHeader label="Time Rate" sortKey="timeRate" active={sortKey} dir={sortDir} onSort={toggleSort} />
            )}
            <SortHeader label="Min Swap" sortKey="minSwap" active={sortKey} dir={sortDir} onSort={toggleSort} />
            <SortHeader label="Max Swap" sortKey="maxSwap" active={sortKey} dir={sortDir} onSort={toggleSort} />
            <SortHeader label="Fidelity Bond" sortKey="bond" active={sortKey} dir={sortDir} onSort={toggleSort} />
            <div className="text-right">Actions</div>
          </div>

          <div className="flex flex-col divide-y divide-line px-4.5 pb-3 font-numeric tabular-nums">
            {loading ? (
              <div className="grid min-h-[220px] place-items-center gap-2.5 text-center text-[13px] text-subtle">
                <RefreshCw size={42} strokeWidth={1.6} className="animate-spin text-primary" />
                <strong className="text-[15px] text-foreground">Syncing market data...</strong>
                <span>Fetching routers over Tor network</span>
              </div>
            ) : good.length + bad.length + unresponsive.length === 0 ? (
              <div className="grid min-h-[220px] place-items-center gap-2.5 text-center text-[13px] text-subtle">
                <Inbox size={42} strokeWidth={1.6} className="text-primary" />
                <strong className="text-[15px] text-foreground">No routers found</strong>
                <Button size="sm" onClick={() => void refresh()}>
                  <RefreshCw size={14} strokeWidth={2} /> Refresh
                </Button>
              </div>
            ) : displayed.length === 0 ? (
              <div className="grid min-h-[220px] place-items-center gap-2.5 text-center text-[13px] text-subtle">
                <Inbox size={42} strokeWidth={1.6} className="text-primary" />
                <strong className="text-[15px] text-foreground">No {tab} routers found</strong>
              </div>
            ) : (
              rows.map((router) => {
                const offer = router.offer;
                const isPolling = pollingAddress === router.address;
                return (
                  <div
                    key={router.address}
                    className={`grid ${ROUTER_ROW_HEIGHT} ${
                      showAllColumns ? ROUTER_TABLE_GRID.expanded : ROUTER_TABLE_GRID.collapsed
                    } items-center gap-3 px-3 py-2.5 font-mono ${
                      showAllColumns ? ROUTER_ROW_TEXT.expanded : ROUTER_ROW_TEXT.collapsed
                    } transition-colors hover:bg-[var(--color-hover)]`}
                  >
                    <div className="truncate text-muted" title={router.address}>
                      {formatTorEndpoint(router.address, 8, 6, true)}
                    </div>
                    {showAllColumns && (
                      <div className="text-right font-semibold text-primary">
                        {(offer?.baseFee ?? 0).toLocaleString()}
                      </div>
                    )}
                    {showAllColumns && (
                      <div className="text-right font-semibold text-foreground">
                        {(offer?.amountRelativeFeePct ?? 0).toFixed(3)}
                      </div>
                    )}
                    {showAllColumns && (
                      <div className="text-right font-semibold text-foreground">
                        {(offer?.timeRelativeFeePct ?? 0).toFixed(4)}
                      </div>
                    )}
                    <div className="text-right font-semibold text-subtle">
                      {(offer?.minSize ?? 0).toLocaleString()}
                    </div>
                    <div className="text-right font-semibold text-subtle">
                      {(offer?.maxSize ?? 0).toLocaleString()}
                    </div>
                    <div className="flex items-center justify-end gap-2 font-semibold text-foreground">
                      <span>{offer && offer.bondAmountSats > 0 ? offer.bondAmountSats.toLocaleString() : "N/A"}</span>
                      {offer && offer.bondAmountSats > 0 && (
                        <button
                          type="button"
                          title="View fidelity bond"
                          onClick={() => setBondRouter(router)}
                          className="grid h-[18px] w-[18px] place-items-center rounded text-subtle hover:bg-primary/10 hover:text-primary"
                        >
                          <ExternalLink size={12} strokeWidth={2} />
                        </button>
                      )}
                    </div>
                    <div className="flex justify-end gap-2">
                      <TooltipButton
                        tooltip="Calculate the estimated router fee for this router using your amount and hop position."
                        onClick={() => setFeeCalcRouter(router)}
                        disabled={!offer}
                      >
                        Calculate
                      </TooltipButton>
                      {router.state !== "good" && (
                        <TooltipButton
                          tooltip="Ask this router for a fresh offer now and update its availability and fee data."
                          onClick={() => void poll(router.address)}
                          disabled={isPolling}
                        >
                          {isPolling ? "Polling..." : "Poll"}
                        </TooltipButton>
                      )}
                      <TooltipButton
                        tooltip="Remove this router from your local offerbook so it no longer appears in market results."
                        onClick={() => setRemoveTarget(router.address)}
                        danger
                      >
                        Remove
                      </TooltipButton>
                    </div>
                  </div>
                );
              })
            )}
          </div>
        </div>

        <div className="flex min-h-[54px] shrink-0 items-center justify-end border-t border-line px-4.5 py-3 font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
          Showing {displayed.length} {tab} offers · {new Date().toLocaleTimeString("en-US", { hour: "2-digit", minute: "2-digit", second: "2-digit" })}
          <span className="hidden">{footerTick}</span>
        </div>
      </Card>

      {feeCalcRouter && <FeeCalculatorModal router={feeCalcRouter} onClose={() => setFeeCalcRouter(null)} />}
      {bondRouter && <FidelityBondModal router={bondRouter} onClose={() => setBondRouter(null)} />}
      {removeTarget && (
        <ConfirmRemoveModal
          address={removeTarget}
          removing={removing}
          onConfirm={() => void confirmRemove()}
          onCancel={() => setRemoveTarget(null)}
        />
      )}

    </div>
  );
}
