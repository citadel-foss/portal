import {
  ArrowLeft,
  Check,
  CheckCircle2,
  ChevronDown,
  Copy,
  ExternalLink,
  XCircle,
} from "lucide-react";
import { openExternal } from "../../platform";
import { motion } from "framer-motion";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Link } from "react-router-dom";
import type { HTMLAttributes, ReactNode } from "react";
import type { LogLine } from "../../api/types";
import {
  explorerTxUrl,
  LOG_LEVEL_TONE,
  logLevel,
} from "../../lib/wallet-format";
import { walletIdentity } from "../../lib/wallet-identity";

// Keep cards on stable painted layers. Per-card backdrop filters caused WebKit to repeatedly
// re-composite borders and highlights while a scroll container was moving.
//
// Elevation and the hover glow come from the `card-lit` utility — see index.css for why they are
// on `box-shadow` rather than an `::after` layer (this node is `overflow-hidden`, which would clip
// an outer glow on a pseudo-element). The glow matches `lift`'s on hover; unlike `lift` it does not
// translate, because a page-sized panel sliding under the cursor reads as instability rather than
// as response.
//
// The interior blooms this used to carry were wide `blur-3xl` circles, whose gentle falloff
// quantised into concentric contour rings across the panel in WebKit. Accent light now comes
// only from `card-lit`'s box-shadow and the `hairline` border, neither of which bands.
export function Card({
  className = "",
  children,
  ...props
}: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={`relative isolate overflow-hidden rounded-card border bg-[color-mix(in_oklab,var(--color-surface-raised)_74%,transparent)] card-lit ${className}`}
      {...props}
    >
      <div className="pointer-events-none absolute inset-x-0 top-0 -z-10 h-20 bg-gradient-to-b from-white/[0.055] to-transparent" />
      {/* Dithers the sheen above: at 8-bit these near-black gradients quantise into flat
          plateaus tens of pixels tall, and the step between two of them reads as a stray band
          of shadow lying across the panel. Sits above the fill and below the content. */}
      <div className="grain pointer-events-none absolute inset-0 -z-10 opacity-[0.045]" />
      {children}
    </div>
  );
}

/** Shared settings surface established by the router workspace. */
export function SettingsSection({
  title,
  subtitle,
  children,
  headerMeta,
  className = "",
  bodyClassName =
    "grid grid-cols-2 gap-4 p-5 max-[620px]:grid-cols-1",
}: {
  title: string;
  subtitle: string;
  children: ReactNode;
  headerMeta?: ReactNode;
  className?: string;
  bodyClassName?: string;
}) {
  return (
    <Card className={`border-line-strong ${className}`}>
      {headerMeta ? (
        <div className="flex items-start justify-between gap-4 border-b border-line px-5 py-4">
          <div>
            <h2 className="font-header text-[14px] font-bold text-foreground">
              {title}
            </h2>
            <p className="mt-1 text-[11px] text-muted">{subtitle}</p>
          </div>
          {headerMeta}
        </div>
      ) : (
        <div className="border-b border-line px-5 py-4">
          <h2 className="font-header text-[14px] font-bold text-foreground">
            {title}
          </h2>
          <p className="mt-1 text-[11px] text-muted">{subtitle}</p>
        </div>
      )}
      <div className={bodyClassName}>{children}</div>
    </Card>
  );
}

/** One verdict from a connectivity probe. */
export interface TestRow {
  label: string;
  /** `pending` is work still in flight — a cross here would report a failure that hasn't happened. */
  state: "pending" | "ok" | "failed";
  message: string;
}

const TEST_ROW_TONE: Record<TestRow["state"], string> = {
  pending: "text-muted",
  ok: "text-success",
  failed: "text-danger",
};

export function TestResultRows({ rows }: { rows: TestRow[] }) {
  return (
    <div className="flex flex-col gap-1.5">
      {rows.map((r) => (
        <div
          key={r.label}
          className="flex items-center justify-between gap-3 rounded-card border border-line bg-surface-raised px-3 py-2 text-[12px]"
        >
          <span
            className={`flex items-center gap-1.5 font-medium ${TEST_ROW_TONE[r.state]}`}
          >
            {r.state === "pending" ? (
              // Same ring `Button` spins, rather than an icon, so a control and a row that are
              // both waiting look like they are waiting on the same thing.
              <span className="h-3 w-3 animate-spin rounded-full border-2 border-current border-t-transparent opacity-70" />
            ) : r.state === "ok" ? (
              <CheckCircle2 size={13} strokeWidth={2} />
            ) : (
              <XCircle size={13} strokeWidth={2} />
            )}
            {r.label}
          </span>
          <span className="truncate text-subtle">{r.message}</span>
        </div>
      ))}
    </div>
  );
}

interface ModalProps {
  title: string;
  children: ReactNode;
  footer?: ReactNode;
  onClose: () => void;
}

export function Modal({ title, children, footer, onClose }: ModalProps) {
  const panelRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const panel = panelRef.current;
    const focusable = panel?.querySelectorAll<HTMLElement>(
      'button, a[href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
    );
    focusable?.[0]?.focus();
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      if (e.key === "Tab" && focusable?.length) {
        const first = focusable[0];
        const last = focusable[focusable.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      previous?.focus();
    };
  }, [onClose]);

  // Portalled to the body because `AppShell` animates each route inside a wrapper that keeps a
  // `transform` and a `filter`, and either one makes that wrapper the containing block for
  // `position: fixed` — which left a modal opened from a page off-centre with its backdrop
  // covering only that subtree.
  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6">
      <motion.div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        initial={{ opacity: 0, y: 8, scale: 0.98 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
        className="raised max-h-[85vh] w-full max-w-md overflow-y-auto rounded-card border border-line-strong bg-surface p-6"
      >
        <h3 className="font-header text-[15px] font-bold text-foreground">
          {title}
        </h3>
        <div className="mt-4 flex flex-col gap-3">{children}</div>
        {footer && <div className="mt-6 flex justify-end gap-3">{footer}</div>}
      </motion.div>
    </div>,
    document.body,
  );
}

export function SatsAmount({
  sats,
  className = "",
  glyphScale = 0.72,
}: {
  sats: number;
  className?: string;
  glyphScale?: number;
}) {
  return (
    <span
      className={`inline-flex items-baseline gap-1.5 font-numeric tabular-nums ${className}`}
    >
      <span>{Math.round(sats).toLocaleString()}</span>
      <SatsGlyph className="text-subtle" scale={glyphScale} />
    </span>
  );
}

/** Stylized sats glyph (a bar with 3 ticks), mirroring the old app's .cs-sats-symbol. */
export function SatsGlyph({
  className = "",
  scale = 0.72,
}: {
  className?: string;
  scale?: number;
}) {
  return (
    <span
      role="img"
      aria-label="satoshis"
      className={`relative inline-block h-[1em] w-[0.72em] align-middle ${className}`}
      style={{ fontSize: `${scale}em` }}
    >
      <span className="absolute left-1/2 top-0 h-[0.14em] w-[0.14em] -translate-x-1/2 rounded-[1px] bg-current" />
      <span className="absolute left-1/2 bottom-0 h-[0.14em] w-[0.14em] -translate-x-1/2 rounded-[1px] bg-current" />
      <span className="absolute left-[0.04em] right-[0.04em] top-[0.245em] h-[0.1em] rounded-[1px] bg-current" />
      <span className="absolute left-[0.04em] right-[0.04em] top-[0.45em] h-[0.1em] rounded-[1px] bg-current" />
      <span className="absolute left-[0.04em] right-[0.04em] top-[0.655em] h-[0.1em] rounded-[1px] bg-current" />
    </span>
  );
}

/** Collapsed-by-default section, e.g. raw proof dumps or verbose logs nobody needs by default. */
export function Disclosure({
  label,
  defaultOpen = false,
  onOpenChange,
  children,
}: {
  label: string;
  defaultOpen?: boolean;
  /** Notified on every toggle — lets a parent gate work (e.g. a poll) on visibility. */
  onOpenChange?: (open: boolean) => void;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(defaultOpen);
  function toggle() {
    setOpen((o) => {
      onOpenChange?.(!o);
      return !o;
    });
  }
  return (
    <div className="flex flex-col gap-2">
      <button
        type="button"
        onClick={toggle}
        className="flex items-center justify-between gap-2 rounded-control border border-line bg-surface-raised px-3.5 py-2.5 text-[12px] font-medium text-muted outline-none transition-colors hover:text-foreground focus-visible:border-primary/60 focus-visible:shadow-ring active:translate-y-px"
      >
        <span>{label}</span>
        <ChevronDown
          size={15}
          strokeWidth={2}
          className={`transition-transform duration-200 ${open ? "rotate-180" : ""}`}
        />
      </button>
      {open && children}
    </div>
  );
}

/**
 * Scrollable, color-coded log tail. Snaps to the bottom on mount (so opening it — including via
 * `Disclosure`, which mounts fresh each time it's expanded — always lands on the latest line
 * first) and keeps following new lines as long as the viewer is still near the bottom.
 */
export function LogViewer({
  lines,
  emptyMessage = "No log lines yet.",
  className = "min-h-0 flex-1",
  newestFirst = false,
}: {
  lines: LogLine[];
  emptyMessage?: string;
  className?: string;
  /** Newest line at the top, following upward instead of down. */
  newestFirst?: boolean;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  // Not "first render": `lines` is usually still empty then (fetch is async), which would
  // consume the flag before there's anything to scroll to.
  const hasSnapped = useRef(false);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el || lines.length === 0) return;
    if (!hasSnapped.current) {
      hasSnapped.current = true;
      el.scrollTop = newestFirst ? 0 : el.scrollHeight;
      return;
    }
    // Only follow when the reader is already at the growing edge, so scrolling back through
    // history isn't yanked away on the next poll.
    if (newestFirst) {
      if (el.scrollTop < 80) el.scrollTop = 0;
      return;
    }
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    if (distanceFromBottom < 80) el.scrollTop = el.scrollHeight;
  }, [lines, newestFirst]);

  const ordered = newestFirst ? [...lines].reverse() : lines;

  return (
    <div ref={scrollRef} className={`overflow-y-auto px-4.5 py-3 ${className}`}>
      {ordered.length === 0 ? (
        <p className="py-6 text-center text-[13px] text-subtle">
          {emptyMessage}
        </p>
      ) : (
        <div className="flex flex-col gap-0.5">
          {ordered.map((l, i) => (
            <div
              key={`${l.line}:${ordered.slice(0, i).filter((row) => row.line === l.line).length}`}
              className={`rounded-sm px-1 py-0.5 whitespace-pre-wrap break-all font-mono text-[11px] transition-colors hover:bg-[var(--color-hover)] ${LOG_LEVEL_TONE[logLevel(l.line)]}`}
            >
              {l.line}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export function ExternalLinkButton({ txid }: { txid: string }) {
  return (
    <button
      type="button"
      title="View on explorer"
      onClick={(e) => {
        e.stopPropagation();
        void openExternal(explorerTxUrl(txid));
      }}
      aria-label="View transaction on explorer"
      className="flex h-[34px] w-[34px] flex-none items-center justify-center rounded-control border border-line text-muted outline-none transition-colors hover:border-primary/60 hover:bg-primary/[0.14] hover:text-primary-hover focus-visible:shadow-ring active:translate-y-px"
    >
      <ExternalLink size={16} strokeWidth={1.8} />
    </button>
  );
}

/** Pairs with `ExternalLinkButton` on txid/address rows — same 34px footprint. */
export function CopyButton({
  text,
  title = "Copy",
}: {
  text: string;
  title?: string;
}) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const id = setTimeout(() => setCopied(false), 1200);
    return () => clearTimeout(id);
  }, [copied]);

  return (
    <button
      type="button"
      title={title}
      onClick={(e) => {
        e.stopPropagation();
        void navigator.clipboard.writeText(text).then(() => setCopied(true));
      }}
      aria-label={title}
      className={`flex h-[34px] w-[34px] flex-none items-center justify-center rounded-control border outline-none transition-colors focus-visible:shadow-ring active:translate-y-px ${
        copied
          ? "border-success/60 bg-success/[0.14] text-success"
          : "border-line text-muted hover:border-primary/60 hover:bg-primary/[0.14] hover:text-primary-hover"
      }`}
    >
      {copied ? (
        <Check size={16} strokeWidth={2} />
      ) : (
        <Copy size={16} strokeWidth={1.8} />
      )}
    </button>
  );
}

/**
 * Tinted by `walletIdentity`, so a folder of wallets is scannable by colour and monogram before
 * the name is even read. The lift uses `translate3d` deliberately: a plain `translateY` makes
 * WebKit re-rasterize the label and it visibly blurs mid-transition.
 */
export function WalletCard({
  name,
  onClick,
}: {
  name: string;
  onClick: () => void;
}) {
  const id = walletIdentity(name);
  return (
    <button
      type="button"
      onClick={onClick}
      style={{ ["--wallet-glow" as string]: id.glow }}
      className="hairline group relative flex w-60 flex-col items-center gap-3 rounded-card border border-line bg-surface-raised/50 px-6 py-8 text-center outline-none transition-[transform,border-color,background-color] duration-200 ease-[cubic-bezier(0.16,1,0.3,1)] hover:transform-[translate3d(0,-3px,0)] hover:border-line-strong hover:bg-surface-raised/75 focus-visible:border-primary/60 focus-visible:shadow-ring active:transform-[translate3d(0,-1px,0)] active:duration-75
        after:pointer-events-none after:absolute after:inset-0 after:rounded-card after:opacity-0 after:shadow-[0_18px_34px_-14px_rgba(0,0,0,0.85),0_0_22px_-6px_var(--wallet-glow)] after:transition-opacity after:duration-200 hover:after:opacity-100"
    >
      <span
        className="grid h-12 w-12 place-items-center rounded-card border font-header text-[15px] font-bold"
        style={{ color: id.ink, background: id.fill, borderColor: id.edge }}
      >
        {id.monogram}
      </span>
      <span className="max-w-full truncate font-header text-[14px] font-bold text-foreground">
        {name}
      </span>
      <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle transition-colors group-hover:text-muted">
        Unlock
      </span>
    </button>
  );
}

export function MicroLabel({
  children,
  tone = "subtle",
  className = "",
}: {
  children: ReactNode;
  tone?: "subtle" | "muted" | "primary";
  className?: string;
}) {
  const tones = {
    subtle: "text-subtle",
    muted: "text-muted",
    primary: "text-primary",
  };
  return (
    <span
      className={`font-mono text-[10.5px] uppercase tracking-[0.18em] ${tones[tone]} ${className}`}
    >
      {children}
    </span>
  );
}

export function EntityMonogram({
  name,
  size = "md",
}: {
  name: string;
  size?: "sm" | "md" | "lg";
}) {
  const id = walletIdentity(name);
  const dimensions =
    size === "sm"
      ? "h-9 w-9 text-[12px]"
      : size === "lg"
        ? "h-14 w-14 text-[18px]"
        : "h-12 w-12 text-[15px]";
  return (
    <span
      className={`grid flex-none place-items-center rounded-card border font-header font-bold ${dimensions}`}
      style={{ color: id.ink, background: id.fill, borderColor: id.edge }}
    >
      {id.monogram}
    </span>
  );
}

export function StatTile({
  label,
  value,
  detail,
  tone = "primary",
  className = "",
}: {
  label: string;
  value: ReactNode;
  detail?: ReactNode;
  tone?: "primary" | "success" | "warning" | "danger" | "foreground";
  className?: string;
}) {
  const tones = {
    primary: "text-primary",
    success: "text-success",
    warning: "text-warning",
    danger: "text-danger",
    foreground: "text-foreground",
  };
  return (
    <div
      className={`raised rounded-card border border-line-strong bg-surface-raised/70 p-5 ${className}`}
    >
      <MicroLabel>{label}</MicroLabel>
      <strong
        className={`mt-2 block font-numeric text-[24px] font-bold ${tones[tone]}`}
      >
        {value}
      </strong>
      {detail && (
        <span className="mt-1 block text-[12px] text-muted">{detail}</span>
      )}
    </div>
  );
}

// Static so Tailwind can see them; a template string would generate nothing.
const STRIP_COLS: Record<number, string> = {
  2: "grid-cols-2",
  3: "grid-cols-3",
  4: "grid-cols-4",
};

/**
 * A page's headline metrics as one panel divided by hairlines, rather than N separate cards.
 * Cheaper visually than a row of `StatTile`s: one border and one translucent fill, so the
 * page's ambient accent reads through it as a single soft glow instead of N competing ones.
 * Emphasis is carried by `tone`, not by making one cell bigger.
 */
export function StatStrip({
  items,
  className = "",
}: {
  items: {
    label: string;
    value: ReactNode;
    detail?: ReactNode;
    tone?: "foreground" | "primary" | "success" | "warning" | "danger";
  }[];
  className?: string;
}) {
  const tones = {
    foreground: "text-foreground",
    primary: "text-primary",
    success: "text-success",
    warning: "text-warning",
    danger: "text-danger",
  };
  return (
    <div
      className={`grid divide-x divide-line rounded-card border border-line-strong bg-surface-raised/45 max-[780px]:grid-cols-2 max-[780px]:divide-x-0 ${STRIP_COLS[items.length] ?? "grid-cols-4"} ${className}`}
    >
      {items.map(({ label, value, detail, tone = "foreground" }) => (
        <div key={label} className="min-w-0 px-5 py-4">
          <span className="font-mono text-[9px] uppercase tracking-[0.18em] text-subtle">
            {label}
          </span>
          <div className={`mt-1 truncate text-[18px] font-bold ${tones[tone]}`}>
            {value}
          </div>
          {detail && (
            <span className="mt-0.5 block truncate text-[10.5px] text-subtle">
              {detail}
            </span>
          )}
        </div>
      ))}
    </div>
  );
}

export function Notice({
  tone = "primary",
  icon,
  children,
  action,
  className = "",
}: {
  tone?: "primary" | "success" | "warning" | "danger";
  icon?: ReactNode;
  children: ReactNode;
  action?: ReactNode;
  className?: string;
}) {
  const tones = {
    primary: "border-primary/45 bg-primary/[0.08] text-primary",
    success: "border-success/45 bg-success/[0.08] text-success",
    warning: "border-warning/45 bg-warning/[0.08] text-warning",
    danger: "border-danger/45 bg-danger/[0.08] text-danger",
  };
  return (
    <div
      className={`flex items-center gap-3 rounded-card border px-4 py-3 text-[12px] ${tones[tone]} ${className}`}
    >
      {icon}
      {<div className="min-w-0 flex-1 text-foreground">{children}</div>}
      {action}
    </div>
  );
}

export function StatusChip({
  tone = "primary",
  shape = "pill",
  icon,
  children,
  className = "",
}: {
  tone?: "primary" | "success" | "warning" | "danger" | "subtle";
  shape?: "pill" | "tile" | "banner" | "dot";
  icon?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  const radius =
    shape === "pill"
      ? "rounded-pill"
      : shape === "tile"
        ? "rounded-control"
        : "rounded-card";
  const tones = {
    primary: "border-primary/45 bg-primary/[0.08] text-primary",
    success: "border-success/45 bg-success/[0.08] text-success",
    warning: "border-warning/45 bg-warning/[0.08] text-warning",
    danger: "border-danger/45 bg-danger/[0.08] text-danger",
    subtle: "border-line bg-white/[0.04] text-subtle",
  };
  const textTones = {
    primary: "text-primary",
    success: "text-success",
    warning: "text-warning",
    danger: "text-danger",
    subtle: "text-subtle",
  };
  const dots = {
    primary: "bg-primary",
    success: "bg-success",
    warning: "bg-warning",
    danger: "bg-danger",
    subtle: "bg-subtle",
  };
  if (shape === "dot")
    return (
      <span
        className={`inline-flex items-center gap-2 font-mono text-[10.5px] uppercase tracking-[0.18em] ${textTones[tone]} ${className}`}
      >
        <span className={`h-1.5 w-1.5 rounded-full ${dots[tone]}`} />
        {children}
      </span>
    );
  return (
    <span
      className={`inline-flex items-center gap-1.5 border px-2 py-1 font-mono text-[9.5px] uppercase tracking-[0.08em] ${tones[tone]} ${radius} ${className}`}
    >
      {icon}
      {children}
    </span>
  );
}

export function EmptyState({
  icon,
  title,
  description,
  action,
  size = "md",
}: {
  icon?: ReactNode;
  title: string;
  description?: ReactNode;
  action?: ReactNode;
  size?: "sm" | "md" | "lg";
}) {
  return (
    <div
      className={`grid place-items-center text-center ${size === "sm" ? "p-5" : size === "lg" ? "p-10" : "p-8"}`}
    >
      <div>
        {icon && <div className="mx-auto mb-3 w-fit text-primary">{icon}</div>}
        <strong className="block text-[14px] text-foreground">{title}</strong>
        {description && (
          <p className="mt-1 text-[12px] text-muted">{description}</p>
        )}
        {action && <div className="mt-5">{action}</div>}
      </div>
    </div>
  );
}

export function SkeletonLines({ count = 8 }: { count?: number }) {
  return (
    <div className="space-y-2 p-4" aria-label="Loading">
      <span className="sr-only">Loading</span>
      {Array.from({ length: count }, (_, i) => (
        <div
          key={i}
          className="h-3 animate-pulse rounded-sm bg-white/[0.07]"
          style={{ width: `${72 + (i % 4) * 7}%` }}
        />
      ))}
    </div>
  );
}

/**
 * Placeholder for a list of records still being fetched, shaped like the rows that will
 * replace them so the panel does not jump when they arrive. Widths vary per row so the
 * block reads as content rather than as a progress bar.
 */
export function SkeletonRows({ count = 4 }: { count?: number }) {
  return (
    <div className="divide-y divide-line" aria-label="Loading">
      <span className="sr-only">Loading</span>
      {Array.from({ length: count }, (_, i) => (
        <div
          key={i}
          className="grid min-h-[58px] grid-cols-[38px_minmax(0,1fr)_auto] items-center gap-3 py-2.5"
        >
          <span className="h-[34px] w-[34px] animate-pulse rounded-control bg-white/[0.07]" />
          <span className="flex min-w-0 flex-col gap-1.5">
            <span
              className="h-3 animate-pulse rounded-sm bg-white/[0.07]"
              style={{ width: `${58 + (i % 3) * 9}%` }}
            />
            <span className="h-2.5 w-16 animate-pulse rounded-sm bg-white/[0.05]" />
          </span>
          <span className="flex flex-col items-end gap-1.5">
            <span className="h-3 w-20 animate-pulse rounded-sm bg-white/[0.07]" />
            <span className="h-2.5 w-12 animate-pulse rounded-sm bg-white/[0.05]" />
          </span>
        </div>
      ))}
    </div>
  );
}

export function IndeterminateBar() {
  return (
    <div className="h-1 overflow-hidden rounded-pill bg-primary/10">
      <span className="block h-full w-1/2 animate-[market-progress_1.5s_ease-in-out_infinite] rounded-pill bg-primary" />
    </div>
  );
}

export function AmountTile({
  label,
  children,
  tone = "success",
  className = "",
}: {
  label: string;
  children: ReactNode;
  tone?: "primary" | "success" | "warning";
  className?: string;
}) {
  const tones = {
    primary: "border-primary/35 bg-primary/[0.07] text-primary",
    success: "border-success/35 bg-success/[0.07] text-success",
    warning: "border-warning/35 bg-warning/[0.07] text-warning",
  };
  return (
    <div
      className={`raised rounded-card border px-4 py-3 text-center ${tones[tone]} ${className}`}
    >
      <MicroLabel>{label}</MicroLabel>
      <div className="mt-1 font-numeric text-[19px] font-bold">{children}</div>
    </div>
  );
}

export function BackButton({
  to,
  label = "Back",
}: {
  to: string;
  label?: string;
}) {
  return (
    <Link
      to={to}
      aria-label={label}
      className="grid h-9 w-9 place-items-center rounded-control border border-line bg-surface-raised text-muted outline-none transition-colors hover:border-line-strong hover:text-foreground focus-visible:shadow-ring active:translate-y-px"
    >
      <ArrowLeft size={16} />
    </Link>
  );
}

export function IconButton({
  icon,
  label,
  onClick,
  disabled,
  size = "md",
  className = "",
}: {
  icon: ReactNode;
  label: string;
  onClick?: () => void;
  disabled?: boolean;
  size?: "sm" | "md";
  className?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={label}
      aria-label={label}
      className={`inline-grid place-items-center rounded-control border border-line bg-surface-raised text-muted shadow-[inset_0_1px_0_rgba(255,255,255,0.05)] outline-none transition-[box-shadow,background-color,border-color,transform,color] hover:border-line-strong hover:bg-[var(--color-hover)] hover:text-foreground focus-visible:shadow-ring active:translate-y-px disabled:cursor-not-allowed disabled:opacity-40 ${size === "sm" ? "h-8 w-8" : "h-9 w-9"} ${className}`}
    >
      {icon}
    </button>
  );
}

export function Tooltip({
  content,
  align = "center",
  children,
}: {
  content: ReactNode;
  /** `right` anchors the panel's right edge to the trigger so triggers near a
   *  clipping container's right edge don't have half the tooltip cut off. */
  align?: "center" | "right";
  children: ReactNode;
}) {
  return (
    <span className="group relative inline-flex">
      {children}
      <span
        role="tooltip"
        className={`pointer-events-none absolute top-[calc(100%+9px)] z-20 w-max max-w-[260px] translate-y-1.5 rounded-card border border-line-strong bg-bg px-2.5 py-2 text-left text-[11.5px] font-medium normal-case leading-snug tracking-normal text-foreground opacity-0 shadow-lg transition-[opacity,transform] group-hover:translate-y-0 group-hover:opacity-100 group-focus-within:translate-y-0 group-focus-within:opacity-100 ${align === "right" ? "right-0" : "left-1/2 -translate-x-1/2"}`}
      >
        {content}
      </span>
    </span>
  );
}
