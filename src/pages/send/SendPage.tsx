import { ArrowDownLeft, ArrowUpRight, ChevronDown, Copy, Download, Droplets, ExternalLink, RefreshCw } from "lucide-react";
import { UnresolvedPayments } from "../../components/app/UnresolvedPayments";
import { spendingBlocked, useUnresolvedStore } from "../../store/unresolved";
import QRCode from "qrcode";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { estimateFees, getBalances, getBtcPrice, getNewAddress, getTransactions, listUtxos, sendToAddress, validateAddress, verifyLastAddress } from "../../api/commands";
import { isAppError } from "../../api/types";
import type { AddressType, Balances, FeeEstimate, NewAddress, Outpoint, TxSummary, UtxoEntry } from "../../api/types";
import { Card, Identifier, Modal, SatsAmount } from "../../components/ui/display";
import { Button, PresetTile, SegmentedToggle, TextField } from "../../components/ui/inputs";
import {
  classifySpendType,
  formatFeeRate,
  formatUnitAmount,
  satsToUnitString,
  unitStringToSats,
  type Unit,
} from "../../lib/wallet-format";
import { useToastStore } from "../../store/toast";
import { refreshWalletCache } from "../../lib/wallet-sync";
import { usePendingSendsStore } from "../../store/pending-sends";
import { useWalletCacheStore } from "../../store/wallet-cache";
import { FAUCET_URL, isOurSignet, useConnectionStore } from "../../store/connection";
import { openExternal } from "../../platform";
import { copyText } from "../../lib/clipboard";

/** Fixed, always-distinct choices. The mempool quote informs the hint below them, not the tiles
 *  themselves — API-derived tiers collapse to the same number on a quiet mempool. */
const FEE_PRESETS = [1, 2, 3] as const;
const MAX_PRESET = FEE_PRESETS[FEE_PRESETS.length - 1];

type FeeKey = (typeof FEE_PRESETS)[number] | "custom";

/** Coins on our signet have no value and nowhere to be bought, so the faucet is the only way
 *  to get any — which is also why this must not appear anywhere else: on a chain whose coins
 *  are real, a "free coins" button is at best a lie. */
function FaucetButton() {
  const ours = useConnectionStore(isOurSignet);
  if (!ours) return null;
  return (
    <button
      type="button"
      onClick={() => void openExternal(FAUCET_URL)}
      title="Get signet coins from our faucet"
      className="lift inline-flex flex-none items-center gap-1.5 rounded-pill border border-primary/30 bg-primary/[0.08] px-2.5 py-1 font-mono text-[10.5px] uppercase tracking-[0.14em] text-primary outline-none hover:border-primary/50 hover:bg-primary/[0.14] focus-visible:shadow-ring"
    >
      <Droplets size={12} strokeWidth={2} />
      Faucet
      <ExternalLink size={11} strokeWidth={2} />
    </button>
  );
}

function SendPanel() {
  const pushToast = useToastStore((s) => s.push);
  const pushFailure = useToastStore((s) => s.pushFailure);
  const walletSyncStatus = useWalletCacheStore((s) => s.syncStatus);
  const walletSyncError = useWalletCacheStore((s) => s.syncError);
  const [balances, setBalances] = useState<Balances | null>(null);
  const [utxos, setUtxos] = useState<UtxoEntry[]>([]);
  const [fees, setFees] = useState<FeeEstimate | null>(null);
  const [feesFailed, setFeesFailed] = useState(false);
  const [btcPrice, setBtcPrice] = useState<number | null>(null);
  const [btcPriceCached, setBtcPriceCached] = useState(false);

  const [recipient, setRecipient] = useState("");
  const [recipientValidation, setRecipientValidation] = useState<"idle" | "checking" | "valid" | "invalid">("idle");
  const [recipientError, setRecipientError] = useState<string | undefined>();
  const [unit, setUnit] = useState<Unit>("sats");
  const [amountInput, setAmountInput] = useState("");
  const [feeKey, setFeeKey] = useState<FeeKey>(2);
  const [customFeeRate, setCustomFeeRate] = useState("");
  const [selectedOutpoints, setSelectedOutpoints] = useState<Outpoint[]>([]);
  const [sending, setSending] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const recordSend = usePendingSendsStore((s) => s.record);

  const load = useCallback(async () => {
    const [nextBalances, nextUtxos] = await Promise.all([getBalances(), listUtxos()]);
    setBalances(nextBalances);
    setUtxos(nextUtxos);
  }, []);

  // Kept out of `load`: this one leaves the wallet entirely and asks mempool.space, so a
  // third-party outage must not take the balance and UTXO picker down with it.
  const loadFees = useCallback(() => {
    setFeesFailed(false);
    void estimateFees()
      .then(setFees)
      .catch(() => {
        setFees(null);
        setFeesFailed(true);
      });
  }, []);

  useEffect(() => {
    void load().catch((e) => pushFailure(e, "Failed to load wallet data."));
    loadFees();
    // BTC/USD price is best-effort — sats/BTC still work fine without it, so its own failure
    // shouldn't toast an error, just leave the USD option disabled.
    void getBtcPrice()
      .then((p) => {
        setBtcPrice(p.usd);
        setBtcPriceCached(p.cached);
      })
      .catch(() => {
        setBtcPrice(null);
        setBtcPriceCached(false);
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function changeUnit(nextUnit: Unit) {
    const sats = unitStringToSats(amountInput, unit, btcPrice);
    setAmountInput(satsToUnitString(sats, nextUnit, btcPrice));
    setUnit(nextUnit);
  }

  const amountSats = useMemo(
    () => unitStringToSats(amountInput, unit, btcPrice),
    [amountInput, unit, btcPrice],
  );
  const otherUnits = useMemo(() => (["sats", "btc", "usd"] as Unit[]).filter((u) => u !== unit), [unit]);

  // The mempool quote as one whole number: the midpoint of the range it gives, rounded, because
  // a rate is only ever chosen in whole sats/vB here. 2-2 reads 2, 2-4 reads 3.
  const mempoolRate = useMemo(
    () => (fees === null ? null : Math.round((fees.low + fees.high) / 2)),
    [fees],
  );

  const feeRate = useMemo(
    () => (feeKey === "custom" ? Number(customFeeRate) || 0 : feeKey),
    [feeKey, customFeeRate],
  );

  const spendableUtxos = useMemo(() => utxos.filter((u) => u.spendable && u.solvable), [utxos]);
  const selectedTotal = useMemo(() => {
    const set = new Set(selectedOutpoints.map((o) => `${o.txid}:${o.vout}`));
    return spendableUtxos.filter((u) => set.has(`${u.txid}:${u.vout}`)).reduce((sum, u) => sum + u.amountSats, 0);
  }, [selectedOutpoints, spendableUtxos]);

  function toggleOutpoint(u: UtxoEntry) {
    const key = `${u.txid}:${u.vout}`;
    setSelectedOutpoints((prev) => {
      const exists = prev.some((o) => `${o.txid}:${o.vout}` === key);
      if (exists) return prev.filter((o) => `${o.txid}:${o.vout}` !== key);
      return [...prev, { txid: u.txid, vout: u.vout }];
    });
  }

  const amountError = amountInput.length > 0 && amountSats <= 0 ? "Enter a valid amount." : undefined;

  useEffect(() => {
    let cancelled = false;
    const address = recipient.trim();
    setRecipientError(undefined);
    if (!address) {
      setRecipientValidation("idle");
      return () => { cancelled = true; };
    }

    setRecipientValidation("checking");
    const timer = setTimeout(() => {
      void validateAddress(address)
        .then((result) => {
          if (cancelled) return;
          setRecipientValidation(result.valid ? "valid" : "invalid");
          setRecipientError(result.error);
        })
        .catch(() => {
          if (cancelled) return;
          setRecipientValidation("invalid");
          setRecipientError("Could not validate this address.");
        });
    }, 200);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [recipient]);

  const walletReadyToSpend = walletSyncStatus === "synced";
  // An earlier payment whose outcome is unknown holds this: sending again could pay twice.
  // Joins the existing readiness check rather than adding a separate gate.
  // Not the list length: a journal we could not read must hold this too, or a failed query
  // reads as "nothing outstanding" and the next payment goes out over an unknown one.
  const paymentsHeld = useUnresolvedStore(spendingBlocked);
  const canSend =
    walletReadyToSpend &&
    recipientValidation === "valid" &&
    amountSats > 0 &&
    feeRate > 0 &&
    !paymentsHeld;

  async function submitSend() {
    if (useWalletCacheStore.getState().syncStatus !== "synced") {
      // Not a fault: the sync runs on arrival and clears on its own.
      pushToast("warning", "Wait for the wallet sync to finish before sending.");
      return;
    }
    setConfirming(false);
    setSending(true);
    try {
      const to = recipient.trim();
      const result = await sendToAddress(
        to,
        amountSats,
        feeRate,
        selectedOutpoints.length > 0 ? selectedOutpoints : undefined,
      );
      // Recorded before anything else: this is the only place the txid is known for certain, and
      // the wallet's own history cannot be relied on to show the transaction — see the note in
      // `store/pending-sends.ts`.
      recordSend({
        txid: result.txid,
        walletPath: useWalletCacheStore.getState().info?.walletPath ?? "",
        address: to,
        amountSats,
        feeRate,
        createdAt: Math.floor(Date.now() / 1000),
      });
      pushToast(
        "success",
        "Broadcast. The payment is in the mempool, waiting to be mined.",
      );
      setRecipient("");
      setAmountInput("");
      setSelectedOutpoints([]);
      await load();
      // The shared cache is what the Wallet page reads, and it otherwise only refreshes on a
      // two-minute interval — long enough that a send looks like it did nothing.
      void refreshWalletCache().catch(() => {});
    } catch (e) {
      const err = isAppError(e) ? e : null;
      pushToast("error", err?.message ?? "Send failed.");
    } finally {
      setSending(false);
    }
  }

  return (
    <Card className="flex flex-col gap-4 border-line-strong p-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2.5">
          <span className="flex h-[30px] w-[30px] items-center justify-center rounded-full border border-danger/40 bg-danger/[0.08] text-danger">
            <ArrowUpRight size={15} strokeWidth={2} />
          </span>
          <h2 className="font-header text-[15px] font-bold text-foreground">Send</h2>
        </div>
        <div className="flex items-center gap-2.5">
          <span className="text-[11px] text-subtle">
            Spendable: <SatsAmount sats={balances?.spendable ?? 0} className="text-foreground" />
          </span>
          <FaucetButton />
        </div>
      </div>

      {!walletReadyToSpend && (
        <div className="rounded-control border border-warning/35 bg-warning/[0.08] px-3.5 py-2.5 text-[12px] text-warning">
          {walletSyncStatus === "error"
            ? `Sending disabled: ${walletSyncError ?? "wallet synchronization failed."}`
            : "Sending is enabled after the initial wallet sync completes."}
        </div>
      )}

      <TextField
        label="Recipient Address"
        placeholder="bc1… or tb1…"
        value={recipient}
        onChange={(e) => setRecipient(e.target.value)}
        error={recipientError}
        hint={
          recipientValidation === "checking"
            ? "Checking address…"
            : recipientValidation === "valid"
              ? "Valid Bitcoin address. Network is checked before sending."
              : undefined
        }
        autoComplete="off"
        spellCheck={false}
      />

      <div className="grid grid-cols-[1fr_auto] items-end gap-3">
        <TextField
          label="Amount"
          inputMode="decimal"
          placeholder="0"
          value={amountInput}
          onChange={(e) => setAmountInput(e.target.value)}
          error={amountError}
        />
        <SegmentedToggle
          groupId="send-unit"
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
      {!amountError && (
        <div className="-mt-2 flex items-center justify-between px-1 text-[11px] text-subtle">
          <span>{formatUnitAmount(amountSats, otherUnits[0], btcPrice) ?? "—"}</span>
          <span>{formatUnitAmount(amountSats, otherUnits[1], btcPrice) ?? "—"}</span>
        </div>
      )}

      <div className="flex flex-col gap-2">
        <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Fee Rate</span>
        <div className="grid grid-cols-4 gap-2">
          {FEE_PRESETS.map((rate) => (
            <PresetTile
              key={rate}
              onClick={() => setFeeKey(rate)}
              selected={feeKey === rate}
              label={`${rate} s/vB`}
              size="sm"
            />
          ))}
          <PresetTile
            onClick={() => setFeeKey("custom")}
            selected={feeKey === "custom"}
            label="Custom"
            size="sm"
          />
        </div>
        {/* The presets cover a normal mempool; this is the line that tells you when it isn't one,
            so a spike doesn't silently leave every preset too low to confirm. */}
        {mempoolRate !== null && (
          <p className={`text-[11.5px] ${mempoolRate > MAX_PRESET ? "text-warning" : "text-subtle"}`}>
            {mempoolRate > MAX_PRESET
              ? `The mempool is asking about ${mempoolRate} s/vB — use Custom, or these will be slow to confirm.`
              : `Mempool right now: ${mempoolRate} s/vB.`}
          </p>
        )}
        {feesFailed && (
          <div className="flex items-center justify-between gap-3">
            <span className="text-[11.5px] text-subtle">Could not read the mempool.</span>
            <Button size="sm" variant="ghost" onClick={loadFees}>
              Try again
            </Button>
          </div>
        )}
        {feeKey === "custom" && (
          <TextField
            label="Custom rate (sats/vB)"
            inputMode="decimal"
            placeholder="e.g. 8"
            value={customFeeRate}
            onChange={(e) => setCustomFeeRate(e.target.value)}
          />
        )}
      </div>

      <details className="border-t border-dashed border-line pt-3">
        <summary className="flex cursor-pointer list-none items-center gap-1.5 font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle marker:content-none hover:text-foreground">
          <ChevronDown size={12} strokeWidth={2.5} />
          Manual UTXO Picker
        </summary>
        <div className="mt-3 flex flex-col gap-2.5">
          <p className="text-[11.5px] text-subtle">Leave nothing selected to let the wallet auto-select coins.</p>
          <div className="flex max-h-45 flex-col gap-1.5 overflow-y-auto">
            {spendableUtxos.length === 0 && <p className="text-[11.5px] text-subtle">No spendable UTXOs.</p>}
            {spendableUtxos.map((u) => {
              const key = `${u.txid}:${u.vout}`;
              const checked = selectedOutpoints.some((o) => `${o.txid}:${o.vout}` === key);
              return (
                <label
                  key={key}
                  className="flex cursor-pointer items-center justify-between gap-3 rounded-control border border-line bg-surface-raised px-3 py-2"
                >
                  <span className="flex min-w-0 items-center gap-2 font-mono text-[11px] text-muted">
                    <input type="checkbox" checked={checked} onChange={() => toggleOutpoint(u)} className="flex-none accent-primary" />
                    <Identifier value={u.address ?? `${u.txid}:${u.vout}`} className="text-[11px] leading-[1.45]" />
                    <span className="flex-none rounded-control border border-line px-1.5 py-0.5 text-[9px] text-subtle">
                      {classifySpendType(u.spendType)}
                    </span>
                  </span>
                  <SatsAmount sats={u.amountSats} className="flex-none text-[11px] font-semibold text-foreground" />
                </label>
              );
            })}
          </div>
          {selectedOutpoints.length > 0 && (
            <p className="text-[11.5px] text-subtle">
              Selected: <SatsAmount sats={selectedTotal} className="text-foreground" />
            </p>
          )}
        </div>
      </details>

      <div className="flex-1" />

      <UnresolvedPayments verb="sending" />
      <Button
        size="md"
        disabled={!canSend}
        loading={sending}
        onClick={() => setConfirming(true)}
      >
        Send
      </Button>

      {confirming && (
        <Modal
          title="Confirm this payment"
          onClose={() => setConfirming(false)}
          footer={
            <>
              <Button variant="secondary" onClick={() => setConfirming(false)}>
                Cancel
              </Button>
              <Button onClick={() => void submitSend()} loading={sending}>
                Broadcast
              </Button>
            </>
          }
        >
          <div className="flex flex-col gap-2.5 rounded-control border border-line bg-surface-raised px-3.5 py-3">
            <span className="flex flex-col gap-1">
              <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
                To
              </span>
              <span className="break-all font-mono text-[12px] text-foreground">
                {recipient.trim()}
              </span>
            </span>
            <span className="flex items-baseline justify-between gap-3 border-t border-line pt-2.5">
              <span className="text-[12px] text-muted">Amount</span>
              <strong className="font-numeric text-[13.5px] text-foreground">
                <SatsAmount sats={amountSats} />
              </strong>
            </span>
            <span className="flex items-baseline justify-between gap-3">
              <span className="text-[12px] text-muted">Fee rate</span>
              <span className="font-numeric text-[12.5px] text-foreground">
                {formatFeeRate(feeRate)} s/vB
              </span>
            </span>
            <span className="flex items-baseline justify-between gap-3">
              <span className="text-[12px] text-muted">Inputs</span>
              <span className="text-[12.5px] text-foreground">
                {selectedOutpoints.length > 0
                  ? `${selectedOutpoints.length} chosen by hand`
                  : "Chosen automatically"}
              </span>
            </span>
          </div>
          <p className="text-[11.5px] leading-5 text-subtle">
            The wallet builds the final network fee from this rate. Broadcasting cannot be undone.
          </p>
        </Modal>
      )}
    </Card>
  );
}

// The Recent Addresses disclosure lists at most 8 entries, and every extra transaction in this
// window costs Electrum an input fetch.
const RECENT_ADDRESS_TX_WINDOW = 25;

function ReceivePanel() {
  const pushToast = useToastStore((s) => s.push);
  const [addressType, setAddressType] = useState<AddressType>("p2wpkh");
  // Kept per type: an unissued address stays valid until it is paid, so switching
  // SegWit/Taproot and back is a lookup instead of another round trip to the wallet.
  const [issued, setIssued] = useState<Partial<Record<AddressType, NewAddress>>>({});
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [pendingType, setPendingType] = useState<AddressType | null>(null);
  const [transactions, setTransactions] = useState<TxSummary[]>([]);
  const current = issued[addressType] ?? null;

  // A ref, not `pendingType`: this has to reject a duplicate synchronously, before the state
  // update lands, or toggling the two types quickly issues two requests for the same one.
  const inFlight = useRef<Set<AddressType>>(new Set());
  // Swapping an address out from under a copy would leave the user pasting one thing while the
  // panel shows another, so a copied address is only ever replaced on request.
  const copied = useRef<Set<string>>(new Set());

  const generate = useCallback(
    async (type: AddressType) => {
      if (inFlight.current.has(type)) return;
      inFlight.current.add(type);
      setPendingType(type);
      try {
        const next = await verifyLastAddress(type);
        setIssued((prev) => ({ ...prev, [type]: next }));
      } catch (e) {
        pushToast("error", (e as { message?: string })?.message ?? "Failed to generate address.");
      } finally {
        inFlight.current.delete(type);
        setPendingType((p) => (p === type ? null : p));
      }
    },
    [pushToast],
  );

  useEffect(() => {
    if (issued[addressType]) return;
    if (inFlight.current.has(addressType)) return;
    inFlight.current.add(addressType);
    setPendingType(addressType);
    void getNewAddress(addressType)
      .then((next) => setIssued((prev) => ({ ...prev, [addressType]: next })))
      .catch((e) =>
        pushToast("error", (e as { message?: string })?.message ?? "Failed to generate address."),
      )
      .finally(() => {
        inFlight.current.delete(addressType);
        setPendingType((p) => (p === addressType ? null : p));
      });
  }, [addressType, issued, pushToast]);

  // The chain check the fast path skipped. Runs once per type, after the address is on screen;
  // if the cached address turns out to have been paid, the fresh one replaces it silently —
  // unless the user has already copied it, in which case they are told instead.
  const verified = useRef<Set<AddressType>>(new Set());
  useEffect(() => {
    const current = issued[addressType];
    if (!current || current.verified || verified.current.has(addressType)) return;
    verified.current.add(addressType);
    void verifyLastAddress(addressType)
      .then((next) => {
        if (next.address === current.address) {
          setIssued((prev) => ({ ...prev, [addressType]: next }));
          return;
        }
        if (copied.current.has(current.address)) {
          pushToast(
            "warning",
            "The address you copied has been paid. Generate a new address before reusing it.",
          );
          return;
        }
        setIssued((prev) => ({ ...prev, [addressType]: next }));
      })
      .catch(() => {
        // A failed check leaves the address on offer: it is the last one issued and almost
        // certainly still unused, and refusing to show one would be worse than not confirming it.
        verified.current.delete(addressType);
      });
  }, [addressType, issued, pushToast]);

  // Deferred behind the first address, and only ever fetched once: both calls take the wallet's
  // read lock, and this one feeds a disclosure the user has to open before it is even visible.
  const historyRequested = useRef(false);
  useEffect(() => {
    if (!current || historyRequested.current) return;
    historyRequested.current = true;
    void getTransactions(RECENT_ADDRESS_TX_WINDOW, 0).then(setTransactions).catch(() => {});
  }, [current]);

  useEffect(() => {
    // Cleared, not left standing: the panel is labelled with the selected type, so holding the
    // previous type's QR while a new address loads offers the wrong address to copy.
    setQrDataUrl(null);
    if (!current) return;
    let cancelled = false;
    void QRCode.toDataURL(current.address, { width: 184, margin: 1 }).then((url) => {
      if (!cancelled) setQrDataUrl(url);
    });
    return () => {
      cancelled = true;
    };
  }, [current]);

  const recentAddresses = useMemo(() => {
    const seen = new Map<string, number>();
    for (const tx of transactions) {
      if (!tx.address || tx.amountSats <= 0) continue;
      seen.set(tx.address, (seen.get(tx.address) ?? 0) + tx.amountSats);
    }
    return [...seen.entries()].slice(0, 8);
  }, [transactions]);

  function copyAddress() {
    if (!current) return;
    copied.current.add(current.address);
    void copyText(current.address).then((ok) =>
      ok
        ? pushToast("success", "Address copied.")
        : pushToast("warning", "Could not reach the clipboard — select the address and copy it."),
    );
  }

  function exportCsv() {
    const rows = ["address,received_sats", ...recentAddresses.map(([addr, sats]) => `${addr},${sats}`)];
    const blob = new Blob([rows.join("\n")], { type: "text/csv" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "receive-addresses.csv";
    a.click();
    URL.revokeObjectURL(url);
  }

  return (
    <Card className="flex flex-col gap-4 border-line-strong p-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2.5">
          <span className="flex h-[30px] w-[30px] items-center justify-center rounded-full border border-success/40 bg-success/[0.08] text-success">
            <ArrowDownLeft size={15} strokeWidth={2} />
          </span>
          <h2 className="font-header text-[15px] font-bold text-foreground">Receive</h2>
        </div>
        <FaucetButton />
      </div>

      <SegmentedToggle
        groupId="address-type"
        value={addressType}
        onChange={setAddressType}
        options={[
          { value: "p2wpkh", label: "SegWit" },
          { value: "p2tr", label: "Taproot" },
        ]}
      />

      <div className="flex justify-center py-1">
        {/* The white plate only appears with the QR on it: a 212px white slab waiting on a dark
            page reads as a broken image rather than as something loading. */}
        <div
          className={`grid h-[212px] w-[212px] place-items-center rounded-card p-3.5 ${
            qrDataUrl
              ? "bg-white shadow-[0_0_0_1px_rgba(255,255,255,0.16)]"
              : "border border-line bg-surface-raised"
          }`}
        >
          {qrDataUrl ? (
            <img src={qrDataUrl} alt="Receive address QR code" width={184} height={184} />
          ) : (
            <RefreshCw size={24} strokeWidth={1.8} className="animate-spin text-subtle" />
          )}
        </div>
      </div>

      <label className="flex flex-col gap-2">
        <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Your Address</span>
        <div className="flex min-h-[46px] items-center justify-between gap-2 rounded-control border border-line-strong bg-surface-raised px-3.5 py-2.5">
          {current ? (
            // `select-all` so one click takes the whole address: this is the value a user
            // falls back to lifting by hand when the clipboard is out of reach.
            <Identifier value={current.address} className="select-all text-[12.5px] leading-[1.5] text-muted" />
          ) : (
            <span className="font-mono text-[12.5px] text-subtle">
              {pendingType === addressType ? "Generating…" : "—"}
            </span>
          )}
          <button
            type="button"
            onClick={copyAddress}
            disabled={!current}
            className="grid h-[26px] w-[26px] flex-none place-items-center rounded text-subtle hover:bg-primary/10 hover:text-primary disabled:opacity-40"
          >
            <Copy size={13} strokeWidth={2} />
          </button>
        </div>
      </label>

      {/* Bypasses the per-type cache: this is the one control that asks the wallet whether the
          address it issued has been paid, and hands over a fresh one if it has. */}
      <Button
        variant="secondary"
        onClick={() => void generate(addressType)}
        loading={pendingType !== null}
      >
        Generate New Address
      </Button>

      <details className="border-t border-dashed border-line pt-3">
        <summary className="flex cursor-pointer list-none items-center gap-1.5 font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle marker:content-none hover:text-foreground">
          <ChevronDown size={12} strokeWidth={2.5} />
          Recent Addresses
        </summary>
        <div className="mt-3 flex flex-col divide-y divide-line">
          {recentAddresses.length === 0 && <p className="py-2 text-[11.5px] text-subtle">No incoming transactions yet.</p>}
          {recentAddresses.map(([addr, sats]) => (
            <div key={addr} className="flex items-center justify-between gap-3 py-2 text-[11.5px]">
              <Identifier value={addr} className="text-[11.5px] leading-[1.45] text-muted" />
              <SatsAmount sats={sats} className="flex-none font-semibold text-success" />
            </div>
          ))}
        </div>
        {recentAddresses.length > 0 && (
          <button
            type="button"
            onClick={exportCsv}
            className="mt-2 flex items-center gap-1.5 font-mono text-[11px] text-primary hover:text-primary-hover"
          >
            <Download size={12} strokeWidth={2} /> Export CSV
          </button>
        )}
      </details>

      <div className="flex-1" />
    </Card>
  );
}

export function SendPage() {
  return (
    <div className="flex h-full flex-col overflow-y-auto px-8 pb-8 pt-2">
      <div className="shrink-0 pb-4">
        <h1 className="font-header text-[26px] font-bold text-foreground">Send &amp; Receive</h1>
        <p className="mt-1 text-[13.5px] text-muted">One shared balance — send a payment or generate a receiving address here.</p>
      </div>
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <SendPanel />
        <ReceivePanel />
      </div>
    </div>
  );
}
