import { ArrowDownLeft, ArrowUpRight, ChevronDown, Copy } from "lucide-react";
import { UnresolvedPayments } from "../../components/app/UnresolvedPayments";
import { spendingBlocked, useUnresolvedStore } from "../../store/unresolved";
import { useCallback, useEffect, useMemo, useState } from "react";
import { estimateSendFee, getBalances, getBtcPrice, getNewAddress, listAddresses, listUtxos, sendToAddress, validateAddress } from "../../api/commands";
import { isAppError } from "../../api/types";
import type { AddressType, Balances, Outpoint, SendFeeEstimate, UtxoEntry } from "../../api/types";
import { AddressQr, Card, Identifier, Modal, SatsAmount } from "../../components/ui/display";
import { Button, FeeRateField, SegmentedToggle, TextField } from "../../components/ui/inputs";
import { chosenFeeRate, type FeeChoice, useFeeEstimate } from "../../lib/fee-rate";
import {
  classifySpendType,
  formatFeeRate,
  formatUnitAmount,
  type Unit,
  useUnitAmount,
} from "../../lib/wallet-format";
import { useToastStore } from "../../store/toast";
import { refreshWalletCache } from "../../lib/wallet-sync";
import { usePendingSendsStore } from "../../store/pending-sends";
import { useWalletCacheStore } from "../../store/wallet-cache";
import { FaucetButton } from "../../components/app/FaucetButton";
import { AddressList } from "../../components/app/AddressList";
import { copyText } from "../../lib/clipboard";

function SendPanel() {
  const pushToast = useToastStore((s) => s.push);
  const pushFailure = useToastStore((s) => s.pushFailure);
  const walletSyncStatus = useWalletCacheStore((s) => s.syncStatus);
  const walletSyncError = useWalletCacheStore((s) => s.syncError);
  const [balances, setBalances] = useState<Balances | null>(null);
  const [utxos, setUtxos] = useState<UtxoEntry[]>([]);
  const { fees, failed: feesFailed, retry: loadFees } = useFeeEstimate();
  const [btcPrice, setBtcPrice] = useState<number | null>(null);
  const [btcPriceCached, setBtcPriceCached] = useState(false);
  const {
    unit,
    input: amountInput,
    setInput: setAmountInput,
    changeUnit,
    sats: amountSats,
  } = useUnitAmount(btcPrice);

  const [recipient, setRecipient] = useState("");
  const [recipientValidation, setRecipientValidation] = useState<"idle" | "checking" | "valid" | "invalid">("idle");
  const [recipientError, setRecipientError] = useState<string | undefined>();
  const [feeKey, setFeeKey] = useState<FeeChoice>("fast");
  const [customFeeRate, setCustomFeeRate] = useState("");
  const [selectedOutpoints, setSelectedOutpoints] = useState<Outpoint[]>([]);
  const [sending, setSending] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [sendFee, setSendFee] = useState<SendFeeEstimate | null>(null);
  const [sendFeeError, setSendFeeError] = useState<string | null>(null);
  const recordSend = usePendingSendsStore((s) => s.record);

  const load = useCallback(async () => {
    const [nextBalances, nextUtxos] = await Promise.all([getBalances(), listUtxos()]);
    setBalances(nextBalances);
    setUtxos(nextUtxos);
  }, []);

  useEffect(() => {
    void load().catch((e) => pushFailure(e, "Failed to load wallet data."));
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

  const otherUnits = useMemo(() => (["sats", "btc", "usd"] as Unit[]).filter((u) => u !== unit), [unit]);

  const feeRate = chosenFeeRate(fees, feeKey, customFeeRate);


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

  // Priced when the confirmation opens, from the coins the send would actually spend: the rate
  // alone hides what a typo like 2000 for 20 costs.
  useEffect(() => {
    if (!confirming) return;
    let live = true;
    setSendFee(null);
    setSendFeeError(null);
    estimateSendFee(
      recipient.trim(),
      amountSats,
      feeRate,
      selectedOutpoints.length > 0 ? selectedOutpoints : undefined,
    )
      .then((estimate) => live && setSendFee(estimate))
      .catch((e) => live && setSendFeeError(isAppError(e) ? e.message : "Could not work out the fee."));
    return () => {
      live = false;
    };
  }, [confirming, recipient, amountSats, feeRate, selectedOutpoints]);
  const feeWarning =
    sendFee === null
      ? null
      : sendFee.feeSats > amountSats / 10
        ? `This fee is ${Math.round((sendFee.feeSats / amountSats) * 100)}% of the amount you are sending.`
        : fees?.fast && feeRate > fees.fast * 3
          ? `This rate is ${Math.round(feeRate / fees.fast)}× the chain server's fast estimate.`
          : null;

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
  // The crate refuses to spend both kinds in one manual selection; said here, not at broadcast.
  const mixedKinds = useMemo(() => {
    const kinds = new Set(
      spendableUtxos
        .filter((u) => selectedOutpoints.some((o) => o.txid === u.txid && o.vout === u.vout))
        .map((u) => (classifySpendType(u.spendType) === "Swap" ? "swap" : "regular")),
    );
    return kinds.size > 1;
  }, [spendableUtxos, selectedOutpoints]);
  const canSend =
    !mixedKinds &&
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

  // Its own row, not the Receive card's whole span: opening Recent Addresses grows only the
  // Receive card, which spills into a second row this card never enters.
  return (
    <Card className="flex flex-col gap-4 self-start border-line-strong p-6 lg:h-full">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2.5">
          <span className="flex h-[30px] w-[30px] items-center justify-center rounded-full border border-danger/40 bg-danger/[0.08] text-danger">
            <ArrowUpRight size={15} strokeWidth={2} />
          </span>
          <h2 className="font-header text-[15px] font-bold text-foreground">Send</h2>
        </div>
        <span className="text-[11px] text-subtle">
          Spendable: <SatsAmount sats={balances?.spendable ?? 0} className="text-foreground" />
        </span>
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
        <FeeRateField
          fees={fees}
          failed={feesFailed}
          onRetry={loadFees}
          choice={feeKey}
          onChoice={setFeeKey}
          custom={customFeeRate}
          onCustom={setCustomFeeRate}
        />
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
          {mixedKinds && (
            <p className="text-[11.5px] text-warning">
              Regular and swap coins can't be spent together. Select only one kind.
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
              {/* A send the fee check refused would only fail again on broadcast. */}
              <Button onClick={() => void submitSend()} loading={sending} disabled={!sendFee || sendFeeError !== null}>
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
              <span className="text-[12px] text-muted">Network fee</span>
              <span className="font-numeric text-[12.5px] text-foreground">
                {sendFee ? (
                  <SatsAmount sats={sendFee.feeSats} />
                ) : sendFeeError ? (
                  <span className="text-danger">{sendFeeError}</span>
                ) : (
                  "Working it out…"
                )}
              </span>
            </span>
            {sendFee && (
              <span className="flex items-baseline justify-between gap-3 border-t border-line pt-2.5">
                <span className="text-[12px] text-muted">Total</span>
                <strong className="font-numeric text-[13.5px] text-foreground">
                  <SatsAmount sats={amountSats + sendFee.feeSats} />
                </strong>
              </span>
            )}
            <span className="flex items-baseline justify-between gap-3">
              <span className="text-[12px] text-muted">Inputs</span>
              <span className="text-[12.5px] text-foreground">
                {selectedOutpoints.length > 0
                  ? `${selectedOutpoints.length} chosen by hand`
                  : "Chosen automatically"}
              </span>
            </span>
          </div>
          {feeWarning && <p className="text-[11.5px] leading-5 text-warning">{feeWarning}</p>}
          <p className="text-[11.5px] leading-5 text-subtle">
            The network fee is the most this payment pays; with no change left over it pays a
            little less. Broadcasting cannot be undone.
          </p>
        </Modal>
      )}
    </Card>
  );
}

// Module-level rather than a ref, so a request still in flight when the page is left blocks a
// duplicate from the next visit, not just from this one.
const receiveRequests = new Set<string>();

function ReceivePanel() {
  const pushToast = useToastStore((s) => s.push);
  const [addressType, setAddressType] = useState<AddressType>("p2tr");
  const [pendingType, setPendingType] = useState<AddressType | null>(null);
  const [recentOpen, setRecentOpen] = useState(false);
  // Both the address and the address list live in the wallet cache, stamped with the sync they
  // were read at, so revisiting this page paints from there. They are only asked for again once
  // a newer sync has landed: the address moves on when a synced coin pays it, and the list's
  // balances are the synced coins.
  const syncedAt = useWalletCacheStore((s) => s.lastSuccessfulSyncAt);
  const cachedAddress = useWalletCacheStore((s) => s.receiveAddresses[addressType]);
  const addressList = useWalletCacheStore((s) => s.addressList);
  const current = cachedAddress?.address ?? null;
  const addresses = addressList?.addresses ?? null;

  useEffect(() => {
    if (cachedAddress && cachedAddress.syncedAt === syncedAt) return;
    const key = `${addressType}:${syncedAt ?? "never"}`;
    if (receiveRequests.has(key)) return;
    receiveRequests.add(key);
    const type = addressType;
    setPendingType(type);
    void getNewAddress(type)
      .then((next) => {
        useWalletCacheStore.getState().setReceiveAddress(type, next, syncedAt);
      })
      .catch((e) =>
        pushToast("error", (e as { message?: string })?.message ?? "Failed to get an address."),
      )
      .finally(() => {
        receiveRequests.delete(key);
        setPendingType((p) => (p === type ? null : p));
      });
  }, [addressType, cachedAddress, syncedAt, pushToast]);

  // Only once opened: it feeds a disclosure, and it takes the wallet's read lock.
  useEffect(() => {
    if (!recentOpen) return;
    if (addressList && addressList.syncedAt === syncedAt) return;
    const key = `addresses:${syncedAt ?? "never"}`;
    if (receiveRequests.has(key)) return;
    receiveRequests.add(key);
    void listAddresses()
      .then((list) => useWalletCacheStore.getState().setAddressList(list, syncedAt))
      .catch((e) =>
        pushToast("error", (e as { message?: string })?.message ?? "Failed to list addresses."),
      )
      .finally(() => receiveRequests.delete(key));
  }, [recentOpen, addressList, syncedAt, pushToast]);


  function copyAddress() {
    if (!current) return;
    void copyText(current.address).then((ok) =>
      ok
        ? pushToast("success", "Address copied.")
        : pushToast("warning", "Could not reach the clipboard — select the address and copy it."),
    );
  }

  // Spans two subgrid rows so it matches Send at rest while the opened list grows only this
  // card. Padding sits on the rows: on the card it would land in row two and break the match.
  return (
    <Card className="flex flex-col border-line-strong lg:row-span-2 lg:grid lg:grid-rows-subgrid">
      <div className="flex flex-col gap-4 p-6">
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
            { value: "p2tr", label: "Taproot" },
            { value: "p2wpkh", label: "SegWit" },
          ]}
        />

        <div className="flex justify-center py-1">
          <AddressQr address={current?.address ?? null} alt="Receive address QR code" raised />
        </div>

        <label className="flex flex-col gap-2">
          <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Your Address</span>
          <div className="flex min-h-[46px] items-center justify-between gap-2 rounded-control border border-line-strong bg-surface-raised px-3.5 py-2.5">
            {current ? (
              // `select-all` so one click takes the whole address: this is the value a user
              // falls back to lifting by hand when the clipboard is out of reach.
              <span className="min-w-0">
                <Identifier value={current.address} className="block select-all text-[12.5px] leading-[1.5] text-muted" />
                {current.derivationPath && (
                  <span className="mt-0.5 block font-mono text-[10.5px] text-subtle">
                    {current.derivationPath}
                  </span>
                )}
              </span>
            ) : (
              <span className="font-mono text-[12.5px] text-subtle">
                {pendingType === addressType ? "Generating…" : "—"}
              </span>
            )}
            <button
              type="button"
              onClick={copyAddress}
              disabled={!current}
              aria-label="Copy address"
              className="grid h-[26px] w-[26px] flex-none place-items-center rounded text-subtle hover:bg-primary/10 hover:text-primary disabled:opacity-40"
            >
              <Copy size={13} strokeWidth={2} />
            </button>
          </div>
        </label>

        <button
          type="button"
          aria-expanded={recentOpen}
          onClick={() => setRecentOpen((open) => !open)}
          className="flex items-center gap-1.5 border-t border-dashed border-line pt-3 font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle hover:text-foreground"
        >
          <ChevronDown size={12} strokeWidth={2.5} />
          Addresses
        </button>

        <div className="flex-1" />
      </div>

      {recentOpen && (
        // Pulled up into the row above's bottom padding, so the list sits as close under its
        // toggle as it did inside a <details>.
        <div className="-mt-3 px-6 pb-6">
          <AddressList addresses={addresses} csvName="wallet-addresses.csv" />
        </div>
      )}
    </Card>
  );
}

export function SendPage() {
  return (
    <div className="flex h-full flex-col overflow-y-auto px-8 pb-8 pt-2">
      <div className="shrink-0 pb-4">
        <h1 className="font-header text-[26px] font-bold text-foreground">Send &amp; Receive</h1>
      </div>
      {/* No row gap on wide screens: the Receive card spans both rows, and a gap would open
          inside it under the toggle. */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2 lg:grid-rows-[auto_auto] lg:gap-y-0">
        <SendPanel />
        <ReceivePanel />
      </div>
    </div>
  );
}
