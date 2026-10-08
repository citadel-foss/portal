import { ArrowRight, Check, Eye, EyeOff } from "lucide-react";
import { motion } from "framer-motion";
import { Link, type LinkProps } from "react-router-dom";
import type { FeeEstimate } from "../../api/types";
import { chosenFeeRate, FEE_TIERS, MAX_FEE_RATE, type FeeChoice } from "../../lib/fee-rate";
import { formatFeeRate } from "../../lib/wallet-format";
import {
  useId,
  useState,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
} from "react";

type Variant = "primary" | "secondary" | "ghost";
type Size = "md" | "sm";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
  loading?: boolean;
  arrow?: boolean; // trailing arrow, e.g. "Test node connection →"
}

const buttonBase =
  "inline-flex items-center justify-center gap-2 rounded-control font-semibold outline-none transition-[box-shadow,background-color,border-color,transform,color] duration-200 disabled:cursor-not-allowed disabled:opacity-50";

const buttonVariants: Record<Variant, string> = {
  // Top-lit: a 1px inner highlight plus an accent-tinted drop shadow, so a filled control reads
  // as raised rather than as a coloured rectangle. Presses translate down and swap the outer
  // shadow for an inner one — the shadow is what sells the press, not the 1px move.
  primary:
    "bg-primary text-on-primary shadow-[inset_0_1px_0_rgba(255,255,255,0.22),0_1px_2px_rgba(0,0,0,0.4),0_8px_20px_-8px_color-mix(in_oklab,var(--color-primary)_50%,transparent)] hover:bg-primary-hover hover:shadow-[inset_0_1px_0_rgba(255,255,255,0.28),0_1px_2px_rgba(0,0,0,0.4),0_12px_28px_-8px_color-mix(in_oklab,var(--color-primary)_68%,transparent)] focus-visible:shadow-[inset_0_1px_0_rgba(255,255,255,0.22),var(--shadow-ring)] active:translate-y-px active:shadow-[inset_0_1px_3px_rgba(0,0,0,0.28)] disabled:shadow-none",
  secondary:
    "border border-line bg-surface-raised text-foreground shadow-[inset_0_1px_0_rgba(255,255,255,0.05)] hover:border-line-strong hover:bg-secondary-hover focus-visible:border-primary/60 focus-visible:shadow-ring active:translate-y-px",
  ghost:
    "text-muted hover:text-foreground focus-visible:shadow-ring active:translate-y-px",
};

const buttonSizes: Record<Size, string> = {
  md: "h-10 px-5 text-[13px]",
  sm: "h-8 px-4 text-[12px]",
};

export function Button({
  variant = "primary",
  size = "md",
  loading = false,
  arrow = false,
  disabled,
  className = "",
  children,
  ...props
}: ButtonProps) {
  return (
    <button
      className={`${buttonBase} ${buttonVariants[variant]} ${buttonSizes[size]} ${className}`}
      disabled={disabled || loading}
      {...props}
    >
      {loading && (
        <span className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-current border-t-transparent opacity-70" />
      )}
      {children}
      {arrow && !loading && <ArrowRight size={15} strokeWidth={2} />}
    </button>
  );
}

export function LinkButton({
  variant = "primary",
  size = "md",
  className = "",
  children,
  ...props
}: LinkProps & { variant?: Variant; size?: Size; className?: string }) {
  return (
    <Link
      className={`${buttonBase} ${buttonVariants[variant]} ${buttonSizes[size]} ${className}`}
      {...props}
    >
      {children}
    </Link>
  );
}

/**
 * A settled value that can be edited in place. Deliberately not a `TextField`: a form of
 * bordered boxes reads as decisions demanded up front, which is the thing this avoids — at
 * rest the row is information, and the input only appears once asked for.
 */
export function SummaryRow({
  label,
  value,
  display,
  suffix,
  hint,
  readOnly = false,
  inputMode = "numeric",
  placeholder,
  secret = false,
  onCommit,
}: {
  label: string;
  /** The raw text, and what an edit starts from. */
  value: string;
  /** Prettified form shown at rest, e.g. thousands separators. Editing still uses `value`. */
  display?: string;
  /** Unit rendered after the value, e.g. "sats" — never part of the edited text. */
  suffix?: string;
  hint?: string;
  readOnly?: boolean;
  inputMode?: "numeric" | "decimal" | "text";
  /** Shown while `value` is empty — at rest and in the edit box. */
  placeholder?: string;
  /** Masks the value at rest and while typing, for credentials. */
  secret?: boolean;
  onCommit?: (next: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);

  function open() {
    setDraft(value);
    setEditing(true);
  }

  function commit() {
    setEditing(false);
    const next = draft.trim();
    if (next && next !== value) onCommit?.(next);
  }

  return (
    <div className="flex min-h-9 items-center justify-between gap-4 text-[12.5px]">
      {/* Basis 0: the label takes only what the value leaves, so it wraps before the value does. */}
      <span className="flex-1 text-muted">{label}</span>
      {editing && !readOnly ? (
        <input
          autoFocus
          type={secret ? "password" : "text"}
          autoComplete={secret ? "off" : undefined}
          inputMode={inputMode}
          placeholder={placeholder}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
            // Blur would otherwise fire after this and commit the abandoned draft.
            if (e.key === "Escape") {
              setDraft(value);
              setEditing(false);
            }
          }}
          // Sized to the value, not the column: a full-width box is the chrome being avoided.
          className="w-28 border-b border-primary bg-transparent pb-0.5 text-right font-mono text-[12.5px] text-foreground outline-none"
        />
      ) : readOnly ? (
        <span className="text-right">
          <span className="font-numeric text-foreground">
            {display ?? value}
            {suffix && <span className="ml-1 text-subtle">{suffix}</span>}
          </span>
          {hint && (
            <span className="mt-0.5 block text-[10px] text-subtle">{hint}</span>
          )}
        </span>
      ) : (
        <button
          type="button"
          onClick={open}
          className="group flex min-w-0 items-baseline gap-2 rounded-sm text-right outline-none transition-colors hover:text-primary focus-visible:shadow-[inset_0_0_0_1px_color-mix(in_oklab,var(--color-primary)_45%,transparent)]"
        >
          <span className="min-w-0 wrap-anywhere font-mono text-foreground group-hover:text-primary">
            {value ? (secret ? "•".repeat(8) : (display ?? value)) : placeholder}
            {suffix && (
              <span className="ml-1 text-subtle group-hover:text-primary">
                {suffix}
              </span>
            )}
          </span>
          <span className="shrink-0 text-[10px] uppercase tracking-widest text-subtle group-hover:text-primary">
            Edit
          </span>
        </button>
      )}
    </div>
  );
}

export function SummaryGroup({
  title,
  warning,
  children,
}: {
  title: string;
  warning?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section>
      <div className="mb-2 flex items-center justify-between gap-3">
        <h3 className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
          {title}
        </h3>
        {warning}
      </div>
      <div className="divide-y divide-line rounded-card border border-line bg-surface/45 px-4 [&>*]:py-2">
        {children}
      </div>
    </section>
  );
}

export function PresetTile({
  selected,
  onClick,
  label,
  value,
  size = "md",
}: {
  selected: boolean;
  onClick: () => void;
  label: string;
  value?: ReactNode;
  size?: "sm" | "md";
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={selected}
      className={`lift relative flex flex-col items-start rounded-card border text-left outline-none focus-visible:shadow-ring ${size === "sm" ? "px-3 py-2" : "px-4 py-3"} ${selected ? "border-primary/60 bg-primary/10" : "border-line-strong bg-surface-raised hover:border-primary/35"}`}
    >
      <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
        {label}
      </span>
      {value && (
        <strong className="mt-1 font-numeric text-[13px] text-foreground">
          {value}
        </strong>
      )}
    </button>
  );
}

/** Fast / Medium / Slow from the chain server's estimator, or a custom rate. */
export function FeeRateField({
  fees,
  failed,
  onRetry,
  choice,
  onChoice,
  custom,
  onCustom,
}: {
  fees: FeeEstimate | null;
  failed: boolean;
  onRetry: () => void;
  choice: FeeChoice;
  onChoice: (choice: FeeChoice) => void;
  custom: string;
  onCustom: (value: string) => void;
}) {
  const shown = (rate: number) =>
    rate > 0 ? `${formatFeeRate(rate)} s/vB` : fees === null && !failed ? "…" : "—";
  return (
    <div className="flex flex-col gap-2">
      <div className="grid grid-cols-4 gap-2">
        {FEE_TIERS.map(({ key, label }) => (
          <PresetTile
            key={key}
            size="sm"
            label={label}
            value={shown(chosenFeeRate(fees, key, ""))}
            selected={choice === key}
            onClick={() => onChoice(key)}
          />
        ))}
        <PresetTile
          size="sm"
          label="Custom"
          value={shown(chosenFeeRate(fees, "custom", custom))}
          selected={choice === "custom"}
          onClick={() => onChoice("custom")}
        />
      </div>
      {failed && (
        <div className="flex items-center justify-between gap-3">
          <span className="text-[11.5px] text-subtle">
            Could not read a fee estimate from the chain server.
          </span>
          <Button size="sm" variant="ghost" onClick={onRetry}>
            Try again
          </Button>
        </div>
      )}
      {choice === "custom" && (
        <TextField
          label="Custom rate (sats/vB)"
          inputMode="decimal"
          placeholder="e.g. 8"
          value={custom}
          onChange={(e) => onCustom(e.target.value)}
          error={
            Number(custom) > MAX_FEE_RATE
              ? `At most ${MAX_FEE_RATE} sats/vB; anything higher is almost certainly a typo.`
              : undefined
          }
          hint="Decimal rates are rounded up to the next whole sat/vB."
        />
      )}
    </div>
  );
}

export function CheckRow({
  checked,
  onToggle,
  primary,
  secondary,
  badge,
}: {
  checked: boolean;
  onToggle: (checked: boolean) => void;
  primary: ReactNode;
  secondary?: ReactNode;
  badge?: ReactNode;
}) {
  const id = useId();
  return (
    <label
      htmlFor={id}
      className={`flex cursor-pointer items-center justify-between gap-3 rounded-control border px-3 py-2 outline-none transition-colors focus-within:shadow-ring ${checked ? "border-primary/45 bg-primary/[0.06]" : "border-line bg-surface-raised hover:bg-[var(--color-hover)]"}`}
    >
      <span className="flex min-w-0 items-center gap-3">
        {/* The native control is kept for semantics and keyboard handling but taken out of the
            paint: WebKit ignores `accent-color` on an unchecked box and fills it white, which on
            this dark surface reads as a broken control rather than an empty one. */}
        <span className="relative grid h-4 w-4 flex-none place-items-center">
          <input
            id={id}
            type="checkbox"
            checked={checked}
            onChange={(e) => onToggle(e.target.checked)}
            className="peer absolute inset-0 h-4 w-4 cursor-pointer appearance-none rounded-[4px] border border-line-strong bg-surface outline-none checked:border-primary checked:bg-primary focus-visible:shadow-ring"
          />
          <Check
            size={11}
            strokeWidth={3}
            className="pointer-events-none relative hidden text-on-primary peer-checked:block"
          />
        </span>
        <span className="min-w-0">
          <strong className="block truncate text-[12px] text-foreground">
            {primary}
          </strong>
          {secondary && (
            <span className="mt-0.5 block truncate text-[10.5px] text-subtle">
              {secondary}
            </span>
          )}
        </span>
      </span>
      {badge}
    </label>
  );
}

interface TextFieldProps extends InputHTMLAttributes<HTMLInputElement> {
  label: string;
  error?: string;
  hint?: string;
}

function FieldLabel({
  htmlFor,
  label,
  required,
}: {
  htmlFor: string;
  label: string;
  required?: boolean;
}) {
  return (
    <label htmlFor={htmlFor} className="text-[12.5px] font-medium text-muted">
      {label}
      {required && (
        <span className="ml-1 text-danger" aria-hidden="true">
          *
        </span>
      )}
    </label>
  );
}

export function TextField({
  label,
  error,
  hint,
  id,
  required,
  className = "",
  ...props
}: TextFieldProps) {
  const inputId = id ?? useId();
  return (
    <div className="flex flex-col gap-1.5">
      <FieldLabel htmlFor={inputId} label={label} required={required} />
      <input
        id={inputId}
        required={required}
        className={`h-10 rounded-control border bg-surface-raised px-3 text-[13px] text-foreground outline-none transition-colors duration-200 placeholder:text-subtle ${
          error
            ? "border-danger bg-danger/5"
            : "border-line focus:border-primary focus:shadow-ring"
        } ${className}`}
        {...props}
      />
      {error ? (
        <span className="text-[11.5px] text-danger">{error}</span>
      ) : hint ? (
        <span className="text-[11.5px] text-subtle">{hint}</span>
      ) : null}
    </div>
  );
}

interface PasswordFieldProps extends InputHTMLAttributes<HTMLInputElement> {
  label: string;
  error?: string;
  hint?: string;
}

export function PasswordField({
  label,
  error,
  hint,
  id,
  required,
  className = "",
  ...props
}: PasswordFieldProps) {
  const inputId = id ?? useId();
  const [visible, setVisible] = useState(false);

  return (
    <div className="flex flex-col gap-1.5">
      <FieldLabel htmlFor={inputId} label={label} required={required} />
      <div className="relative">
        <input
          id={inputId}
          required={required}
          type={visible ? "text" : "password"}
          className={`h-10 w-full rounded-control border bg-surface-raised px-3 pr-10 text-[13px] text-foreground outline-none transition-colors duration-200 placeholder:text-subtle ${
            error
              ? "border-danger bg-danger/5"
              : "border-line focus:border-primary focus:shadow-ring"
          } ${className}`}
          {...props}
        />
        <button
          type="button"
          onClick={() => setVisible((v) => !v)}
          aria-label={visible ? "Hide password" : "Show password"}
          className="absolute right-0 top-0 flex h-10 w-10 items-center justify-center rounded-control text-subtle outline-none hover:text-muted focus-visible:shadow-[inset_0_0_0_1px_color-mix(in_oklab,var(--color-primary)_45%,transparent)] active:translate-y-px"
        >
          {visible ? (
            <EyeOff size={16} strokeWidth={1.6} />
          ) : (
            <Eye size={16} strokeWidth={1.6} />
          )}
        </button>
      </div>
      {error ? (
        <span className="text-[11.5px] text-danger">{error}</span>
      ) : hint ? (
        <span className="text-[11.5px] text-subtle">{hint}</span>
      ) : null}
    </div>
  );
}

const SEGMENTED_GLOW_TRANSITION = {
  type: "spring" as const,
  stiffness: 420,
  damping: 34,
  mass: 0.6,
};

/** Pill toggle with an animated glow that slides between options; used for unit/mode/protocol switches. */
export function SegmentedToggle<T extends string>({
  groupId,
  value,
  onChange,
  options,
  subdued = false,
}: {
  groupId: string;
  value: T;
  onChange: (v: T) => void;
  options: {
    value: T;
    label: string;
    disabled?: boolean;
    title?: string;
    suffix?: ReactNode;
  }[];
  subdued?: boolean;
}) {
  // No track fill: the page shows straight through, and only the active option is painted.
  return (
    <div className="inline-flex items-center gap-1 rounded-full p-1">
      {options.map((opt) => (
        <button
          key={opt.value}
          type="button"
          disabled={opt.disabled}
          onClick={() => onChange(opt.value)}
          title={opt.title}
          className={`relative flex min-h-[30px] items-center gap-1 whitespace-nowrap rounded-full px-3.5 text-[11.5px] font-medium outline-none transition-colors focus-visible:shadow-ring active:translate-y-px disabled:cursor-not-allowed disabled:opacity-40 ${
            value === opt.value
              ? "text-primary"
              : "text-muted hover:text-foreground"
          }`}
        >
          {value === opt.value && (
            <motion.span
              layoutId={`toggle-glow-${groupId}`}
              transition={SEGMENTED_GLOW_TRANSITION}
              className={`absolute inset-0 -z-10 rounded-full bg-primary/15 ${subdued ? "" : "shadow-glow"}`}
            />
          )}
          {opt.label}
          {opt.suffix}
        </button>
      ))}
    </div>
  );
}

/** Two-state-per-key sort control built on `SegmentedToggle`: clicking the active key flips its direction. */
export function SortToggle<T extends string>({
  groupId,
  sortKey,
  sortDir,
  onChange,
  options,
}: {
  groupId: string;
  sortKey: T;
  sortDir: Record<T, "asc" | "desc">;
  onChange: (key: T) => void;
  options: { key: T; label: string }[];
}) {
  return (
    <SegmentedToggle
      groupId={groupId}
      value={sortKey}
      onChange={onChange}
      options={options.map((opt) => ({
        value: opt.key,
        label: opt.label,
        suffix: <span>{sortDir[opt.key] === "desc" ? "↓" : "↑"}</span>,
      }))}
    />
  );
}
