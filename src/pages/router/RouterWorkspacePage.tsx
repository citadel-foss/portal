import {
  AlertTriangle,
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
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Link,
  useNavigate,
  useParams,
  useSearchParams,
} from "react-router-dom";
import {
  checkTor,
  clearRouterSettings,
  getRouterBalances,
  getRouterInfo,
  getRouterLogs,
  getRouterNewAddress,
  getRouterStatus,
  getRouterTransactions,
  getSavedRouterSettings,
  listRouterFidelityBonds,
  listRouterSwapReports,
  listRouterUtxos,
  startRouter,
  stopRouter,
  sendRouterToAddress,
  syncRouterWallet,
  updateRouterSettings,
} from "../../api/commands";
import { isAppError } from "../../api/types";
import type {
  AddressType,
  Balances,
  FidelityBond,
  LogLine,
  RouterSettings,
  RouterStatus,
  RouterSwapReportSummary,
  NewAddress,
  TxSummary,
  UtxoEntry,
  WalletInfo,
} from "../../api/types";
import {
  BackButton,
  Card,
  ExternalLinkButton,
  IconButton,
  Identifier,
  LogViewer,
  Modal,
  SatsAmount,
  SettingsSection,
  SkeletonLines,
} from "../../components/ui/display";
import {
  Button,
  PasswordField,
  TextField,
  SegmentedToggle,
  SummaryGroup,
  SummaryRow,
} from "../../components/ui/inputs";
import { formatTorEndpoint } from "../../lib/market-format";
import { copyText } from "../../lib/clipboard";
import {
  formatRelativeTime,
  logLevel,
  type LogLevel,
} from "../../lib/wallet-format";
import { useToastStore } from "../../store/toast";

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
        <div className="grid grid-cols-3 gap-px bg-line max-[800px]:grid-cols-1">
          {[
            { label: "Wallet", value: info.walletName, title: info.walletName },
            {
              label: "Data directory",
              value: info.dataDir,
              title: info.dataDir,
            },
            {
              label: "Ports",
              value: `${settings.networkPort} / ${settings.rpcPort}`,
            },
            {
              label: "Tor",
              value: `${settings.socksPort} / ${settings.controlPort}`,
            },
            {
              label: "Minimum swap",
              value: <SatsAmount sats={settings.minSwapAmount} />,
            },
            { label: "Status", value: status.phase.phase },
          ].map(({ label, value, title }) => (
            <div key={label} className="min-w-0 bg-surface/80 p-4 transition-colors duration-200 hover:bg-white/[0.035]">
              <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
                {label}
              </span>
              <strong
                className="mt-1.5 block truncate font-mono text-[11.5px] text-foreground"
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
  const [amount, setAmount] = useState("");
  const [feeRate, setFeeRate] = useState("2");
  const [sending, setSending] = useState(false);

  const spendable = utxos
    .filter((u) => u.spendable && u.solvable)
    .reduce((sum, u) => sum + u.amountSats, 0);
  const amountSats = Math.floor(Number(amount)) || 0;
  const rate = Number(feeRate);
  const blocked =
    recipient.trim().length === 0 ||
    amountSats <= 0 ||
    amountSats > spendable ||
    !Number.isFinite(rate) ||
    rate <= 0;

  async function send() {
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
        <div className="grid grid-cols-[1fr_110px] gap-3">
          <TextField
            label="Amount"
            inputMode="numeric"
            placeholder="0"
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
            error={amountSats > spendable ? "More than this wallet holds." : undefined}
          />
          <TextField
            label="s/vB"
            inputMode="decimal"
            value={feeRate}
            onChange={(e) => setFeeRate(e.target.value)}
          />
        </div>
        <Button className="w-full" disabled={blocked} loading={sending} onClick={() => void send()}>
          Send
        </Button>
      </div>
    </Card>
  );
}

function WalletPanel({
  routerId,
  running,
}: {
  routerId: string;
  running: boolean;
}) {
  const pushToast = useToastStore((state) => state.push);
  const [utxos, setUtxos] = useState<UtxoEntry[]>([]);
  const [transactions, setTransactions] = useState<TxSummary[]>([]);
  const [address, setAddress] = useState<NewAddress | null>(null);
  const [addressType, setAddressType] = useState<AddressType>("p2wpkh");
  const [generating, setGenerating] = useState(false);
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
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

  const load = useCallback(async () => {
    if (!running) return;
    setLoading(true);
    const [u, t] = await Promise.all([
      listRouterUtxos(routerId),
      getRouterTransactions(routerId, 30, 0),
    ]);
    setUtxos(u);
    setTransactions(t);
    setLoading(false);
  }, [routerId, running]);
  useEffect(() => {
    void load().catch(() => setLoading(false));
  }, [load]);
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
    <div className="space-y-4">
      <div className="grid grid-cols-2 gap-4 max-[900px]:grid-cols-1">
      <RouterSendPanel routerId={routerId} utxos={utxos} onSent={load} />
      <Card className="border-line-strong p-5">
        <div className="flex items-center justify-between gap-4">
          <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
            Receive Bitcoin
          </span>
          <SegmentedToggle
            groupId="router-address-type"
            value={addressType}
            onChange={(next) => {
              setAddressType(next);
              // Cleared rather than left standing: the panel is labelled with the selected
              // type, so holding the previous one offers the wrong address to copy — and the
              // QR is a picture of that same wrong address.
              setAddress(null);
              setQrDataUrl(null);
            }}
            options={[
              { value: "p2wpkh", label: "SegWit" },
              { value: "p2tr", label: "Taproot" },
            ]}
          />
        </div>
        <div className="mt-4 flex justify-center">
          {/* The plate is always here, empty or not: a button that reaches the wallet and comes
              back with nothing on screen reads as a button that did nothing. The white ground
              only appears with a QR on it — a bare white square waiting looks like a broken
              image rather than something loading. */}
          <div
            className={`grid h-[212px] w-[212px] place-items-center rounded-card p-3.5 ${
              qrDataUrl
                ? "bg-white shadow-[0_0_0_1px_rgba(255,255,255,0.16)]"
                : "border border-line bg-surface"
            }`}
          >
            {qrDataUrl ? (
              <img src={qrDataUrl} alt="Router receive address QR code" width={184} height={184} />
            ) : generating ? (
              <RefreshCw size={24} strokeWidth={1.8} className="animate-spin text-subtle" />
            ) : (
              <span className="px-4 text-center text-[11.5px] leading-5 text-subtle">
                Generate a fresh {addressType === "p2tr" ? "Taproot" : "SegWit"} address to
                receive into this router's wallet.
              </span>
            )}
          </div>
        </div>
        {address && (
          <code
            className="mt-4 block break-all rounded-control border border-line bg-surface p-3
              text-[11px] text-foreground"
          >
            {address.address}
          </code>
        )}
        <Button
          className="mt-4 w-full"
          loading={generating}
          onClick={() => {
            setGenerating(true);
            void getRouterNewAddress(routerId, addressType)
              .then(setAddress)
              .catch((e) => pushToast("error", e.message))
              .finally(() => setGenerating(false));
          }}
        >
          Generate address
        </Button>
      </Card>
      </div>
      <Card className="border-line-strong">
        <div className="flex items-center justify-between gap-4 border-b border-line px-5 py-3">
          <h2 className="font-header text-[14px] font-bold">
            UTXOs{" "}
            <span className="ml-2 font-mono text-[10px] text-subtle">
              {utxos.length}
            </span>
          </h2>
          <IconButton
            label="Sync wallet"
            disabled={loading}
            onClick={() =>
              void syncRouterWallet(routerId)
                .then(load)
                .catch((e) => pushToast("error", e.message))
            }
            icon={<RefreshCw size={16} strokeWidth={1.8} className={loading ? "animate-spin" : ""} />}
          />
        </div>
        <div className="max-h-[330px] overflow-auto">
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
      <Card className="border-line-strong">
        <div className="border-b border-line px-5 py-4">
          <h2 className="font-header text-[14px] font-bold">
            Recent transactions
          </h2>
        </div>
        <div className="divide-y divide-line">
          {transactions.slice(0, 8).map((tx) => (
            <div
              key={`${tx.txid}:${tx.category}`}
              className="flex items-center justify-between gap-4 px-5 py-3"
            >
              <Identifier value={tx.txid} className="min-w-0 text-[11px] leading-[1.45] text-muted" />
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
  );
}

type RouterLogFilter = "all" | "info" | "warn" | "error";

function LogsPanel({ routerId }: { routerId: string }) {
  const [lines, setLines] = useState<LogLine[]>([]);
  const [loading, setLoading] = useState(false);
  const [levelFilter, setLevelFilter] = useState<RouterLogFilter>("all");
  const load = useCallback(async () => {
    setLoading(true);
    try {
      setLines(await getRouterLogs(routerId, 500));
    } finally {
      setLoading(false);
    }
  }, [routerId]);
  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), 3000);
    return () => clearInterval(timer);
  }, [load]);
  const counts = useMemo(
    () =>
      lines.reduce(
        (result, row) => {
          const level = logLevel(row.line);
          result[level] += 1;
          return result;
        },
        { error: 0, warn: 0, info: 0, debug: 0, other: 0 } as Record<LogLevel, number>,
      ),
    [lines],
  );
  const filteredLines = useMemo(
    () =>
      levelFilter === "all"
        ? lines
        : lines.filter((line) => logLevel(line.line) === levelFilter),
    [levelFilter, lines],
  );
  return (
    <Card className="flex h-[580px] flex-col border-line-strong">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-line px-5 py-3">
        <div>
          <h2 className="font-header text-[14px] font-bold">Router logs</h2>
          <span className="text-[10px] text-subtle">
            Latest 500 lines · refreshes every 3 seconds
          </span>
        </div>
        <div className="flex items-center gap-2">
          <SegmentedToggle
            groupId={`router-log-level-${routerId}`}
            subdued
            value={levelFilter}
            onChange={setLevelFilter}
            options={[
              { value: "all", label: "All", suffix: <span className="font-mono text-[9px] opacity-60">{lines.length}</span> },
              { value: "info", label: "Info", suffix: <span className="font-mono text-[9px] opacity-60">{counts.info}</span> },
              { value: "warn", label: "Warn", suffix: <span className="font-mono text-[9px] opacity-60">{counts.warn}</span> },
              { value: "error", label: "Error", suffix: <span className="font-mono text-[9px] opacity-60">{counts.error}</span> },
            ]}
          />
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void load()}
            loading={loading}
          >
            <RefreshCw size={13} />
            Refresh
          </Button>
        </div>
      </div>
      <LogViewer
        lines={filteredLines}
        emptyMessage={
          lines.length === 0
            ? "No log lines yet."
            : `No ${levelFilter} log entries in the latest 500 lines.`
        }
      />
    </Card>
  );
}

// Tor's ports are Portal's to choose, not the router's: `build_config` overrides whatever a
// registration carries with the live runtime's pair. They are shown below as status only.
const EDITABLE_SETTING_KEYS = [
  "networkPort",
  "rpcPort",
  "requiredConfirms",
  "minSwapAmount",
  "baseFee",
  "amountRelativeFeePct",
  "timeRelativeFeePct",
  "fidelityAmount",
  "fidelityTimelock",
] as const;
type EditableSettingKey = (typeof EDITABLE_SETTING_KEYS)[number];
type SettingsForm = Record<EditableSettingKey, string>;

function settingsToForm(settings: RouterSettings): SettingsForm {
  return Object.fromEntries(
    EDITABLE_SETTING_KEYS.map((key) => [key, String(settings[key])]),
  ) as SettingsForm;
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
    "minSwapAmount",
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
  if (values.minSwapAmount < 10_000)
    return "Minimum swap amount must be at least 10,000 sats.";
  if (values.fidelityAmount < 1)
    return "Fidelity amount must be greater than zero.";
  if (values.requiredConfirms < 1)
    return "Required confirmations must be at least one.";
  if (values.fidelityTimelock < 12_960 || values.fidelityTimelock > 25_920) {
    return "Fidelity timelock must be between 12,960 and 25,920 blocks.";
  }
  return { ...settings, ...values };
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

  const parsed = parseSettingsForm(settings, form);
  const error = typeof parsed === "string" ? parsed : null;
  const dirty = EDITABLE_SETTING_KEYS.some(
    (key) => form[key] !== String(settings[key]),
  );
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
      pushToast("success", `${routerId} is running again.`);
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
    if (typeof parsed === "string" || transitioning) return;
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
          subtitle="Minimum size and advertised router fees"
        >
          <div className="col-span-2 max-[620px]:col-span-1">
            <SummaryGroup title="Advertised policy">
              {row("minSwapAmount", "Minimum swap amount", { suffix: "sats" })}
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
              {row("fidelityAmount", "Target amount", { suffix: "sats" })}
              {row("fidelityTimelock", "Timelock", { suffix: "blocks" })}
            </SummaryGroup>
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
              ? `Start ${routerId} again`
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
          title={`Remove ${routerId}?`}
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
                      pushToast("success", `${routerId} was removed.`);
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
            This removes <strong className="text-foreground">{routerId}</strong>{" "}
            from the app, permanently — it will not reappear. Its wallet file
            and anything on-chain are left untouched.
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
  const [copied, setCopied] = useState(false);
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
  if (loading || !status || !settings || !info)
    return (
      <div className="mx-auto w-full max-w-xl pt-20">
        <SkeletonLines count={10} />
      </div>
    );
  const tor = status.torAddress;
  return (
    <div className="h-full overflow-y-auto p-8">
      <div className="mx-auto w-full max-w-[1380px] pb-8">
        <header className="flex flex-wrap items-center justify-between gap-4">
          <div className="flex min-w-0 items-center gap-3">
            <BackButton to="/router" label="Back to routers" />
            <div className="min-w-0">
              <span className="mb-1 block font-mono text-[9px] font-semibold uppercase tracking-[0.18em] text-primary">Router workspace · Signet</span>
              <div className="flex items-center gap-2">
                <h1 className="truncate font-header text-[27px] font-bold">
                  {id}
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
              <button
                type="button"
                disabled={!tor}
                onClick={() =>
                  tor &&
                  void copyText(tor).then((ok) => {
                    if (!ok) return;
                    setCopied(true);
                    setTimeout(() => setCopied(false), 1200);
                  })
                }
                className="mt-1 flex max-w-full items-center gap-2 text-left font-mono text-[10.5px]
                  text-subtle disabled:cursor-default"
              >
                <span className="truncate">
                  {tor
                    ? formatTorEndpoint(tor, 24, 14, true)
                    : `Status · ${phase}`}
                </span>
                {tor && (
                  <Copy size={12} className={copied ? "text-success" : ""} />
                )}
              </button>
            </div>
          </div>
          <div className="flex gap-2">
            {running ? (
              <Button
                onClick={() => void stop()}
                loading={actionLoading}
                disabled={transitioning}
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
        <main className="mt-5">
          {tab === "overview" && (
            <OverviewPanel
              status={status}
              settings={settings}
              info={info}
              balances={balances}
              bonds={bonds}
              reports={reports}
            />
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
