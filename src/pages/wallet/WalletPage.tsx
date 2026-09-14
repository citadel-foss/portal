import { ArrowDownLeft, ArrowDownToLine, ArrowUpRight, Clock } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { openExternal } from "../../platform";
import {
  Card,
  ExternalLinkButton,
  SatsAmount,
  SkeletonRows,
  StatStrip,
  StatusChip,
} from "../../components/ui/display";
import { LinkButton, SegmentedToggle, SortToggle } from "../../components/ui/inputs";
import { hydrateWalletCache, refreshWalletCache } from "../../lib/wallet-sync";
import { WalletFooterCard } from "./WalletBackupCard";
import { useHeaderActionsStore } from "../../store/header-actions";
import { sendConfirmations, usePendingSendsStore } from "../../store/pending-sends";
import { useWalletCacheStore } from "../../store/wallet-cache";
import {
  classifySpendType,
  explorerTxUrl,
  formatRelativeTime,
  getTransactionKind,
  scriptTypeFromAddress,
  truncateMiddle,
} from "../../lib/wallet-format";

type UtxoFilter = "all" | "regular" | "contract" | "swap";
type TxFilter = "all" | "received" | "sent" | "swap";
type TxSortKey = "newest" | "amount";
type SortDir = "asc" | "desc";

const SCRIPT_PILL_CLASS: Record<string, string> = {
  Taproot: "text-router border-router/35 bg-router/10",
  SegWit: "text-primary border-primary/35 bg-primary/[0.12]",
};

const TYPE_PILL_CLASS: Record<string, string> = {
  Swap: "text-primary border-primary/35 bg-primary/[0.12]",
  Regular: "text-muted border-line bg-white/[0.03]",
  Contract: "text-warning border-warning/35 bg-warning/10",
  Fidelity: "text-warning border-warning/35 bg-warning/10",
};

function Pill({ label, className }: { label: string; className: string }) {
  return (
    <span className={`w-fit rounded-control border px-2 py-0.5 font-mono text-[9.5px] ${className}`}>{label}</span>
  );
}

export function WalletPage() {
  const [, updateClock] = useState(0);
  const info = useWalletCacheStore((s) => s.info);
  const balances = useWalletCacheStore((s) => s.balances);
  const utxos = useWalletCacheStore((s) => s.utxos);
  const transactions = useWalletCacheStore((s) => s.transactions);
  const syncStatus = useWalletCacheStore((s) => s.syncStatus);
  const syncError = useWalletCacheStore((s) => s.syncError);
  const lastSuccessfulSyncAt = useWalletCacheStore((s) => s.lastSuccessfulSyncAt);
  const historyStatus = useWalletCacheStore((s) => s.historyStatus);
  const allPendingSends = usePendingSendsStore((s) => s.sends);
  const reconcileSends = usePendingSendsStore((s) => s.reconcile);
  const walletPath = info?.walletPath ?? null;
  const pendingSends = useMemo(
    () => allPendingSends.filter((x) => x.walletPath === walletPath),
    [allPendingSends, walletPath],
  );
  const historyError = useWalletCacheStore((s) => s.historyError);

  const refreshing = syncStatus === "syncing";
  // "idle" counts as pending too: the first fetch is queued but has not started, and calling
  // that settled would flash "no transactions" before a single row has been read.
  const historyPending =
    historyStatus === "loading" || historyStatus === "idle" || refreshing;
  // A process-local snapshot can paint immediately. On a fresh process, only
  // wait for the persisted wallet data read, never for Electrum synchronization.
  const [initialLoading, setInitialLoading] = useState(() => useWalletCacheStore.getState().balances === null);
  const [initialError, setInitialError] = useState<string | null>(null);

  const [utxoFilter, setUtxoFilter] = useState<UtxoFilter>("all");
  const [txFilter, setTxFilter] = useState<TxFilter>("all");
  const [txSort, setTxSort] = useState<TxSortKey>("newest");
  const [sortDir, setSortDir] = useState<Record<TxSortKey, SortDir>>({ newest: "desc", amount: "desc" });

  const loadStored = useCallback(hydrateWalletCache, []);

  const refresh = useCallback(async () => {
    await refreshWalletCache();
  }, []);

  // Hydrate the encrypted wallet file's last saved UTXO snapshot first. Once it
  // is visible, refresh it from Electrum without blocking the wallet screen.
  useEffect(() => {
    if (useWalletCacheStore.getState().balances !== null) {
      void refresh();
      return;
    }
    void loadStored().then(
      () => {
        setInitialLoading(false);
        void refresh();
      },
      (error: unknown) => {
        setInitialLoading(false);
        setInitialError((error as { message?: string })?.message ?? "Could not read the saved wallet data.");
      },
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    useHeaderActionsStore.getState().register(() => void refresh());
    return () => useHeaderActionsStore.getState().register(null);
  }, [refresh]);

  // A recorded send stops being pending once the chain agrees: either an output of it is
  // confirmed, or it turned up in the wallet's own history.
  useEffect(() => {
    if (!walletPath) return;
    reconcileSends(
      walletPath,
      utxos,
      transactions.map((tx) => tx.txid),
    );
  }, [walletPath, utxos, transactions, reconcileSends]);

  useEffect(() => {
    useHeaderActionsStore.getState().setRefreshing(refreshing);
  }, [refreshing]);

  useEffect(() => {
    const id = setInterval(() => updateClock((value) => value + 1), 30_000);
    return () => clearInterval(id);
  }, []);

  const utxoCounts = useMemo(() => {
    const counts = { all: utxos.length, regular: 0, contract: 0, swap: 0 };
    for (const u of utxos) {
      const bucket = classifySpendType(u.spendType);
      if (bucket === "Regular") counts.regular++;
      else if (bucket === "Contract" || bucket === "Fidelity") counts.contract++;
      else if (bucket === "Swap") counts.swap++;
    }
    return counts;
  }, [utxos]);

  const filteredUtxos = useMemo(() => {
    if (utxoFilter === "all") return utxos;
    return utxos.filter((u) => {
      const bucket = classifySpendType(u.spendType);
      if (utxoFilter === "regular") return bucket === "Regular";
      if (utxoFilter === "contract") return bucket === "Contract" || bucket === "Fidelity";
      return bucket === "Swap";
    });
  }, [utxos, utxoFilter]);

  const filteredTx = useMemo(() => {
    let rows = txFilter === "all" ? transactions : transactions.filter((tx) => getTransactionKind(tx.category, tx.label, tx.amountSats) === txFilter);
    rows = [...rows];
    const dir = sortDir[txSort] === "asc" ? 1 : -1;
    if (txSort === "amount") {
      rows.sort((a, b) => (Math.abs(a.amountSats) - Math.abs(b.amountSats)) * dir);
    } else {
      rows.sort((a, b) => (a.time - b.time) * dir);
    }
    return rows;
  }, [transactions, txFilter, txSort, sortDir]);

  function toggleSort(key: TxSortKey) {
    if (key === txSort) {
      setSortDir((prev) => ({ ...prev, [key]: prev[key] === "desc" ? "asc" : "desc" }));
    } else {
      setTxSort(key);
    }
  }

  const totalBalance = (balances?.regular ?? 0) + (balances?.swap ?? 0);
  const lastSyncLabel = lastSuccessfulSyncAt
    ? formatRelativeTime(Math.floor(lastSuccessfulSyncAt / 1000))
    : "never";

  if (initialLoading) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-4">
        <span className="font-header text-[13px] uppercase tracking-widest text-muted">Loading wallet…</span>
        <span className="relative h-[2px] w-48 overflow-hidden rounded-pill bg-line">
          {/* transform-origin belongs to the keyframes, which flip it mid-cycle. */}
          <span className="absolute inset-y-0 left-0 w-full animate-[status-fill_1.9s_ease-in-out_infinite] bg-success shadow-[0_0_8px_color-mix(in_oklab,var(--color-success)_70%,transparent)]" />
        </span>
      </div>
    );
  }

  if (initialError || balances === null || info === null) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 px-8 text-center">
        <span className="font-header text-[13px] uppercase tracking-widest text-danger">Wallet data unavailable</span>
        <p className="max-w-md text-[13px] text-muted">{initialError ?? "Could not read the saved wallet snapshot."}</p>
        <button
          type="button"
          className="rounded-control border border-line-strong px-4 py-2 text-[12px] text-foreground hover:bg-[var(--color-hover)]"
          onClick={() => {
            setInitialError(null);
            setInitialLoading(true);
            void loadStored().then(
              () => {
                setInitialLoading(false);
                void refresh();
              },
              (error: unknown) => {
                setInitialLoading(false);
                setInitialError((error as { message?: string })?.message ?? "Could not read the saved wallet data.");
              },
            );
          }}
        >
          Try again
        </button>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto px-8 pb-8 pt-2">
      <div className="flex shrink-0 items-center gap-2 text-[13px] text-subtle">
        <span
          className={`h-[7px] w-[7px] rounded-full ${
            refreshing
              ? "animate-pulse bg-warning shadow-[0_0_8px_color-mix(in_oklab,var(--color-warning)_70%,transparent)]"
              : syncStatus === "error"
                ? "bg-danger shadow-[0_0_8px_color-mix(in_oklab,var(--color-danger)_70%,transparent)]"
                : "bg-success shadow-[0_0_8px_color-mix(in_oklab,var(--color-success)_70%,transparent)]"
          }`}
        />
        <span title={syncError ?? undefined}>
          {refreshing
            ? `Last synced ${lastSyncLabel} · Updating…`
            : syncStatus === "error"
              ? `Sync unavailable · Last synced ${lastSyncLabel}`
              : `Last synced ${lastSyncLabel}`}
        </span>
        <span>·</span>
        <span className="font-mono uppercase">{info?.walletName ?? "—"}</span>
      </div>

      {syncStatus === "error" && (
        <div className="mt-3 shrink-0 rounded-control border border-warning/35 bg-warning/[0.08] px-3.5 py-2.5 text-[12px] text-warning">
          {syncError ?? "Wallet synchronization failed."} Saved balances remain visible, but sending and swaps are disabled.
        </div>
      )}

      {/* A freshly initialized wallet has nothing to show and no obvious next step; the empty
          tables below read as a broken dashboard without this. Disappears on the first deposit. */}
      {totalBalance === 0 && transactions.length === 0 && syncStatus !== "error" && (
        <div className="mt-3 flex shrink-0 flex-wrap items-center justify-between gap-3 rounded-control border border-primary/35 bg-primary/[0.06] px-4 py-3">
          <span className="flex items-center gap-2.5">
            <ArrowDownToLine size={16} strokeWidth={1.9} className="flex-none text-primary" />
            <span className="text-[12.5px] text-muted">
              <strong className="font-semibold text-foreground">This wallet is empty.</strong> Receive
              bitcoin into it before starting a swap.
            </span>
          </span>
          <LinkButton to="/send" size="sm">
            Receive bitcoin
          </LinkButton>
        </div>
      )}

      <StatStrip
        className="mt-5 shrink-0"
        items={[
          {
            label: "Total balance",
            value: <SatsAmount sats={totalBalance} />,
            detail: `≈ ${(totalBalance / 1e8).toFixed(8)} BTC`,
            tone: "primary",
          },
          { label: "Swaps", value: <SatsAmount sats={balances?.swap ?? 0} />, detail: "received by swap txs" },
          { label: "Regular", value: <SatsAmount sats={balances?.regular ?? 0} />, detail: "received by regular txs" },
          { label: "Contracts", value: <SatsAmount sats={balances?.contract ?? 0} />, detail: "held in a swap contract" },
        ]}
      />

      <section className="mt-3 grid grid-cols-[minmax(0,1.46fr)_minmax(410px,1fr)] gap-3">
        <Card className="flex min-h-[min(52vh,470px)] max-h-[min(68vh,680px)] flex-col border-line-strong">
          <header className="flex items-baseline gap-3 border-b border-line px-4.5 py-4">
            <h3 className="font-header text-[15px] font-bold text-foreground">UTXOs</h3>
            <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
              {utxoCounts.all} unspent
            </span>
          </header>
          <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-hidden p-3.5">
            <SegmentedToggle
              groupId="utxo-filter"
              value={utxoFilter}
              onChange={setUtxoFilter}
              options={[
                { value: "all", label: "All", suffix: <span>{utxoCounts.all}</span> },
                { value: "regular", label: "Regular", suffix: <span>{utxoCounts.regular}</span> },
                { value: "contract", label: "Contract", suffix: <span>{utxoCounts.contract}</span> },
                { value: "swap", label: "Swap", suffix: <span>{utxoCounts.swap}</span> },
              ]}
            />
            <div className="grid grid-cols-[1.35fr_0.58fr_0.58fr_1.1fr_52px] gap-3 px-3 font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
              <span>Txid . Amount</span>
              <span>Script</span>
              <span>Type</span>
              <span>Address</span>
              <span />
            </div>
            <div className="flex flex-1 flex-col divide-y divide-line overflow-y-auto">
              {filteredUtxos.length === 0 && (
                <p className="px-3 py-6 text-center text-[13px] text-subtle">No UTXOs match this filter.</p>
              )}
              {filteredUtxos.map((u) => {
                const bucket = classifySpendType(u.spendType);
                const script = scriptTypeFromAddress(u.address);
                return (
                  <div
                    key={`${u.txid}:${u.vout}`}
                    className="grid min-h-[58px] grid-cols-[1.35fr_0.58fr_0.58fr_1.1fr_52px] items-center gap-3 px-3 py-2.5 transition-colors duration-200 hover:bg-[var(--color-hover)]"
                  >
                    <span className="flex min-w-0 flex-col gap-1">
                      <span className="truncate font-mono text-[12px] text-muted">
                        {truncateMiddle(u.txid, 12, 4)}:{u.vout}
                      </span>
                      <SatsAmount sats={u.amountSats} className="text-[13px] font-semibold text-success" />
                    </span>
                    <Pill label={script.toUpperCase()} className={SCRIPT_PILL_CLASS[script]} />
                    <Pill label={bucket.toUpperCase()} className={TYPE_PILL_CLASS[bucket]} />
                    <span className="truncate font-mono text-[11.5px] text-muted">{u.address ?? "No address"}</span>
                    <ExternalLinkButton txid={u.txid} />
                  </div>
                );
              })}
            </div>
          </div>
        </Card>

        <Card className="flex min-h-[min(52vh,470px)] max-h-[min(68vh,680px)] flex-col border-line-strong">
          <header className="flex flex-wrap items-start justify-between gap-3 border-b border-line px-4.5 py-4">
            <div className="flex items-baseline gap-3">
              <h3 className="font-header text-[15px] font-bold text-foreground">Recent transactions</h3>
              <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
                {transactions.length + pendingSends.length} total
              </span>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <SegmentedToggle
                groupId="tx-filter"
                value={txFilter}
                onChange={setTxFilter}
                options={[
                  { value: "all", label: "All" },
                  { value: "received", label: "Received" },
                  { value: "sent", label: "Sent" },
                  { value: "swap", label: "Swaps" },
                ]}
              />
              <SortToggle groupId="tx-sort" sortKey={txSort} sortDir={sortDir} onChange={toggleSort} options={[{ key: "newest", label: "Newest" }, { key: "amount", label: "Amount" }]} />
            </div>
          </header>
          <div className="flex min-h-0 flex-1 flex-col divide-y divide-line overflow-y-auto px-3.5">
            {/* Above the history, and outside the filters, because these are the transactions the
                user just made and most wants to see land. */}
            {pendingSends.map((send) => {
              const confirmations = sendConfirmations(send.txid, utxos);
              return (
                <div key={send.txid} className="flex items-center gap-3 px-3 py-3">
                  <StatusChip tone="warning" shape="tile" className="h-[34px] w-[34px] justify-center px-0">
                    <Clock size={16} strokeWidth={2} />
                  </StatusChip>
                  <span className="flex min-w-0 flex-1 flex-col gap-1">
                    <span className="truncate font-mono text-[12px] text-muted" title={send.txid}>
                      {truncateMiddle(send.txid, 12, 8)}
                    </span>
                    <StatusChip tone="warning" className="self-start">
                      {confirmations === null
                        ? "Broadcast — waiting for the mempool"
                        : confirmations === 0
                          ? "In the mempool — waiting for a block"
                          : `${confirmations} confirmation${confirmations === 1 ? "" : "s"}`}
                    </StatusChip>
                  </span>
                  <span className="flex flex-none items-center gap-2">
                    <span className="font-numeric text-[12.5px] text-danger">
                      −<SatsAmount sats={send.amountSats} />
                    </span>
                    <ExternalLinkButton txid={send.txid} />
                  </span>
                </div>
              );
            })}
            {/* Placeholders only while there is nothing to show yet. A refresh over existing
                rows keeps them: swapping real history for skeletons on every poll would read
                as the list emptying and refilling. */}
            {filteredTx.length === 0 && pendingSends.length === 0 && historyPending && (
              <SkeletonRows count={4} />
            )}
            {filteredTx.length === 0 && pendingSends.length === 0 && !historyPending && (
              <p className="px-3 py-6 text-center text-[13px] text-subtle">
                {historyStatus === "error"
                  ? `Transaction history unavailable: ${historyError ?? "Electrum query failed."}`
                  : syncStatus === "error"
                    ? "Transaction history unavailable until wallet sync succeeds."
                    : transactions.length === 0
                      ? "No wallet transactions were returned."
                      : "No transactions match the selected filter."}
              </p>
            )}
            {filteredTx.map((tx) => {
              const isReceive = tx.amountSats >= 0;
              return (
                <div
                  key={`${tx.txid}:${tx.category}:${tx.address ?? ""}:${tx.amountSats}`}
                  role="button"
                  tabIndex={0}
                  onClick={() => void openExternal(explorerTxUrl(tx.txid))}
                  onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && openExternal(explorerTxUrl(tx.txid))}
                  className="grid min-h-[58px] cursor-pointer grid-cols-[38px_minmax(0,1fr)_auto_52px] items-center gap-3 px-0 py-2.5 text-left outline-none transition-colors duration-200 hover:bg-[var(--color-hover)] focus-visible:shadow-[inset_0_0_0_1px_color-mix(in_oklab,var(--color-primary)_45%,transparent)]"
                >
                  <span
                    className={`flex h-[34px] w-[34px] items-center justify-center rounded-control border ${
                      isReceive
                        ? "border-success/45 bg-success/[0.08] text-success"
                        : "border-danger/45 bg-danger/[0.08] text-danger"
                    }`}
                  >
                    {isReceive ? <ArrowDownLeft size={20} strokeWidth={2} /> : <ArrowUpRight size={20} strokeWidth={2} />}
                  </span>
                  <span className="flex min-w-0 flex-col gap-1">
                    <span className="truncate font-mono text-[12px] text-muted">{truncateMiddle(tx.txid, 16, 8)}</span>
                    <span className="flex items-center gap-1.5">
                      <span
                        className={`w-fit rounded-control border px-1.5 py-0.5 font-mono text-[9.5px] tracking-wide ${
                          tx.confirmations >= 6
                            ? "border-success/32 bg-success/10 text-success"
                            : "border-warning/35 bg-warning/10 text-warning"
                        }`}
                      >
                        {Math.min(tx.confirmations, 6)}/6 CONF
                      </span>
                    </span>
                  </span>
                  <span className="flex flex-col items-end gap-1">
                    <SatsAmount
                      sats={Math.abs(tx.amountSats)}
                      className={`text-[13px] font-semibold ${isReceive ? "text-success" : "text-danger"}`}
                    />
                    <span className="font-mono text-[10.5px] text-subtle">{formatRelativeTime(tx.time)}</span>
                  </span>
                  <ExternalLinkButton txid={tx.txid} />
                </div>
              );
            })}
          </div>
        </Card>
      </section>

      <section className="mt-4">
        <WalletFooterCard />
      </section>
    </div>
  );
}
