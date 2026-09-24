/**
 * The pieces both swap reports are built from.
 *
 * A wallet's report and a router's report describe the same swap from the two ends of it, so
 * they are one layout with different figures in it, not two designs. Everything structural
 * lives here; each page supplies only what its own side of the swap actually knows.
 */
import { AlertTriangle, CheckCircle2, RefreshCw, Timer, XCircle } from "lucide-react";
import { useState } from "react";
import type { ReactNode } from "react";

import type { ReportUtxo, SwapStatus } from "../../api/types";
import { isAppError } from "../../api/types";
import { formatDuration, SATS_PER_BTC, scriptTypeFromAddress, swapStatusPresentation } from "../../lib/wallet-format";
import {
  BackButton,
  Card,
  CopyButton,
  Disclosure,
  ExternalLinkButton,
  Identifier,
  IndeterminateBar,
  SatsAmount,
} from "./display";
import { Button } from "./inputs";

export const STATUS_LABEL: Record<SwapStatus, string> = {
  success: "Completed",
  recovery_hashlock: "Recovered (hashlock)",
  recovery_timelock: "Recovered (timelock)",
  recovered: "Interrupted · recovered",
  interrupted: "Interrupted · recovering",
  unfinished: "Never finished",
  failed: "Failed",
};

/** One accent per hop so a funding tx is visually tied to the router it funded. */
export const HOP_ACCENTS = [
  "var(--color-primary)",
  "var(--color-info)",
  "var(--color-router)",
  "var(--color-success)",
];
export const OUTGOING_ACCENT = "var(--color-warning)";

export function satsToBtc(sats: number): string {
  return (sats / SATS_PER_BTC).toFixed(8);
}

export function formatTimestamp(unixSeconds: number): string {
  if (!unixSeconds) return "—";
  return new Date(unixSeconds * 1000).toLocaleString();
}

/**
 * The crate writes the literal string "Unknown" when it cannot resolve an address, and its
 * Electrum backend never can — that backend builds every UTXO entry with `address: None`
 * (`wallet/blockchain/electrum.rs`). Printing that word where an address belongs tells the
 * reader nothing, so it is treated as absent everywhere it could reach the page.
 */
export function identifiedAddress(utxo: { address: string }): string | null {
  const address = utxo.address?.trim();
  return address && address !== "Unknown" ? address : null;
}

/** Full txid, not truncated — the whole point of this row is being able to read and copy it. */
export function TxArtifact({ label, caption, txid, vout, amountSats, accent, arrow }: {
  label: string;
  caption?: string;
  txid: string;
  vout?: number;
  amountSats?: number;
  accent: string;
  arrow: string;
}) {
  // `txid:vout` names the coin, which is what the contract actually holds; a bare txid only
  // names the transaction that created it.
  const reference = vout === undefined ? txid : `${txid}:${vout}`;
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_34px_34px] items-center gap-2.5 rounded-control border border-line bg-surface-raised p-5">
      <div className="min-w-0">
        <h4 className="mb-3.5 flex items-center gap-3 text-[15px] font-extrabold text-foreground">
          <span className="font-mono" style={{ color: accent }} aria-hidden>
            {arrow}
          </span>
          {label}
        </h4>
        <p className="break-all font-mono text-[12px] leading-relaxed text-muted">{reference}</p>
        {amountSats !== undefined && (
          <p className="mt-2 font-numeric text-[13px] text-foreground">
            <SatsAmount sats={amountSats} />
          </p>
        )}
        {caption && <p className="mt-2 text-[11.5px] leading-5 text-subtle">{caption}</p>}
      </div>
      <CopyButton text={reference} title={vout === undefined ? "Copy transaction ID" : "Copy outpoint"} />
      <ExternalLinkButton txid={txid} />
    </div>
  );
}

export function CoinRow({ label, caption, coins, accent, arrow }: {
  label: string;
  caption: string;
  coins: ReportUtxo[];
  accent: string;
  arrow: string;
}) {
  return (
    <div className="rounded-control border border-line bg-surface-raised p-5">
      <h4 className="mb-3.5 flex items-center gap-3 text-[15px] font-extrabold text-foreground">
        <span className="font-mono" style={{ color: accent }} aria-hidden>
          {arrow}
        </span>
        {label}
      </h4>
      <div className="flex flex-col gap-3">
        {coins.map((coin, i) => {
          const address = identifiedAddress(coin);
          return (
            <div
              key={`${coin.address}-${i}`}
              className="grid grid-cols-[minmax(0,1fr)_34px] items-start gap-2.5"
            >
              <div className="min-w-0">
                {address && (
                  <Identifier value={address} className="block text-[12px] leading-relaxed text-muted" />
                )}
                <p className={`flex items-center gap-2${address ? " mt-2" : ""}`}>
                  {address && (
                    <span className="rounded-control border border-line px-1.5 py-0.5 font-mono text-[9px] text-subtle">
                      {scriptTypeFromAddress(address)}
                    </span>
                  )}
                  <span className="font-numeric text-[13px] text-foreground">
                    <SatsAmount sats={coin.valueSats} />
                  </span>
                </p>
              </div>
              {address && <ExternalLinkButton address={address} />}
            </div>
          );
        })}
      </div>
      <p className="mt-3 text-[11.5px] leading-5 text-subtle">{caption}</p>
    </div>
  );
}

export function SectionCard({ title, children }: { title: string; children: ReactNode }) {
  return (
    <Card className="flex flex-col gap-3 border-line-strong p-5">
      <h3 className="font-header text-[14px] font-bold text-foreground">{title}</h3>
      {children}
    </Card>
  );
}

export function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-3 text-[12px]">
      <span className="text-subtle">{label}</span>
      <span className="text-right font-mono text-foreground">{children}</span>
    </div>
  );
}

export function TxidRow({ label, txid }: { label: string; txid: string }) {
  return (
    <div className="flex items-center justify-between gap-3 rounded-control border border-line bg-surface-raised px-3 py-2">
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">{label}</span>
        <Identifier value={txid} className="text-[11.5px] leading-[1.45] text-muted" />
      </span>
      <ExternalLinkButton txid={txid} />
    </div>
  );
}

/** The back link, the outcome and the swap's own id — identical on both reports. */
export function ReportHeader({ backTo, backLabel, swapId, status }: {
  backTo: string;
  backLabel: string;
  swapId: string;
  status: SwapStatus;
}) {
  const { Icon, tone, label } = swapStatusPresentation(status);
  return (
    <div className="flex shrink-0 items-center gap-3 pb-4">
      <BackButton to={backTo} label={backLabel} />
      <div className="flex items-center gap-2.5">
        <Icon size={22} strokeWidth={2} className={tone} />
        <div>
          <h1 className="break-all font-header text-[20px] font-bold leading-[1.3] text-foreground">{swapId}</h1>
          <p className={`mt-0.5 text-[11.5px] font-medium ${tone}`}>{STATUS_LABEL[status] ?? label}</p>
        </div>
      </div>
    </div>
  );
}

/** Shown for every outcome that is not a completed swap; all of them can leave funds on-chain. */
export function ReportFailureBanner({ errorMessage, action }: {
  errorMessage?: string;
  action?: ReactNode;
}) {
  return (
    <div className="mb-4 flex shrink-0 flex-wrap items-start justify-between gap-3 rounded-control border border-danger/35 bg-danger/[0.06] px-4 py-3.5">
      <div className="flex min-w-0 items-start gap-3">
        <AlertTriangle size={18} strokeWidth={2} className="mt-0.5 flex-none text-danger" />
        <span className="flex min-w-0 flex-col gap-1">
          <strong className="text-[13px] font-semibold text-foreground">
            {errorMessage ? "Failure reason" : "This swap did not complete"}
          </strong>
          {errorMessage ? (
            <span className="break-words font-mono text-[11.5px] leading-relaxed text-danger">{errorMessage}</span>
          ) : (
            <span className="text-[11.5px] leading-relaxed text-muted">No reason was recorded for this one.</span>
          )}
        </span>
      </div>
      {action}
    </div>
  );
}

/** The one figure that names the swap, with what it cost or earned underneath it. */
export function ReportHero({ label, amountSats, secondary, network, durationSeconds, startTimestamp, endTimestamp }: {
  label: string;
  amountSats: number;
  secondary?: ReactNode;
  network: string;
  durationSeconds: number;
  startTimestamp: number;
  endTimestamp: number;
}) {
  return (
    <Card className="grid justify-items-center border-line-strong px-5 py-14 text-center">
      <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">{label}</span>
      <SatsAmount
        sats={amountSats}
        glyphScale={0.5}
        className="my-4 text-[clamp(38px,6vw,58px)] leading-none text-foreground"
      />
      <p className="mb-6 font-mono text-[14px] text-muted">≈ {satsToBtc(amountSats)} BTC</p>
      {secondary && <p className="-mt-3 mb-6 font-mono text-[13px] text-subtle">{secondary}</p>}
      <div className="flex flex-wrap items-center justify-center gap-2.5">
        <span className="inline-flex items-center gap-2 rounded-full border border-primary/45 bg-primary/[0.12] px-4.5 py-2.5 font-mono text-[12px] uppercase tracking-[0.08em] text-primary-hover">
          <Timer size={15} strokeWidth={1.8} />
          Duration {formatDuration(durationSeconds)}
        </span>
        <span className="inline-flex items-center rounded-full border border-line-strong bg-surface-raised px-4.5 py-2.5 font-mono text-[12px] uppercase tracking-[0.08em] text-muted">
          {network}
        </span>
      </div>
      <p className="mt-5 font-mono text-[11px] text-subtle">
        {formatTimestamp(startTimestamp)} → {formatTimestamp(endTimestamp)}
      </p>
    </Card>
  );
}

/**
 * Renders whatever the crate's `DeniabilityProof` JSON happens to contain, without hardcoding
 * field names — the Taproot and Legacy variants differ and the shape may evolve upstream.
 */
export function JsonEntries({ value, depth = 0 }: { value: unknown; depth?: number }) {
  if (value === null || value === undefined) return <span className="text-subtle">—</span>;

  if (typeof value === "string") {
    return <Identifier value={value} className="text-[11px] leading-[1.45] text-muted" />;
  }
  if (typeof value === "number" || typeof value === "boolean") {
    return <span className="font-mono text-[11px] text-muted">{String(value)}</span>;
  }

  if (Array.isArray(value)) {
    return (
      <div className="flex flex-col gap-1.5" style={{ marginLeft: depth > 0 ? 12 : 0 }}>
        {value.map((item, i) => (
          <div key={i} className="flex items-start gap-2">
            <span className="font-mono text-[10px] text-subtle">[{i}]</span>
            <JsonEntries value={item} depth={depth + 1} />
          </div>
        ))}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-1.5" style={{ marginLeft: depth > 0 ? 12 : 0 }}>
      {Object.entries(value as Record<string, unknown>).map(([key, val]) => (
        <div key={key} className="flex flex-col gap-0.5">
          <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">{key}</span>
          <JsonEntries value={val} depth={depth + 1} />
        </div>
      ))}
    </div>
  );
}

/**
 * Verify-on-chain plus the proof itself. `verify` resolves true when the proof checks out;
 * both sides of a swap hold their own proof and check it the same way.
 */
export function DeniabilityCard({ swapId, proof, verify }: {
  swapId: string;
  proof: Record<string, unknown> | null;
  verify: () => Promise<boolean>;
}) {
  const [verifying, setVerifying] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; message: string } | null>(null);

  async function run() {
    setVerifying(true);
    setResult(null);
    try {
      const ok = await verify();
      setResult({
        ok,
        message: ok
          ? "The proof's signatures and contract details check out against the blockchain."
          : "The proof did not verify against the blockchain — it may be incomplete or the contract may not have been observed on-chain.",
      });
    } catch (e) {
      setResult({ ok: false, message: isAppError(e) ? e.message : "Failed to verify deniability proof." });
    } finally {
      setVerifying(false);
    }
  }

  return (
    <SectionCard title="Deniability Proof">
      {proof ? (
        <>
          <div className="flex items-center gap-2.5">
            <Button size="sm" variant="secondary" onClick={() => void run()} loading={verifying}>
              Verify on-chain
            </Button>
          </div>
          {verifying && (
            <div className="flex flex-col gap-2">
              <IndeterminateBar />
              <span className="text-[11.5px] text-subtle">Fetching the contract transaction from the chain…</span>
            </div>
          )}
          {result && !verifying && (
            <div
              className={`flex items-start gap-2 rounded-control border px-3.5 py-2.5 text-[12px] ${
                result.ok
                  ? "border-success/35 bg-success/[0.06] text-success"
                  : "border-danger/35 bg-danger/[0.06] text-danger"
              }`}
            >
              {result.ok ? (
                <CheckCircle2 size={14} strokeWidth={2} className="mt-0.5 flex-none" />
              ) : (
                <XCircle size={14} strokeWidth={2} className="mt-0.5 flex-none" />
              )}
              <span>
                <strong className="font-semibold">{result.ok ? "Verified on-chain." : "Verification failed."}</strong>{" "}
                {result.message}
              </span>
            </div>
          )}
          <Disclosure label="Show proof details">
            <div className="flex flex-col gap-2 rounded-control border border-line bg-surface-raised px-3.5 py-3">
              {/* The swap this proof belongs to leads it: the panel is reachable from a list,
                  and a wall of hashes with no name on it identifies nothing. */}
              <div className="flex flex-col gap-0.5 border-b border-dashed border-line pb-2">
                <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Swap id</span>
                <Identifier value={swapId} className="text-[11px] leading-[1.45] text-muted" />
              </div>
              <JsonEntries value={proof} />
            </div>
          </Disclosure>
        </>
      ) : (
        <p className="text-[12px] text-subtle">No deniability proof was generated for this swap.</p>
      )}
    </SectionCard>
  );
}

/** Loading and missing states, so neither report invents its own. */
export function ReportLoading({ what }: { what: string }) {
  return (
    <div className="grid h-full place-items-center gap-2.5 text-center text-[13px] text-subtle">
      <RefreshCw size={28} strokeWidth={1.6} className="animate-spin text-primary" />
      <span>Loading {what}…</span>
    </div>
  );
}
