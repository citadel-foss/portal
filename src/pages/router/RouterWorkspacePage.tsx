import {
  AlertTriangle,
  Check,
  ChevronDown,
  CircleDollarSign,
  Copy,
  LockKeyhole,
  Play,
  RefreshCw,
  Save,
  Square,
  Trash2,
  WalletCards,
  ShieldCheck,
} from "lucide-react";
import QRCode from "qrcode";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Link,
  useNavigate,
  useParams,
  useSearchParams,
} from "react-router-dom";
import {
  checkRouterConfig,
  checkTor,
  getRouterDefaults,
  clearRouterSettings,
  getBtcPrice,
  getRouterBalances,
  getRouterInfo,
  getRouterLogs,
  getRouterNewAddress,
  listRouterAddresses,
  getRouterStatus,
  getSavedRouterSettings,
  listRouterFidelityBonds,
  listRouterSwapReports,
  startRouter,
  stopRouter,
  sendRouterToAddress,
  syncRouterWallet,
  updateRouterSettings,
} from "../../api/commands";
import { chosenFeeRate, type FeeChoice, useFeeEstimate } from "../../lib/fee-rate";
import { BackupForm } from "../wallet/WalletBackupCard";
import { AddressList } from "../../components/app/AddressList";
import { isAppError } from "../../api/types";
import type {
  AddressType,
  Balances,
  FidelityBond,
  RouterSettings,
  RouterStatus,
  RouterSwapReportSummary,
  UtxoEntry,
  WalletAddress,
  WalletInfo,
} from "../../api/types";
import {
  BackButton,
  Card,
  ExternalLinkButton,
  IconButton,
  Identifier,
  Modal,
  SatsAmount,
  SettingsSection,
  SkeletonLines,
} from "../../components/ui/display";
import {
  Button,
  FeeRateField,
  LinkButton,
  PasswordField,
  TextField,
  SegmentedToggle,
  SummaryGroup,
  SummaryRow,
} from "../../components/ui/inputs";
import { copyText } from "../../lib/clipboard";
import {
  formatNumber,
  formatRelativeTime,
  formatUnitAmount,
  type Unit,
  useUnitAmount,
} from "../../lib/wallet-format";
import { useToastStore } from "../../store/toast";
import { LogPanel } from "../../components/app/LogPanel";
import { FaucetButton } from "../../components/app/FaucetButton";
import { routerNameError } from "./router-defaults";
import { formatTimestamp } from "../../components/ui/report";
import { useHeaderActionsStore } from "../../store/header-actions";
import {
  refreshRouterWallet,
  useRouterWalletCacheStore,
  useRouterWalletSnapshot,
} from "../../store/router-wallet-cache";

type Tab = "overview" | "wallet" | "logs" | "settings";
const TAB_OPTIONS: { value: Tab; label: string }[] = [
  { value: "overview", label: "Overview" },
  { value: "wallet", label: "Tx" },
  { value: "logs", label: "Logs" },
  { value: "settings", label: "Settings" },
];

function DataMetric({
  label,
  value,
  detail,
  icon,
  tone = "text-foreground",
}: {
  label: string;
  value: React.ReactNode;
  detail: string;
  icon: React.ReactNode;
  tone?: string;
}) {
  const accent = tone.includes("success")
    ? "bg-success"
    : tone.includes("warning")
      ? "bg-warning"
      : "bg-primary";
  return (
    <Card className="group min-h-[138px] border-line-strong p-5 transition-colors duration-200 hover:border-primary/30">
      <div className={`absolute inset-x-5 top-0 h-px ${accent} opacity-55`} />
      <div className="flex items-center justify-between gap-3">
        <span className="font-mono text-[10px] uppercase tracking-[0.18em] text-subtle">{label}</span>
        <span className={`grid h-7 w-7 place-items-center rounded-control border border-line bg-surface/65 ${tone}`}>{icon}</span>
      </div>
      <strong className={`mt-3 block font-mono text-[24px] tracking-tight ${tone}`}>
        {value}
      </strong>
      <span className="mt-2 block text-[11px] text-muted">{detail}</span>
    </Card>
  );
}

function OverviewPanel({
  status,
  settings,
  info,
  balances,
  bonds,
  reports,
}: {
  status: RouterStatus;
  settings: RouterSettings;
  info: WalletInfo;
  balances: Balances | null;
  bonds: FidelityBond[];
  reports: RouterSwapReportSummary[];
}) {
  const [copied, setCopied] = useState(false);
  const tor = status.torAddress;
  const total = balances
    ? balances.regular + balances.swap + balances.contract + balances.fidelity
    : 0;
  const earnings = reports.reduce(
    (sum, report) => sum + report.feeEarnedSats,
    0,
  );
  return (
    <div className="space-y-4">
      {balances ? (
        <div className="grid grid-cols-[1.45fr_repeat(2,1fr)] gap-4 max-[1000px]:grid-cols-2 max-[680px]:grid-cols-1">
          <Card className="row-span-2 border-primary/30 p-6 shadow-[inset_0_1px_0_rgba(255,255,255,0.12),0_18px_55px_-38px_color-mix(in_oklab,var(--color-primary)_65%,transparent)] max-[1000px]:col-span-2 max-[680px]:col-span-1">
            <div className="flex items-center justify-between">
              <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Total router balance</span>
              <span className="grid h-8 w-8 place-items-center rounded-control border border-primary/25 bg-primary/10 text-primary"><WalletCards size={15} /></span>
            </div>
            <strong className="mt-4 block font-mono text-[38px] text-primary">
              <SatsAmount sats={total} />
            </strong>
            <p className="mt-2 text-[12px] text-muted">
              Regular + swap + contract + fidelity
            </p>
            <div
              className="mt-6 grid grid-cols-2 overflow-hidden rounded-control border border-line bg-line
                [&>*]:bg-surface/75 [&>*]:p-3 [&>*:nth-child(odd)]:mr-px [&>*:nth-child(-n+2)]:mb-px"
            >
              <span className="text-[11px] text-muted">
                Regular{" "}
                <strong className="mt-1 block font-mono text-foreground">
                  <SatsAmount sats={balances.regular} />
                </strong>
              </span>
              <span className="text-[11px] text-muted">
                Swap{" "}
                <strong className="mt-1 block font-mono text-success">
                  <SatsAmount sats={balances.swap} />
                </strong>
              </span>
              <span className="text-[11px] text-muted">
                Contract{" "}
                <strong className="mt-1 block font-mono text-warning">
                  <SatsAmount sats={balances.contract} />
                </strong>
              </span>
              <span className="text-[11px] text-muted">
                Fidelity{" "}
                <strong className="mt-1 block font-mono text-warning">
                  <SatsAmount sats={balances.fidelity} />
                </strong>
              </span>
            </div>
          </Card>
          <DataMetric
            label="Spendable"
            value={<SatsAmount sats={balances.spendable} />}
            detail="Regular and swap funds available"
            icon={<WalletCards size={14} />}
            tone="text-primary"
          />
          <DataMetric
            label="Net earnings"
            value={<SatsAmount sats={earnings} />}
            detail={`${reports.length} saved swap reports`}
            icon={<CircleDollarSign size={14} />}
            tone="text-success"
          />
          <DataMetric
            label="Fidelity bonds"
            value={bonds.filter((bond) => bond.isLocked).length}
            detail={`${bonds.length} bond records`}
            icon={<ShieldCheck size={14} />}
            tone="text-warning"
          />
          <DataMetric
            label="Contract balance"
            value={<SatsAmount sats={balances.contract} />}
            detail="Funds currently locked in contracts"
            icon={<LockKeyhole size={14} />}
            tone="text-warning"
          />
        </div>
      ) : (
        <Card className="grid min-h-[220px] place-items-center border-dashed border-line-strong p-8 text-center">
          <div>
            <WalletCards size={34} className="mx-auto text-primary" />
            <strong className="mt-3 block text-[14px]">
              Start the router to load wallet data
            </strong>
            <span className="mt-1 block text-[12px] text-muted">
              Saved configuration remains available while the runtime is
              stopped.
            </span>
          </div>
        </Card>
      )}
      <Card className="border-line-strong">
        <div className="flex items-center justify-between border-b border-line px-5 py-4">
          <h2 className="font-header text-[14px] font-bold">Runtime configuration</h2>
          <span className="flex items-center gap-2 font-mono text-[9px] uppercase tracking-widest text-subtle">
            <i className={`h-1.5 w-1.5 rounded-full ${status.running ? "bg-success" : "bg-subtle"}`} /> {status.phase.phase}
          </span>
        </div>
        {/* The first column is sized to its content so the Tor address sits on one line; the
            others share what is left, where the data directory already truncates. */}
        <div className="grid grid-cols-[minmax(0,max-content)_minmax(0,1fr)_minmax(0,1fr)] gap-px bg-line max-[800px]:grid-cols-1">
          {[
            {
              label: "Tor address",
              // In full, never truncated: this is the value an operator copies out to check
              // their router is reachable. Wraps only when the window is too narrow for it.
              wrap: true,
              value: tor ? (
                <button
                  type="button"
                  title="Copy Tor address"
                  onClick={() =>
                    void copyText(tor).then((ok) => {
                      if (!ok) return;
                      setCopied(true);
                      setTimeout(() => setCopied(false), 1200);
                    })
                  }
                  className="group flex max-w-full items-start gap-2 rounded-sm text-left outline-none hover:text-primary focus-visible:shadow-ring"
                >
                  <span className="break-all">{tor}</span>
                  <Copy
                    size={12}
                    className={`mt-0.5 flex-none ${copied ? "text-success" : "text-subtle group-hover:text-primary"}`}
                  />
                </button>
              ) : (
                <span className="text-subtle">Available once the router has started</span>
              ),
            },
            {
              label: "Data directory",
              // Empty on the web host, which keeps server paths to itself.
              value: info.dataDir || "Default",
              title: info.dataDir || undefined,
            },
            {
              label: "Ports",
              value: `${settings.networkPort} / ${settings.rpcPort}`,
            },
            {
              label: "Tor",
              value: `${settings.socksPort} / ${settings.controlPort}`,
            },
            { label: "Status", value: status.phase.phase },
          ].map(({ label, value, title, wrap }: { label: string; value: React.ReactNode; title?: string; wrap?: boolean }) => (
            <div key={label} className="min-w-0 bg-surface/80 p-4 transition-colors duration-200 hover:bg-white/[0.035]">
              <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
                {label}
              </span>
              <strong
                className={`mt-1.5 block font-mono text-[11.5px] text-foreground ${wrap ? "" : "truncate"}`}
                title={title}
              >
                {value}
              </strong>
            </div>
          ))}
        </div>
      </Card>
      <ReportList routerId={settings.routerId} reports={reports} />
    </div>
  );
}

function ReportList({
  routerId,
  reports,
}: {
  routerId: string;
  reports: RouterSwapReportSummary[];
}) {
  return (
    <Card className="border-line-strong">
      <div className="flex items-center justify-between border-b border-line px-5 py-4">
        <h2 className="font-header text-[14px] font-bold">Swap reports</h2>
        <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
          {reports.length} reports
        </span>
      </div>
      {reports.length === 0 ? (
        <p className="p-8 text-center text-[12px] text-subtle">
          Completed router swaps will appear here.
        </p>
      ) : (
        <div className="divide-y divide-line">
          {reports.slice(0, 10).map((report) => (
            <Link
              key={report.swapId}
              to={`/router/${encodeURIComponent(routerId)}/report/${encodeURIComponent(report.swapId)}`}
              className="grid grid-cols-[1fr_auto_auto] items-center gap-5 px-5 py-3.5 hover:bg-[var(--color-hover)]"
            >
              <div className="min-w-0">
                <Identifier value={report.swapId} className="block text-[11.5px] font-semibold leading-[1.45]" />
                <span className="mt-1 block text-[10px] text-subtle">
                  {formatRelativeTime(report.endTimestamp)}
                </span>
              </div>
              <span className="rounded-pill border border-line px-2 py-1 font-mono text-[9px] uppercase text-muted">
                {report.status}
              </span>
              <strong className="font-mono text-[12px] text-success">
                +<SatsAmount sats={report.feeEarnedSats} />
              </strong>
            </Link>
          ))}
        </div>
      )}
    </Card>
  );
}

/**
 * Spending out of a router's wallet.
 *
 * Deliberately the plain half of the taker's Send page — recipient, amount, fee rate. A router
 * wallet is not where anyone composes a careful payment; it is where fees accumulate and
 * occasionally need to leave, and before this the only way out was to stop the router and open
 * its file somewhere else.
 */
function RouterSendPanel({
  routerId,
  utxos,
  onSent,
}: {
  routerId: string;
  utxos: UtxoEntry[];
  onSent: () => Promise<void>;
}) {
  const pushToast = useToastStore((state) => state.push);
  const [recipient, setRecipient] = useState("");
  const { fees, failed: feesFailed, retry: retryFees } = useFeeEstimate();
  const [feeChoice, setFeeChoice] = useState<FeeChoice>("medium");
  const [customFeeRate, setCustomFeeRate] = useState("");
  const [sending, setSending] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [btcPrice, setBtcPrice] = useState<number | null>(null);
  const { unit, input: amount, setInput: setAmount, changeUnit, sats: amountSats } =
    useUnitAmount(btcPrice);
  const [btcPriceCached, setBtcPriceCached] = useState(false);

  // Best-effort, as on the wallet's Send: without a price only the USD option is unavailable.
  useEffect(() => {
    void getBtcPrice()
      .then((p) => {
        setBtcPrice(p.usd);
        setBtcPriceCached(p.cached);
      })
      .catch(() => setBtcPrice(null));
  }, []);

  const spendable = utxos
    .filter((u) => u.spendable && u.solvable)
    .reduce((sum, u) => sum + u.amountSats, 0);
  const otherUnits = (["sats", "btc", "usd"] as Unit[]).filter((u) => u !== unit);
  const rate = chosenFeeRate(fees, feeChoice, customFeeRate);
  const blocked =
    recipient.trim().length === 0 || amountSats <= 0 || amountSats > spendable || rate <= 0;

  async function send() {
    setConfirming(false);
    setSending(true);
    try {
      const { txid } = await sendRouterToAddress(routerId, recipient.trim(), amountSats, rate);
      setRecipient("");
      setAmount("");
      pushToast("success", `Sent. Transaction ${txid}`);
      await onSent();
    } catch (e) {
      pushToast("error", (e as { message?: string })?.message ?? "Could not send from this router.");
    } finally {
      setSending(false);
    }
  }

  return (
    <Card className="border-line-strong p-5">
      <div className="flex items-center justify-between gap-4">
        <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
          Send Bitcoin
        </span>
        <span className="text-[11px] text-subtle">
          Spendable: <SatsAmount sats={spendable} className="text-foreground" />
        </span>
      </div>
      <div className="mt-4 flex flex-col gap-3">
        <TextField
          label="Recipient address"
          placeholder="bc1… or tb1…"
          value={recipient}
          onChange={(e) => setRecipient(e.target.value)}
          autoComplete="off"
          spellCheck={false}
        />
        <div className="grid grid-cols-[1fr_auto] items-end gap-3">
          <TextField
            label="Amount"
            inputMode="decimal"
            placeholder="0"
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
            error={amountSats > spendable ? "More than this wallet holds." : undefined}
          />
          <SegmentedToggle
            groupId="router-send-unit"
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
        {amountSats > 0 && (
          <div className="-mt-1.5 flex items-center justify-between px-1 text-[11px] text-subtle">
            <span>{formatUnitAmount(amountSats, otherUnits[0], btcPrice) ?? "—"}</span>
            <span>{formatUnitAmount(amountSats, otherUnits[1], btcPrice) ?? "—"}</span>
          </div>
        )}
        <div className="flex flex-col gap-2">
          <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Fee rate</span>
          <FeeRateField
            fees={fees}
            failed={feesFailed}
            onRetry={retryFees}
            choice={feeChoice}
            onChoice={setFeeChoice}
            custom={customFeeRate}
            onCustom={setCustomFeeRate}
          />
        </div>
        <Button className="w-full" disabled={blocked} loading={sending} onClick={() => setConfirming(true)}>
          Send
        </Button>
      </div>
      {confirming && (
        <Modal
          title="Confirm this payment"
          onClose={() => setConfirming(false)}
          footer={
            <>
              <Button variant="secondary" onClick={() => setConfirming(false)}>
                Cancel
              </Button>
              <Button onClick={() => void send()} loading={sending}>
                Broadcast
              </Button>
            </>
          }
        >
          <div className="flex flex-col gap-2.5 rounded-control border border-line bg-surface-raised px-3.5 py-3">
            <span className="flex flex-col gap-1">
              <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">To</span>
              <span className="break-all font-mono text-[12px] text-foreground">{recipient.trim()}</span>
            </span>
            <span className="flex items-baseline justify-between gap-3 border-t border-line pt-2.5">
              <span className="text-[12px] text-muted">Amount</span>
              <strong className="font-numeric text-[13.5px] text-foreground">
                <SatsAmount sats={amountSats} />
              </strong>
            </span>
            <span className="flex items-baseline justify-between gap-3">
              <span className="text-[12px] text-muted">Fee rate</span>
              <span className="font-numeric text-[12.5px] text-foreground">{rate} s/vB</span>
            </span>
          </div>
          {(fees?.fast ? rate > fees.fast * 3 : rate > 100) && (
            <p className="text-[11.5px] leading-5 text-warning">
              {fees?.fast
                ? `This rate is ${Math.round(rate / fees.fast)}× the chain server's fast estimate.`
                : `${rate} s/vB is far above a normal rate.`}{" "}
              Check it before broadcasting.
            </p>
          )}
          <p className="text-[11.5px] leading-5 text-subtle">Broadcasting cannot be undone.</p>
        </Modal>
      )}
    </Card>
  );
}

// Module-level, not a ref: the Tx tab remounts on every visit, and a request still in flight
// from the last one must still block a duplicate, or the backend derives two addresses for one.
const addressRequests = new Set<string>();

function WalletPanel({
  routerId,
  running,
}: {
  routerId: string;
  running: boolean;
}) {
  const { utxos, transactions, addresses } = useRouterWalletSnapshot(routerId);
  const [addressesOpen, setAddressesOpen] = useState(false);
  const [walletAddresses, setWalletAddresses] = useState<WalletAddress[] | null>(null);
  const [addressType, setAddressType] = useState<AddressType>("p2tr");
  const [addressError, setAddressError] = useState<string | null>(null);
  const address = addresses[addressType] ?? null;
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const pushToast = useToastStore((state) => state.push);
  const utxoKey = JSON.stringify(utxos
    .map((u) => [u.txid, u.vout, u.amountSats, u.confirmations > 0, u.address, u.derivationPath])
    .sort());
  const addressKey = JSON.stringify(addresses);
  // Re-read when balances, confirmation status or issued addresses change, not every poll.
  useEffect(() => {
    if (!addressesOpen || !running) return;
    let live = true;
    void listRouterAddresses(routerId)
      .then((list) => live && setWalletAddresses(list))
      .catch((e) => {
        if (!live) return;
        setWalletAddresses([]);
        pushToast("error", (e as { message?: string })?.message ?? "Failed to list addresses.");
      });
    return () => {
      live = false;
    };
  }, [addressesOpen, running, routerId, utxoKey, addressKey, pushToast]);
  useEffect(() => {
    setQrDataUrl(null);
    if (!address) return;
    let cancelled = false;
    void QRCode.toDataURL(address.address, { width: 184, margin: 1 }).then((url) => {
      if (!cancelled) setQrDataUrl(url);
    });
    return () => {
      cancelled = true;
    };
  }, [address]);

  // Both backends list oldest-first. Sorted on first sight, newest on top; the backend's own
  // order breaks ties, reversed, so rows seen in the same second still read newest-first.
  const newestFirst = useMemo(
    () =>
      transactions
        .map((tx, i) => ({ tx, i }))
        .sort((a, b) => (b.tx.firstSeen ?? b.tx.time) - (a.tx.firstSeen ?? a.tx.time) || b.i - a.i)
        .map(({ tx }) => tx),
    [transactions],
  );

  // The cached snapshot is already on screen; this only brings it up to date behind it.
  const load = useCallback(async () => {
    if (!running) return;
    await refreshRouterWallet(routerId).catch(() => {});
  }, [routerId, running]);
  // Match the running router's two-minute wallet sync. Re-reading more often cannot reveal a new
  // payment, and resolving Electrum UTXO addresses can otherwise open redundant connections.
  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), 2 * 60 * 1000);
    return () => clearInterval(timer);
  }, [load]);

  // Asked again whenever the transactions reload, since that is when a payment to the address
  // on offer shows up. The backend re-offers the same address until then, so this never burns
  // a derivation index — only a paid address moves it on.
  useEffect(() => {
    const key = `${routerId}:${addressType}`;
    if (!running || addressRequests.has(key)) return;
    addressRequests.add(key);
    const type = addressType;
    void getRouterNewAddress(routerId, type)
      .then((next) => {
        setAddressError(null);
        useRouterWalletCacheStore.getState().setAddress(routerId, type, next);
      })
      .catch((e) => setAddressError(e?.message ?? "Could not get a receive address."))
      .finally(() => addressRequests.delete(key));
  }, [routerId, running, addressType, transactions]);

  if (!running)
    return (
      <Card className="grid min-h-[300px] place-items-center border-dashed border-line-strong text-center">
        <div>
          <WalletCards className="mx-auto text-primary" />
          <strong className="mt-3 block">Router wallet is offline</strong>
          <span className="mt-1 block text-[12px] text-muted">
            Start the router to receive, sync, and inspect UTXOs.
          </span>
        </div>
      </Card>
    );
  return (
    // Fills the height the workspace hands it: Send/Receive keep their size and the two lists
    // share the rest, each scrolling inside its own card instead of the page scrolling.
    <div className="flex min-h-0 flex-1 flex-col gap-4">
      <div className="grid flex-none grid-cols-2 gap-4 max-[900px]:grid-cols-1">
      <RouterSendPanel routerId={routerId} utxos={utxos} onSent={load} />
      <Card className="border-line-strong p-5">
        <div className="flex items-center justify-between gap-4">
          <span className="flex items-center gap-3">
            <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
              Receive Bitcoin
            </span>
            <FaucetButton />
          </span>
          <SegmentedToggle
            groupId="router-address-type"
            value={addressType}
            onChange={(next) => {
              setAddressType(next);
              setAddressError(null);
            }}
            options={[
              { value: "p2tr", label: "Taproot" },
              { value: "p2wpkh", label: "SegWit" },
            ]}
          />
        </div>
        <div className="mt-4 flex justify-center">
          {/* The white ground only appears with a QR on it: a bare white square waiting looks
              like a broken image rather than something loading. */}
          <div
            className={`grid h-[212px] w-[212px] place-items-center rounded-card p-3.5 ${
              qrDataUrl
                ? "bg-white shadow-[0_0_0_1px_rgba(255,255,255,0.16)]"
                : "border border-line bg-surface"
            }`}
          >
            {qrDataUrl ? (
              <img src={qrDataUrl} alt="Router receive address QR code" width={184} height={184} />
            ) : addressError ? (
              <span className="px-4 text-center text-[11.5px] leading-5 text-danger">
                {addressError}
              </span>
            ) : (
              <RefreshCw size={24} strokeWidth={1.8} className="animate-spin text-subtle" />
            )}
          </div>
        </div>
        {address && (
          <div className="mt-4 flex items-start justify-between gap-2 rounded-control border border-line bg-surface p-3">
            <span className="min-w-0">
              {/* `select-all` so one click takes the whole address when the clipboard is out
                  of reach and it has to be lifted by hand. */}
              <Identifier
                value={address.address}
                className="block select-all text-[11px] leading-[1.5] text-foreground"
              />
              {address.derivationPath && (
                <span className="mt-1 block font-mono text-[10px] text-subtle">
                  {address.derivationPath}
                </span>
              )}
            </span>
            <IconButton
              label={copied ? "Copied" : "Copy address"}
              onClick={() =>
                void copyText(address.address).then((ok) => {
                  if (!ok) {
                    pushToast("warning", "Could not reach the clipboard — select the address and copy it.");
                    return;
                  }
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1200);
                })
              }
              className={copied ? "text-success" : ""}
              icon={copied ? <Check size={14} strokeWidth={2} /> : <Copy size={14} strokeWidth={1.8} />}
            />
          </div>
        )}
        <button
          type="button"
          aria-expanded={addressesOpen}
          onClick={() => setAddressesOpen((open) => !open)}
          className="mt-4 flex items-center gap-1.5 border-t border-dashed border-line pt-3 font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle hover:text-foreground"
        >
          <ChevronDown size={12} strokeWidth={2.5} />
          Addresses
        </button>
        {addressesOpen && (
          <div className="mt-2">
            <AddressList addresses={walletAddresses} csvName={`${routerId}-addresses.csv`} />
          </div>
        )}
      </Card>
      </div>
      {/* The floor keeps both lists usable on a short window; below it the page scrolls. The
          page's bottom gap lives here, inside the floor, rather than on the scroll container:
          once the page scrolls, the container's own bottom padding is not part of what
          scrolls, and the last card would end flush with the window. */}
      <div className="grid min-h-[344px] flex-1 grid-cols-2 grid-rows-1 gap-4 pb-16 max-[1100px]:min-h-[544px] max-[1100px]:grid-cols-1 max-[1100px]:grid-rows-2">
      <Card className="flex min-h-0 min-w-0 flex-col border-line-strong">
        <div className="flex items-center justify-between gap-4 border-b border-line px-5 py-4">
          <h2 className="font-header text-[14px] font-bold">
            UTXOs{" "}
            <span className="ml-2 font-mono text-[10px] text-subtle">
              {utxos.length}
            </span>
          </h2>
        </div>
        <div className="min-h-0 flex-1 overflow-auto">
          <table className="w-full text-left text-[11px]">
            <thead className="sticky top-0 bg-surface">
              <tr className="text-subtle">
                <th className="px-5 py-3">Address</th>
                <th className="px-5 py-3">Type</th>
                <th className="px-5 py-3">Confirmations</th>
                <th className="px-5 py-3 text-right">Amount</th>
                <th className="w-[52px] px-5 py-3" />
              </tr>
            </thead>
            <tbody className="divide-y divide-line">
              {utxos.map((utxo) => (
                <tr key={`${utxo.txid}:${utxo.vout}`}>
                  <td className="max-w-[300px] px-5 py-3 align-top">
                    <Identifier
                      value={utxo.address ?? `${utxo.txid}:${utxo.vout}`}
                      className="leading-[1.45]"
                    />
                    {utxo.derivationPath && (
                      <span className="mt-1 block font-mono text-[10px] text-subtle">
                        {utxo.derivationPath}
                      </span>
                    )}
                  </td>
                  <td className="px-5 py-3 align-top text-muted">{utxo.spendType}</td>
                  <td className="px-5 py-3 align-top font-mono">{utxo.confirmations}</td>
                  <td className="px-5 py-3 text-right align-top font-mono">
                    <SatsAmount sats={utxo.amountSats} />
                  </td>
                  <td className="px-5 py-3 align-top">
                    {/* The address page, not the funding transaction: this row is a coin, and
                        what you want is everything that ever touched it. */}
                    <ExternalLinkButton address={utxo.address} txid={utxo.txid} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {utxos.length === 0 && (
            <p className="p-8 text-center text-subtle">No UTXOs found.</p>
          )}
        </div>
      </Card>
      <Card className="flex min-h-0 min-w-0 flex-col border-line-strong">
        <div className="border-b border-line px-5 py-4">
          <h2 className="font-header text-[14px] font-bold">
            Recent transactions
          </h2>
        </div>
        {/* All of the fetched window rather than the first 8: the list scrolls now. */}
        <div className="min-h-0 flex-1 divide-y divide-line overflow-auto">
          {newestFirst.map((tx) => (
            <div
              key={`${tx.txid}:${tx.category}`}
              className="flex items-center justify-between gap-4 px-5 py-3"
            >
              <span className="min-w-0">
                <Identifier value={tx.txid} className="block text-[11px] leading-[1.45] text-muted" />
                <span className="mt-1 block font-mono text-[10px] text-subtle">
                  {formatTimestamp(tx.firstSeen ?? tx.time)}
                </span>
              </span>
              <span className="flex flex-none items-center gap-2">
                <strong
                  className={`font-mono text-[11.5px] ${tx.amountSats >= 0 ? "text-success" : "text-danger"}`}
                >
                  <SatsAmount sats={tx.amountSats} />
                </strong>
                <ExternalLinkButton txid={tx.txid} />
              </span>
            </div>
          ))}
          {transactions.length === 0 && (
            <p className="p-8 text-center text-subtle">
              No transactions found.
            </p>
          )}
        </div>
      </Card>
      </div>
    </div>
  );
}

function LogsPanel({ routerId }: { routerId: string }) {
  const load = useCallback((lines: number) => getRouterLogs(routerId, lines), [routerId]);
  return <LogPanel title="Router logs" load={load} className="h-[580px]" />;
}

// Tor's ports are Portal's to choose, not the router's: `build_config` overrides whatever a
// registration carries with the live runtime's pair. They are shown below as status only.
const EDITABLE_SETTING_KEYS = [
  "networkPort",
  "rpcPort",
  "requiredConfirms",
  "baseFee",
  "amountRelativeFeePct",
  "timeRelativeFeePct",
  "fidelityAmount",
  "fidelityTimelock",
  "fidelityFeerate",
] as const;
type EditableSettingKey = (typeof EDITABLE_SETTING_KEYS)[number];
type SettingsForm = Record<EditableSettingKey, string> & { name: string };

function settingsToForm(settings: RouterSettings): SettingsForm {
  return {
    ...(Object.fromEntries(
      EDITABLE_SETTING_KEYS.map((key) => [key, String(settings[key])]),
    ) as Record<EditableSettingKey, string>),
    name: settings.name,
  };
}

function parseSettingsForm(
  settings: RouterSettings,
  form: SettingsForm,
): RouterSettings | string {
  const values = Object.fromEntries(
    EDITABLE_SETTING_KEYS.map((key) => [key, Number(form[key])]),
  ) as Record<EditableSettingKey, number>;
  const integers: EditableSettingKey[] = [
    "networkPort",
    "rpcPort",
    "requiredConfirms",
    "baseFee",
    "fidelityAmount",
    "fidelityTimelock",
  ];
  if (
    EDITABLE_SETTING_KEYS.some(
      (key) => !Number.isFinite(values[key]) || values[key] < 0,
    )
  ) {
    return "All settings must be valid non-negative numbers.";
  }
  if (integers.some((key) => !Number.isSafeInteger(values[key]))) {
    return "Ports, amounts, confirmations, and timelocks must be whole numbers.";
  }
  const ports = [values.networkPort, values.rpcPort];
  if (ports.some((port) => port < 1 || port > 65_535))
    return "Ports must be between 1 and 65535.";
  if (new Set(ports).size !== ports.length)
    return "Network and RPC ports must be different.";
  if (values.requiredConfirms < 1)
    return "Required confirmations must be at least one.";
  if (values.fidelityFeerate < 1)
    return "Fidelity fee rate must be at least 1 sat/vB.";
  const nameError = routerNameError(form.name);
  if (nameError) return `Public name: ${nameError}`;
  return { ...settings, ...values, name: form.name.trim() };
}

function SettingsPanel({
  settings,
  routerId,
  running,
  walletEncrypted,
  transitioning,
  onSaved,
}: {
  settings: RouterSettings;
  routerId: string;
  running: boolean;
  walletEncrypted: boolean;
  transitioning: boolean;
  onSaved: () => Promise<void>;
}) {
  const navigate = useNavigate();
  const pushToast = useToastStore((s) => s.push);
  const [form, setForm] = useState<SettingsForm>(() =>
    settingsToForm(settings),
  );
  const [torPorts, setTorPorts] = useState<{ socksPort: number; controlPort: number } | null>(null);
  const [saving, setSaving] = useState(false);
  const [confirmSave, setConfirmSave] = useState(false);
  const [restartPassword, setRestartPassword] = useState("");
  /** Set once the config is on disk but the router is still down — the dialog then has one
   *  job left, and asking again is all it takes. */
  const [restartPending, setRestartPending] = useState(false);
  const [restartError, setRestartError] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const bondFees = useFeeEstimate();
  const [minBond, setMinBond] = useState<number | null>(null);
  useEffect(() => {
    void getRouterDefaults()
      .then((d) => setMinBond(d.minFidelityAmount ?? null))
      .catch(() => {});
  }, []);
  // Starts on Custom showing the saved rate: opening Settings must not change it.
  const [bondFeeChoice, setBondFeeChoice] = useState<FeeChoice>("custom");
  useEffect(() => setForm(settingsToForm(settings)), [routerId]);
  useEffect(() => {
    void checkTor()
      .then((status) => {
        if (status.socksPort !== undefined && status.controlPort !== undefined) {
          setTorPorts({ socksPort: status.socksPort, controlPort: status.controlPort });
        }
      })
      .catch(() => {});
  }, []);

  const displayName = settings.name || routerId;
  const parsed = parseSettingsForm(settings, form);
  // The crate's own config check, asked as values change: its limits are not public, and
  // otherwise its refusal would only surface when the router next starts.
  const [crateError, setCrateError] = useState<string | null>(null);
  const parsedKey = typeof parsed === "string" ? null : JSON.stringify(parsed);
  useEffect(() => {
    if (parsedKey === null) {
      setCrateError(null);
      return;
    }
    let live = true;
    const timer = setTimeout(() => {
      void checkRouterConfig(JSON.parse(parsedKey) as RouterSettings)
        .then((verdict) => live && setCrateError(verdict))
        .catch(() => live && setCrateError(null));
    }, 400);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [parsedKey]);
  const error = typeof parsed === "string" ? parsed : crateError;
  const dirty =
    form.name !== settings.name ||
    EDITABLE_SETTING_KEYS.some((key) => form[key] !== String(settings[key]));
  const row = (
    key: EditableSettingKey,
    label: string,
    options: {
      suffix?: string;
      hint?: string;
      inputMode?: "numeric" | "decimal";
    } = {},
  ) => (
    <SummaryRow
      label={label}
      value={form[key]}
      suffix={options.suffix}
      hint={options.hint}
      inputMode={options.inputMode}
      readOnly={transitioning || saving}
      onCommit={(value) => setForm((current) => ({ ...current, [key]: value }))}
    />
  );

  /** The start half, on its own: once the config is written this is all that is left, and a
   *  wrong password should cost another attempt rather than the whole dialog. */
  async function start(): Promise<boolean> {
    try {
      await startRouter(routerId, walletEncrypted ? restartPassword : undefined);
      return true;
    } catch (e) {
      setRestartPending(true);
      setRestartError(
        (e as { message?: string }).message ?? "The router did not start.",
      );
      await onSaved().catch(() => {});
      return false;
    }
  }

  async function retryStart() {
    setSaving(true);
    setRestartError(null);
    try {
      if (!(await start())) return;
      setConfirmSave(false);
      setRestartPending(false);
      setRestartPassword("");
      await onSaved();
      pushToast("success", `${displayName} is running again.`);
    } finally {
      setSaving(false);
    }
  }

  /**
   * A running router has to come down to have its config.toml rewritten, so saving is really
   * stop → write → start. Portal drives all three rather than dropping the operator on a
   * stopped router and asking them to remember to bring it back: an unattended router that
   * silently stayed down is a router not earning and a fidelity bond locked for nothing.
   *
   * Which is also why a refused password does not end the dialog. The settings are on disk by
   * then and only the start is outstanding, so the dialog keeps the prompt and asks again.
   */
  async function save(next: RouterSettings) {
    const shouldRestart = running;
    let settingsWereSaved = false;
    setSaving(true);
    setRestartError(null);
    try {
      if (shouldRestart) await stopRouter(routerId);
      const saved = await updateRouterSettings(routerId, next);
      settingsWereSaved = true;
      setForm(settingsToForm(saved));
      // Its own failure, reported as its own thing: the settings are already on disk by
      // here, and saying "could not save" for a restart that did not take would send
      // someone back to re-enter changes that are no longer pending.
      if (shouldRestart && !(await start())) return;
      setConfirmSave(false);
      setRestartPending(false);
      setRestartPassword("");
      await onSaved();
      pushToast(
        "success",
        shouldRestart
          ? "Router settings saved and the router restarted."
          : "Router settings saved to config.toml.",
      );
    } catch (e) {
      await onSaved().catch(() => {});
      const message =
        (e as { message?: string }).message ?? "Could not save router settings.";
      pushToast(
        "error",
        settingsWereSaved
          ? `Settings were saved, but the view could not refresh: ${message}`
          : shouldRestart
            ? `Could not apply settings. The router may be stopped: ${message}`
            : message,
      );
    } finally {
      setSaving(false);
    }
  }

  function requestSave() {
    if (typeof parsed === "string" || crateError !== null || transitioning) return;
    setConfirmSave(true);
  }

  return (
    <div className="space-y-4">
      {transitioning ? (
        <div className="rounded-control border border-warning/35 bg-warning/[0.08] px-4 py-3 text-[12px] text-warning">
          Wait for the current router operation to finish before editing settings.
        </div>
      ) : running ? (
        <div className="rounded-control border border-primary/30 bg-primary/[0.07] px-4 py-3 text-[12px] text-primary">
          Saving changes stops this router and writes its config.toml. Re-enter
          its wallet password afterwards to start it again.
        </div>
      ) : null}
      <Card className="border-line-strong p-5">
        <div className="grid grid-cols-3 gap-4 max-[760px]:grid-cols-1">
          {[
            ["Router ID", settings.routerId],
            ["Wallet", settings.walletName],
            ["Data directory", settings.dataDir ?? "Default"],
          ].map(([label, value]) => (
            <div key={label} className="min-w-0">
              <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
                {label}
              </span>
              <strong
                className="mt-1.5 block truncate font-mono text-[11px]"
                title={value}
              >
                {value}
              </strong>
            </div>
          ))}
        </div>
        <p className="mt-4 border-t border-line pt-3 text-[11px] text-muted">
          Router identity and wallet location are fixed to prevent accidentally
          pointing this router at another wallet.
        </p>
      </Card>
      <div className="grid grid-cols-2 gap-4 max-[900px]:grid-cols-1">
        <SettingsSection
          title="Network"
          subtitle="Inbound router and local RPC listeners"
        >
          <div className="col-span-2 max-[620px]:col-span-1">
            <SummaryGroup title="Listeners">
              {row("networkPort", "Network port", {
                hint: "Keep fixed after bonding",
              })}
              {row("rpcPort", "RPC port")}
              {row("requiredConfirms", "Required confirmations")}
            </SummaryGroup>
          </div>
        </SettingsSection>
        <SettingsSection
          title="Tor"
          subtitle="Shared Tor SOCKS and control service"
        >
          <div className="col-span-2 max-[620px]:col-span-1">
            <SummaryGroup title="Tor ports">
              <SummaryRow
                label="SOCKS port"
                value={torPorts ? String(torPorts.socksPort) : "…"}
                readOnly
                hint="Chosen by Portal's own Tor each launch"
              />
              <SummaryRow
                label="Control port"
                value={torPorts ? String(torPorts.controlPort) : "…"}
                readOnly
              />
            </SummaryGroup>
          </div>
        </SettingsSection>
        <SettingsSection
          title="Swap policy"
          subtitle="What wallets see in this router's offer"
        >
          <div className="col-span-2 max-[620px]:col-span-1">
            <SummaryGroup title="Advertised policy">
              <SummaryRow
                label="Public name"
                value={form.name}
                inputMode="text"
                hint="Shown to wallets. Anyone can claim any name."
                readOnly={transitioning || saving}
                onCommit={(name) => setForm((current) => ({ ...current, name }))}
              />
              {row("baseFee", "Base fee", { suffix: "sats" })}
              {row("amountRelativeFeePct", "Amount-relative fee", {
                suffix: "%",
                inputMode: "decimal",
              })}
              {row("timeRelativeFeePct", "Time-relative fee", {
                suffix: "%",
                inputMode: "decimal",
              })}
            </SummaryGroup>
          </div>
        </SettingsSection>
        <SettingsSection
          title="Fidelity"
          subtitle="Defaults used when creating future fidelity bonds"
        >
          <div className="col-span-2 max-[620px]:col-span-1">
            <SummaryGroup title="Bond defaults">
              {row("fidelityAmount", "Target amount", {
                suffix: "sats",
                hint: minBond !== null ? `Minimum ${formatNumber(minBond)} sats` : undefined,
              })}
              {row("fidelityTimelock", "Timelock", { suffix: "blocks" })}
            </SummaryGroup>
            <div className="mt-4 flex flex-col gap-2">
              <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Bond fee rate</span>
              <FeeRateField
                fees={bondFees.fees}
                failed={bondFees.failed}
                onRetry={bondFees.retry}
                choice={bondFeeChoice}
                onChoice={(choice) => {
                  setBondFeeChoice(choice);
                  const rate = chosenFeeRate(bondFees.fees, choice, form.fidelityFeerate);
                  if (choice !== "custom" && rate > 0)
                    setForm((current) => ({ ...current, fidelityFeerate: String(rate) }));
                }}
                custom={form.fidelityFeerate}
                onCustom={(value) => setForm((current) => ({ ...current, fidelityFeerate: value }))}
              />
            </div>
          </div>
        </SettingsSection>
      </div>
      <Card className="border-line-strong p-5">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <h2 className="font-header text-[14px] font-bold">
              Save configuration
            </h2>
            <p
              className={`mt-1 text-[11.5px] ${error ? "text-danger" : "text-muted"}`}
            >
              {error ??
                (dirty
                  ? running
                    ? "Unsaved changes will be written to config.toml, then the router stops until you start it again."
                    : "Unsaved changes will be written to this router’s config.toml."
                  : "config.toml is up to date.")}
            </p>
          </div>
          <div className="flex gap-2">
            <Button
              variant="secondary"
              disabled={!dirty || saving}
              onClick={() => setForm(settingsToForm(settings))}
            >
              Discard
            </Button>
            <Button
              disabled={transitioning || !dirty || !!error}
              loading={saving}
              onClick={requestSave}
            >
              <Save size={14} />
              {running ? "Save & restart" : "Save changes"}
            </Button>
          </div>
        </div>
      </Card>
      <Card className="border-danger/30 p-5">
        <div className="flex items-center justify-between gap-4">
          <div>
            <h2 className="font-header text-[14px] font-bold text-danger">
              Remove this router
            </h2>
            <p className="mt-1 text-[11.5px] text-muted">
              Stops managing this router here. Wallet files and on-chain funds
              are not deleted.
            </p>
          </div>
          <Button
            variant="secondary"
            disabled={transitioning || running}
            onClick={() => setConfirmRemove(true)}
          >
            <Trash2 size={14} />
            Remove
          </Button>
        </div>
      </Card>
      {confirmSave && typeof parsed !== "string" && (
        <Modal
          title={
            restartPending
              ? `Start ${displayName} again`
              : running
                ? "Save and restart router?"
                : "Save router settings?"
          }
          onClose={() => {
            setConfirmSave(false);
            setRestartPending(false);
            setRestartError(null);
          }}
          footer={
            <>
              <Button
                variant="secondary"
                onClick={() => {
                  setConfirmSave(false);
                  setRestartPending(false);
                  setRestartError(null);
                }}
              >
                {restartPending ? "Leave it stopped" : "Cancel"}
              </Button>
              <Button
                onClick={() => void (restartPending ? retryStart() : save(parsed))}
                loading={saving}
                disabled={
                  (running || restartPending) &&
                  walletEncrypted &&
                  restartPassword.length === 0
                }
              >
                {restartPending ? "Start router" : running ? "Save & restart" : "Save changes"}
              </Button>
            </>
          }
        >
          <div className="space-y-3 text-[12px] leading-5 text-muted">
            <p>
              {restartPending
                ? "Your settings are already written to config.toml. All that is left is starting the router again."
                : running
                  ? "The changes will be written to this router’s config.toml. The router stops while that happens and Portal starts it again."
                  : "The changes will be written to this router’s config.toml."}
            </p>
            {restartError && (
              <p className="rounded-control border border-danger/35 bg-danger/[0.06] px-3 py-2 text-danger">
                {restartError}
              </p>
            )}
            {(running || restartPending) && walletEncrypted && (
              <PasswordField
                label="Router wallet password"
                autoComplete="current-password"
                value={restartPassword}
                onChange={(e) => {
                  setRestartPassword(e.target.value);
                  setRestartError(null);
                }}
                hint="Needed to unlock the wallet again once the config is written."
              />
            )}
            {parsed.networkPort !== settings.networkPort && (
              <div className="flex gap-3 rounded-control border border-warning/30 bg-warning/[0.07] p-3">
                <AlertTriangle className="mt-0.5 shrink-0 text-warning" size={18} />
                <p>
                  A fidelity bond commits to the router address using network port{" "}
                  <strong className="text-foreground">{settings.networkPort}</strong>.
                  Changing it to{" "}
                  <strong className="text-foreground">{parsed.networkPort}</strong>{" "}
                  can make an existing bond unusable for this router. Continue only
                  if there is no active fidelity bond or you understand the migration.
                </p>
              </div>
            )}
          </div>
        </Modal>
      )}
      {confirmRemove && (
        <Modal
          title={`Remove ${displayName}?`}
          onClose={() => setConfirmRemove(false)}
          footer={
            <>
              <Button
                variant="secondary"
                onClick={() => setConfirmRemove(false)}
              >
                Cancel
              </Button>
              <Button
                onClick={() =>
                  void clearRouterSettings(routerId)
                    .then(() => {
                      pushToast("success", `${displayName} was removed.`);
                      navigate("/router");
                    })
                    .catch((e) => pushToast("error", e.message))
                }
              >
                Remove router
              </Button>
            </>
          }
        >
          <p className="text-[12px] leading-5 text-muted">
            This removes <strong className="text-foreground">{displayName}</strong>{" "}
            from the app. Its wallet file and anything on-chain are left untouched:
            to bring it back, add a router with the same router ID and its wallet
            password.
          </p>
        </Modal>
      )}
    </div>
  );
}

export function RouterWorkspacePage() {
  const { routerId = "" } = useParams();
  const id = decodeURIComponent(routerId);
  const pushToast = useToastStore((s) => s.push);
  const [searchParams, setSearchParams] = useSearchParams();
  const requestedTab = searchParams.get("tab") as Tab | null;
  const tab = TAB_OPTIONS.some((item) => item.value === requestedTab)
    ? requestedTab!
    : "overview";
  const [status, setStatus] = useState<RouterStatus | null>(null);
  const [settings, setSettings] = useState<RouterSettings | null>(null);
  const [info, setInfo] = useState<WalletInfo | null>(null);
  const [balances, setBalances] = useState<Balances | null>(null);
  const [bonds, setBonds] = useState<FidelityBond[]>([]);
  const [reports, setReports] = useState<RouterSwapReportSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [actionLoading, setActionLoading] = useState(false);
  const [showStart, setShowStart] = useState(false);
  const [walletPassword, setWalletPassword] = useState("");
  const [startError, setStartError] = useState<string | undefined>();
  const load = useCallback(async () => {
    const [nextStatus, nextSettings, nextInfo] = await Promise.all([
      getRouterStatus(id),
      getSavedRouterSettings(id),
      getRouterInfo(id),
    ]);
    if (!nextSettings) throw new Error("This router was not found.");
    setStatus(nextStatus);
    setSettings(nextSettings);
    setInfo(nextInfo);
    const [b, f, r] = await Promise.allSettled([
      getRouterBalances(id),
      listRouterFidelityBonds(id),
      listRouterSwapReports(id),
    ]);
    setBalances(b.status === "fulfilled" ? b.value : null);
    setBonds(f.status === "fulfilled" ? f.value : []);
    setReports(r.status === "fulfilled" ? r.value : []);
    setLoading(false);
  }, [id]);
  useEffect(() => {
    void load().catch((e) => {
      pushToast("error", e.message);
      setLoading(false);
    });
    const timer = setInterval(() => void load().catch(() => {}), 5000);
    return () => clearInterval(timer);
  }, [load, pushToast]);
  const phase = status?.phase.phase ?? "notConfigured";
  const running = phase === "running" || phase === "starting";

  // The shell's header button, beside Sign out, as on the wallet side. Syncing is what makes a
  // new payment show up at all — the page's 5s poll only re-reads what the wallet already holds
  // — so it lives in the header for every tab, not inside the Tx tab's tables.
  // Its own guard, not the header's `refreshing`: the wallet page can leave that set if it
  // unmounts mid-refresh, which would otherwise block this sync for good.
  const syncing = useRef(false);
  const sync = useCallback(async () => {
    if (syncing.current) return;
    syncing.current = true;
    useHeaderActionsStore.getState().setRefreshing(true);
    try {
      await syncRouterWallet(id);
      await Promise.all([load(), refreshRouterWallet(id)]);
    } catch (e) {
      pushToast("error", (e as { message?: string })?.message ?? "Could not sync the router wallet.");
    } finally {
      syncing.current = false;
      useHeaderActionsStore.getState().setRefreshing(false);
    }
  }, [id, load, pushToast]);
  useEffect(() => {
    if (phase !== "running") return;
    // Whatever page held the header last may have left its spinner on.
    useHeaderActionsStore.getState().setRefreshing(syncing.current);
    useHeaderActionsStore.getState().register(() => void sync());
    return () => useHeaderActionsStore.getState().register(null);
  }, [phase, sync]);
  const transitioning = ["initializing", "starting", "stopping"].includes(
    phase,
  );
  async function start() {
    setActionLoading(true);
    setStartError(undefined);
    try {
      await startRouter(id, walletPassword || undefined);
      setShowStart(false);
      setWalletPassword("");
      await load();
    } catch (e) {
      setStartError(
        isAppError(e) && e.code === "WALLET_WRONG_PASSWORD"
          ? "Wrong password for this router's wallet."
          : ((e as { message?: string }).message ?? "Could not start router."),
      );
    } finally {
      setActionLoading(false);
    }
  }
  async function stop() {
    setActionLoading(true);
    try {
      await stopRouter(id);
      await load();
    } catch (e) {
      pushToast(
        "error",
        (e as { message?: string }).message ?? "Could not stop router.",
      );
    } finally {
      setActionLoading(false);
    }
  }
  // The Tx tab sizes itself to the window so only its two lists scroll. The offline card
  // and every other tab keep the ordinary scrolling page.
  const fitScreen = tab === "wallet" && running;
  if (loading || !status || !settings || !info)
    return (
      <div className="mx-auto w-full max-w-xl pt-20">
        <SkeletonLines count={10} />
      </div>
    );
  return (
    <div className={`h-full overflow-y-auto ${fitScreen ? "flex flex-col px-8 pt-8" : "p-8"}`}>
      <div className={`mx-auto w-full max-w-[1380px] ${fitScreen ? "flex min-h-0 flex-1 flex-col" : "pb-8"}`}>
        <header className="flex flex-wrap items-center justify-between gap-4">
          <div className="flex min-w-0 items-center gap-3">
            <BackButton to="/router" label="Back to routers" />
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h1 className="truncate font-header text-[27px] font-bold">
                  {settings.name || id}
                </h1>
                <span
                  className={`h-2 w-2 rounded-full ${
                    phase === "running"
                      ? "bg-success"
                      : phase === "failed"
                        ? "bg-danger"
                        : transitioning
                          ? "bg-warning"
                          : "bg-subtle"
                  }`}
                />
              </div>
              {settings.name && settings.name !== id && (
                <p className="truncate font-mono text-[11.5px] text-subtle">{id}</p>
              )}
            </div>
          </div>
          <div className="flex gap-2">
            {phase === "starting" && status?.hasBond === false && (
              <LinkButton to={`/router/${encodeURIComponent(id)}/setup`} variant="secondary">
                Continue setup
              </LinkButton>
            )}
            {running ? (
              <Button
                onClick={() => void stop()}
                loading={actionLoading}
              >
                <Square size={13} />
                Stop router
              </Button>
            ) : (
              <Button
                onClick={() => {
                  setWalletPassword("");
                  setStartError(undefined);
                  setShowStart(true);
                }}
                disabled={transitioning}
              >
                <Play size={13} />
                Start router
              </Button>
            )}
          </div>
        </header>
        {phase === "failed" && status.phase.phase === "failed" && (
          <div
            className="mt-4 rounded-control border border-danger/35 bg-danger/[0.08]
              px-4 py-3 text-[12px] text-danger"
          >
            {status.phase.message}
          </div>
        )}
        <div className="mt-6 border-b border-line">
          <SegmentedToggle
            groupId="router-workspace-tabs"
            value={tab}
            onChange={(value) =>
              setSearchParams(value === "overview" ? {} : { tab: value })
            }
            options={TAB_OPTIONS}
          />
        </div>
        <main className={`mt-5 ${fitScreen ? "flex min-h-0 flex-1 flex-col" : ""}`}>
          {tab === "overview" && (
            <div className="flex flex-col gap-4 pb-16">
              <OverviewPanel
                status={status}
                settings={settings}
                info={info}
                balances={balances}
                bonds={bonds}
                reports={reports}
              />
              <Card className="border-line-strong">
                <div className="border-b border-line px-5 py-4">
                  <h2 className="font-header text-[14px] font-bold">Wallet backup</h2>
                  <p className="mt-1 text-[11px] text-muted">An encrypted export of this router&apos;s wallet</p>
                </div>
                <div className="p-5">
                  <BackupForm routerId={id} routerStopped={!running} />
                </div>
              </Card>
            </div>
          )}
          {tab === "wallet" && <WalletPanel routerId={id} running={running} />}
          {tab === "logs" && <LogsPanel routerId={id} />}
          {tab === "settings" && (
            <SettingsPanel
              settings={settings}
              routerId={id}
              running={running}
              // Unknown means the wallet file could not be inspected, and the safe guess is
              // "encrypted": asking for a password that turns out not to be needed costs a
              // keystroke, while not asking for one that is needed fails the restart.
              walletEncrypted={status?.walletEncrypted ?? true}
              transitioning={transitioning}
              onSaved={load}
            />
          )}
        </main>
        {showStart && (
          <Modal
            title={`Start ${id}`}
            onClose={() => setShowStart(false)}
            footer={
              <>
                <Button variant="secondary" onClick={() => setShowStart(false)}>
                  Cancel
                </Button>
                <Button
                  onClick={() => void start()}
                  loading={actionLoading}
                  disabled={status.walletEncrypted === true && !walletPassword}
                >
                  Start router
                </Button>
              </>
            }
          >
            <p className="text-[12px] text-muted">
              Passwords stay in memory for this process and are never written to
              disk.
            </p>
            {status.walletEncrypted !== false && (
              <PasswordField
                label={
                  status.walletEncrypted
                    ? "Wallet password"
                    : "Wallet password (if encrypted)"
                }
                autoComplete="current-password"
                value={walletPassword}
                onChange={(e) => setWalletPassword(e.target.value)}
                error={startError}
                onKeyDown={(e) => e.key === "Enter" && void start()}
              />
            )}
            {startError && status.walletEncrypted === false && (
              <p className="mt-2 text-[12px] text-danger">{startError}</p>
            )}
          </Modal>
        )}
      </div>
    </div>
  );
}
