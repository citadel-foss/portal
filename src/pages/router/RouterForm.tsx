import { AlertTriangle } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { checkRouterConfig, checkRouterPorts, checkTor, getRouterDefaults, getSuggestedRouterPorts } from "../../api/commands";
import type { RestoreSelection, RouterInitConfig, RouterPortCheck } from "../../api/types";
import { Disclosure } from "../../components/ui/display";
import { Button, FeeRateField, PasswordField, SummaryGroup, SummaryRow, TextField } from "../../components/ui/inputs";
import { validateNewPassword } from "../../lib/password-policy";
import { selectBackup } from "../../platform";
import { chosenFeeRate, type FeeChoice, useFeeEstimate } from "../../lib/fee-rate";
import { ROUTER_ID_PATTERN, ROUTER_NAME_MAX, routerNameError, timelockDays } from "./router-defaults";
import { formatNumber } from "../../lib/wallet-format";

// Long enough that editing a port digit-by-digit doesn't fire a check per keystroke.
const CHECK_DEBOUNCE_MS = 400;

/**
 * Every value the form edits. Strings, so an in-progress edit is representable — and all of
 * them start empty, including the economics: those come from the protocol crate over IPC, and
 * seeding them with a number here would be the second source of truth all over again. Rows
 * render "…" until the answer lands, which is the same thing the port fields already do.
 */
const INITIAL_VALUES = {
  socksPort: "",
  controlPort: "",
  networkPort: "",
  rpcPort: "",
  baseFee: "",
  amountRelativeFeePct: "",
  timeRelativeFeePct: "",
  requiredConfirms: "",
  fidelityAmount: "",
  fidelityTimelock: "",
  fidelityFeerate: "",
};

type Values = typeof INITIAL_VALUES;

export interface RouterForm {
  values: Values;
  set: (key: keyof Values) => (next: string) => void;
  walletName: string;
  setWalletName: (next: string) => void;
  /** Null until the user edits it; until then the name follows the wallet name. */
  publicName: string | null;
  setPublicName: (next: string) => void;
  /** The name published when the user hasn't set one: the wallet name, else the router ID. */
  defaultPublicName: (routerId: string) => string;
  publicNameError: string | null;
  dataDir: string;
  setDataDir: (next: string) => void;
  torError: string | null;
  portErrors: RouterPortCheck;
  /** The crate's own objection to the economics, such as a bond under its minimum. */
  configError: string | null;
  /** The crate's minimum bond amount, once its defaults have arrived. */
  minFidelityAmount: number | null;
  /** True while Tor, a port or the crate's config check refuses, whatever the rest says. */
  blocked: boolean;
  /** Builds the init config for a validated id and password, or null if the numbers don't hold. */
  config: (routerId: string, walletPassword: string) => RouterInitConfig | null;
}

/**
 * Ports, Tor and the router's economics — the half of new-router setup that is the same
 * whether it is the first router or the fifth, so both entry points drive one copy of it.
 */
export function useRouterForm(): RouterForm {
  const [values, setValues] = useState<Values>(INITIAL_VALUES);
  const [walletName, setWalletName] = useState("");
  const [publicName, setPublicName] = useState<string | null>(null);
  const [dataDir, setDataDir] = useState("");
  const [torError, setTorError] = useState<string | null>(null);
  const [portErrors, setPortErrors] = useState<RouterPortCheck>({});
  const [configError, setConfigError] = useState<string | null>(null);
  const [minFidelityAmount, setMinFidelityAmount] = useState<number | null>(null);
  const configRun = useRef(0);

  // Bumped on every edit so a slow in-flight check can't paint a verdict for a value the
  // user has already changed.
  const portRun = useRef(0);

  const numbers = useMemo(
    () =>
      Object.fromEntries(Object.entries(values).map(([k, v]) => [k, Number(v)])) as Record<
        keyof Values,
        number
      >,
    [values],
  );

  useEffect(() => {
    void getRouterDefaults()
      .then((d) => {
        setMinFidelityAmount(d.minFidelityAmount ?? null);
        setValues((v) => ({
          ...v,
          baseFee: String(d.baseFee),
          amountRelativeFeePct: String(d.amountRelativeFeePct),
          timeRelativeFeePct: String(d.timeRelativeFeePct),
          requiredConfirms: String(d.requiredConfirms),
          fidelityAmount: String(d.fidelityAmount),
          fidelityTimelock: String(d.fidelityTimelock),
        }));
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    void getSuggestedRouterPorts()
      .then((ports) =>
        setValues((v) => ({ ...v, networkPort: String(ports.networkPort), rpcPort: String(ports.rpcPort) })),
      )
      .catch((e) =>
        setPortErrors({ networkPort: (e as { message?: string })?.message ?? "Could not find free ports." }),
      );
  }, []);

  // Confirms Portal's own Tor is still up and picks up the ports it landed on, which the
  // cached defaults can only be stale about after a restart.
  useEffect(() => {
    void checkTor()
      .then((status) => {
        setTorError(status.reachable && status.authenticated ? null : (status.error ?? "Tor is unreachable."));
        if (status.socksPort === undefined || status.controlPort === undefined) return;
        setValues((v) => ({ ...v, socksPort: String(status.socksPort), controlPort: String(status.controlPort) }));
      })
      .catch((e) => setTorError((e as { message?: string })?.message ?? "Tor is unreachable."));
  }, []);

  useEffect(() => {
    const { networkPort, rpcPort } = numbers;
    if (![networkPort, rpcPort].every((p) => Number.isInteger(p) && p > 0)) return;
    const run = ++portRun.current;
    const timer = setTimeout(() => {
      void checkRouterPorts(networkPort, rpcPort)
        .then((result) => {
          if (run === portRun.current) setPortErrors(result);
        })
        .catch(() => {});
    }, CHECK_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [numbers.networkPort, numbers.rpcPort, numbers.socksPort, numbers.controlPort]);

  // The crate checks a router's economics only when it reads them back from config.toml, so
  // it is asked here, as they are edited, rather than after Create.
  useEffect(() => {
    if (Object.values(values).some((v) => v.trim() === "")) {
      setConfigError(null);
      return;
    }
    const run = ++configRun.current;
    const timer = setTimeout(() => {
      void checkRouterConfig({
        routerId: "check",
        walletName: "check",
        name: "check",
        networkPort: numbers.networkPort,
        rpcPort: numbers.rpcPort,
        socksPort: numbers.socksPort,
        controlPort: numbers.controlPort,
        baseFee: numbers.baseFee,
        amountRelativeFeePct: numbers.amountRelativeFeePct,
        timeRelativeFeePct: numbers.timeRelativeFeePct,
        requiredConfirms: numbers.requiredConfirms,
        fidelityAmount: numbers.fidelityAmount,
        fidelityTimelock: numbers.fidelityTimelock,
        fidelityFeerate: numbers.fidelityFeerate,
      })
        .then((verdict) => run === configRun.current && setConfigError(verdict))
        .catch(() => run === configRun.current && setConfigError(null));
    }, CHECK_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [values, numbers]);

  const publicNameError = publicName?.trim() ? routerNameError(publicName) : null;
  // Neither the wallet name nor the id has a length cap; the published name does.
  const defaultPublicName = (routerId: string) =>
    [...(walletName.trim() || routerId)].slice(0, ROUTER_NAME_MAX).join("");

  return {
    values,
    set: (key) => (next) => setValues((v) => ({ ...v, [key]: next })),
    walletName,
    setWalletName,
    publicName,
    setPublicName,
    defaultPublicName,
    publicNameError,
    dataDir,
    setDataDir,
    torError,
    portErrors,
    configError,
    minFidelityAmount,
    blocked:
      torError !== null ||
      portErrors.networkPort !== undefined ||
      portErrors.rpcPort !== undefined ||
      configError !== null,
    config: (routerId, walletPassword) => {
      // An empty string parses as 0, which is a legal-looking fee. Nothing may be submitted
      // before the crate's defaults have actually arrived.
      if (Object.values(values).some((v) => v.trim() === "")) return null;
      if (Object.values(numbers).some((n) => !Number.isFinite(n) || n < 0)) return null;
      if (numbers.requiredConfirms < 1) return null;
      if (numbers.fidelityFeerate < 1) return null;
      if (publicNameError) return null;
      return {
        routerId,
        // A router's wallet is its own, so the id doubles as the wallet name unless overridden.
        walletName: walletName.trim() || routerId,
        name: publicName?.trim() || defaultPublicName(routerId),
        dataDir: dataDir.trim() || undefined,
        walletPassword,
        networkPort: numbers.networkPort,
        rpcPort: numbers.rpcPort,
        socksPort: numbers.socksPort,
        controlPort: numbers.controlPort,
        baseFee: numbers.baseFee,
        amountRelativeFeePct: numbers.amountRelativeFeePct,
        timeRelativeFeePct: numbers.timeRelativeFeePct,
        requiredConfirms: numbers.requiredConfirms,
        fidelityAmount: numbers.fidelityAmount,
        fidelityTimelock: numbers.fidelityTimelock,
        fidelityFeerate: numbers.fidelityFeerate,
      };
    },
  };
}

export { ROUTER_ID_PATTERN };

function warningLine(text: string) {
  return (
    <p className="flex items-start gap-1.5 text-[10.5px] text-danger">
      <AlertTriangle size={13} strokeWidth={2} className="mt-0.5 shrink-0" />
      {text}
    </p>
  );
}

function sats(value: string) {
  // An unloaded field is blank, and `Number("")` is 0 — which would render a real-looking
  // "0 sats" fee for the moment before the crate's defaults arrive.
  if (value.trim() === "") return "…";
  const n = Number(value);
  return Number.isFinite(n) ? formatNumber(n) : value;
}

/** Sits under the router ID rather than in Advanced: it is the one value strangers see. */
export function PublicNameField({ form, routerId }: { form: RouterForm; routerId: string }) {
  return (
    <TextField
      label="Public name"
      // Sits right above the wallet password, where a password manager otherwise guesses the
      // username goes, and fills in whatever it saved for the previous router.
      autoComplete="off"
      placeholder={form.defaultPublicName(routerId) || "Same as wallet name"}
      // Filled in rather than left as a placeholder, so a value nobody typed is visible.
      value={form.publicName ?? form.defaultPublicName(routerId)}
      onChange={(e) => form.setPublicName(e.target.value)}
      error={form.publicNameError ?? undefined}
      hint={
        form.publicNameError
          ? undefined
          : "Shown to every wallet on the market. Anyone can claim any name. Rename it any time in Settings."
      }
    />
  );
}

/**
 * The bond is the only thing here the operator cannot change afterwards, so it is the only
 * thing shown without being asked for. Everything else is a working default.
 */
export function FidelityFields({ form }: { form: RouterForm }) {
  const timelock = Number(form.values.fidelityTimelock);
  return (
    <SummaryGroup title="Fidelity bond" warning={form.configError ? warningLine(form.configError) : undefined}>
      <SummaryRow
        label="Target amount"
        value={form.values.fidelityAmount}
        display={sats(form.values.fidelityAmount)}
        suffix="sats"
        hint={form.minFidelityAmount !== null ? `Minimum ${formatNumber(form.minFidelityAmount)} sats` : undefined}
        onCommit={form.set("fidelityAmount")}
      />
      <SummaryRow
        label="Timelock"
        value={form.values.fidelityTimelock}
        display={sats(form.values.fidelityTimelock)}
        suffix="blocks"
        hint={timelock > 0 ? `≈ ${timelockDays(timelock)} days locked` : undefined}
        onCommit={form.set("fidelityTimelock")}
      />
    </SummaryGroup>
  );
}

/** The bond transaction's fee rate: a stuck bond cannot be bumped, the crate has no fee-bump
 *  for bonds. Owns `fidelityFeerate`, so the form holds whichever rate is picked here. */
export function BondFeeRateField({ form }: { form: RouterForm }) {
  const { fees, failed, retry } = useFeeEstimate();
  const [choice, setChoice] = useState<FeeChoice>("fast");
  const [custom, setCustom] = useState("");
  const rate = chosenFeeRate(fees, choice, custom);
  const setFeerate = form.set("fidelityFeerate");
  useEffect(() => {
    setFeerate(rate > 0 ? String(rate) : "");
    // `form.set` is rebuilt every render; only the rate decides the value.
  }, [rate]);
  return (
    <div className="flex flex-col gap-2">
      <span className="font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">Bond fee rate</span>
      <FeeRateField
        fees={fees}
        failed={failed}
        onRetry={retry}
        choice={choice}
        onChoice={setChoice}
        custom={custom}
        onCustom={setCustom}
      />
    </div>
  );
}

/** `null` while a new router gets a fresh wallet; otherwise its wallet comes from a backup file,
 *  with `selection` null until one is chosen. */
export type RouterRestore = { selection: RestoreSelection | null } | null;

/**
 * The router's wallet password. A new wallet takes a new password, entered twice; a restored one
 * keeps the backup's, so restoring asks for that instead. `error` is null once the router can be
 * created, and otherwise says what is missing.
 */
export function useRouterWallet() {
  const [restore, setRestore] = useState<RouterRestore>(null);
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const error =
    restore === null
      ? validateNewPassword(password, confirm)
      : !restore.selection
        ? "Choose the backup file to continue."
        : !password
          ? "Enter the backup password to continue."
          : null;
  return {
    restore,
    setRestore,
    password,
    setPassword,
    confirm,
    setConfirm,
    error,
    /** The init config's wallet fields. */
    fields: (config: RouterInitConfig | null): RouterInitConfig | null =>
      config && restore?.selection
        ? { ...config, restoreSelection: restore.selection.selectionId }
        : config,
    clear: () => {
      setPassword("");
      setConfirm("");
      // A selection is single-use; one a finished restore consumed cannot be submitted again.
      setRestore(null);
    },
  };
}

export type RouterWallet = ReturnType<typeof useRouterWallet>;

/** Above everything else when restoring: the file decides what is being added, and the name
 *  and password follow from it. */
export function RouterRestoreChoice({ wallet }: { wallet: RouterWallet }) {
  const [choosing, setChoosing] = useState(false);
  const [chooseError, setChooseError] = useState<string | null>(null);
  const { restore, setRestore, setPassword, setConfirm } = wallet;

  async function choose() {
    setChoosing(true);
    setChooseError(null);
    try {
      setRestore({ selection: await selectBackup() });
    } catch (e) {
      if ((e as { code?: string })?.code !== "USER_CANCELLED")
        setChooseError((e as { message?: string })?.message ?? "Could not open the backup file.");
    } finally {
      setChoosing(false);
    }
  }

  return (
    <div className="flex flex-col gap-3">
      <label className="flex cursor-pointer items-center gap-2.5 text-[12.5px] text-foreground">
        <input
          type="checkbox"
          className="accent-primary"
          checked={restore !== null}
          onChange={(e) => {
            setRestore(e.target.checked ? { selection: null } : null);
            setPassword("");
            setConfirm("");
          }}
        />
        Restore from a backup file
      </label>
      {restore !== null && (
        <>
          <div className="flex items-center gap-3">
            <Button size="sm" variant="secondary" loading={choosing} onClick={() => void choose()}>
              {restore.selection ? "Choose another file" : "Choose backup file"}
            </Button>
            {restore.selection && (
              <span className="min-w-0 truncate font-mono text-[11.5px] text-muted">
                {restore.selection.displayName}
              </span>
            )}
          </div>
          {chooseError && <p className="text-[11.5px] text-danger">{chooseError}</p>}
          <p className="text-[11.5px] leading-5 text-subtle">
            Then name the router and enter the backup&apos;s password below.
          </p>
        </>
      )}
    </div>
  );
}

export function RouterPasswordFields({ wallet }: { wallet: RouterWallet }) {
  const { restore, password, setPassword, confirm, setConfirm, error } = wallet;
  if (restore !== null)
    return (
      <PasswordField
        label="Backup password"
        autoComplete="current-password"
        required
        value={password}
        onChange={(e) => setPassword(e.target.value)}
        hint="The restored router wallet keeps this password."
      />
    );
  return (
    <div className="flex flex-col gap-3">
      <PasswordField label="Wallet password" autoComplete="new-password" required value={password} onChange={(e) => setPassword(e.target.value)} />
      <PasswordField
        label="Confirm wallet password"
        autoComplete="new-password"
        required
        value={confirm}
        onChange={(e) => setConfirm(e.target.value)}
        error={password || confirm ? (error ?? undefined) : undefined}
      />
      <p className="text-[11.5px] leading-5 text-subtle">
        Portal encrypts every router wallet it creates. This password cannot be recovered if it is
        lost.
      </p>
    </div>
  );
}

/** Ports, fees and storage: all changeable from the router's Settings tab afterwards. */
export function AdvancedFields({ form, routerId }: { form: RouterForm; routerId: string }) {
  return (
    <Disclosure label="Advanced settings">
      <div className="flex flex-col gap-4 pt-2">
        <SummaryGroup title="Swap policy">
          <SummaryRow label="Base fee" value={form.values.baseFee} display={sats(form.values.baseFee)} suffix="sats" onCommit={form.set("baseFee")} />
          <SummaryRow label="Amount-relative fee" value={form.values.amountRelativeFeePct} display={form.values.amountRelativeFeePct || "…"} suffix="%" inputMode="decimal" onCommit={form.set("amountRelativeFeePct")} />
          <SummaryRow label="Time-relative fee" value={form.values.timeRelativeFeePct} display={form.values.timeRelativeFeePct || "…"} suffix="%" inputMode="decimal" onCommit={form.set("timeRelativeFeePct")} />
          <SummaryRow label="Required confirmations" value={form.values.requiredConfirms} display={form.values.requiredConfirms || "…"} onCommit={form.set("requiredConfirms")} />
        </SummaryGroup>

        <SummaryGroup title="Router ports" warning={form.portErrors.networkPort ?? form.portErrors.rpcPort ? warningLine((form.portErrors.networkPort ?? form.portErrors.rpcPort)!) : undefined}>
          <SummaryRow label="Network port" value={form.values.networkPort || "…"} onCommit={form.set("networkPort")} />
          <SummaryRow label="RPC port" value={form.values.rpcPort || "…"} onCommit={form.set("rpcPort")} />
        </SummaryGroup>

        <SummaryGroup title="Tor" warning={form.torError ? warningLine(form.torError) : undefined}>
          <SummaryRow label="SOCKS port" value={form.values.socksPort} readOnly hint="Portal runs its own Tor" />
          <SummaryRow label="Control port" value={form.values.controlPort} readOnly />
        </SummaryGroup>

        <div className="flex flex-col gap-3">
          <TextField label="Wallet name" placeholder={routerId || "Same as router ID"} value={form.walletName} onChange={(e) => form.setWalletName(e.target.value)} />
          <TextField label="Data directory" placeholder="Default router directory" value={form.dataDir} onChange={(e) => form.setDataDir(e.target.value)} />
          <p className="text-[11.5px] leading-5 text-subtle">
            Wallet name and data directory are permanent and cannot be changed later.
          </p>
        </div>
      </div>
    </Disclosure>
  );
}
