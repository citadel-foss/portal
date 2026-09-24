import { AlertTriangle } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { checkRouterPorts, checkTor, getRouterDefaults, getSuggestedRouterPorts } from "../../api/commands";
import type { RouterInitConfig, RouterPortCheck } from "../../api/types";
import { Disclosure } from "../../components/ui/display";
import { SummaryGroup, SummaryRow, TextField } from "../../components/ui/inputs";
import { ROUTER_ID_PATTERN, timelockDays } from "./router-defaults";

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
  minSwapAmount: "",
  baseFee: "",
  amountRelativeFeePct: "",
  timeRelativeFeePct: "",
  requiredConfirms: "",
  fidelityAmount: "",
  fidelityTimelock: "",
};

type Values = typeof INITIAL_VALUES;

export interface RouterForm {
  values: Values;
  set: (key: keyof Values) => (next: string) => void;
  walletName: string;
  setWalletName: (next: string) => void;
  dataDir: string;
  setDataDir: (next: string) => void;
  torError: string | null;
  portErrors: RouterPortCheck;
  /** True while Tor or a port is unusable, whatever the rest of the form says. */
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
  const [dataDir, setDataDir] = useState("");
  const [torError, setTorError] = useState<string | null>(null);
  const [portErrors, setPortErrors] = useState<RouterPortCheck>({});

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
      .then((d) =>
        setValues((v) => ({
          ...v,
          minSwapAmount: String(d.minSwapAmount),
          baseFee: String(d.baseFee),
          amountRelativeFeePct: String(d.amountRelativeFeePct),
          timeRelativeFeePct: String(d.timeRelativeFeePct),
          requiredConfirms: String(d.requiredConfirms),
          fidelityAmount: String(d.fidelityAmount),
          fidelityTimelock: String(d.fidelityTimelock),
        })),
      )
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

  return {
    values,
    set: (key) => (next) => setValues((v) => ({ ...v, [key]: next })),
    walletName,
    setWalletName,
    dataDir,
    setDataDir,
    torError,
    portErrors,
    blocked:
      torError !== null || portErrors.networkPort !== undefined || portErrors.rpcPort !== undefined,
    config: (routerId, walletPassword) => {
      // An empty string parses as 0, which is a legal-looking fee. Nothing may be submitted
      // before the crate's defaults have actually arrived.
      if (Object.values(values).some((v) => v.trim() === "")) return null;
      if (Object.values(numbers).some((n) => !Number.isFinite(n) || n < 0)) return null;
      if (numbers.requiredConfirms < 1) return null;
      return {
        routerId,
        // A router's wallet is its own, so the id doubles as the wallet name unless overridden.
        walletName: walletName.trim() || routerId,
        dataDir: dataDir.trim() || undefined,
        walletPassword,
        networkPort: numbers.networkPort,
        rpcPort: numbers.rpcPort,
        socksPort: numbers.socksPort,
        controlPort: numbers.controlPort,
        minSwapAmount: numbers.minSwapAmount,
        baseFee: numbers.baseFee,
        amountRelativeFeePct: numbers.amountRelativeFeePct,
        timeRelativeFeePct: numbers.timeRelativeFeePct,
        requiredConfirms: numbers.requiredConfirms,
        fidelityAmount: numbers.fidelityAmount,
        fidelityTimelock: numbers.fidelityTimelock,
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
  return Number.isFinite(n) ? n.toLocaleString() : value;
}

/**
 * The bond is the only thing here the operator cannot change afterwards, so it is the only
 * thing shown without being asked for. Everything else is a working default.
 */
export function FidelityFields({ form }: { form: RouterForm }) {
  const timelock = Number(form.values.fidelityTimelock);
  return (
    <SummaryGroup title="Fidelity bond">
      <SummaryRow
        label="Target amount"
        value={form.values.fidelityAmount}
        display={sats(form.values.fidelityAmount)}
        suffix="sats"
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

/** Ports, fees and storage: all changeable from the router's Settings tab afterwards. */
export function AdvancedFields({ form, routerId }: { form: RouterForm; routerId: string }) {
  return (
    <Disclosure label="Advanced settings">
      <div className="flex flex-col gap-4 pt-2">
        <SummaryGroup title="Swap policy">
          <SummaryRow label="Minimum swap amount" value={form.values.minSwapAmount} display={sats(form.values.minSwapAmount)} suffix="sats" onCommit={form.set("minSwapAmount")} />
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
          <TextField label="Wallet name" placeholder={routerId || "Same as router name"} value={form.walletName} onChange={(e) => form.setWalletName(e.target.value)} />
          <TextField label="Data directory" placeholder="Default router directory" value={form.dataDir} onChange={(e) => form.setDataDir(e.target.value)} />
          <p className="text-[11.5px] leading-5 text-subtle">
            Wallet name and data directory are permanent and cannot be changed later.
          </p>
        </div>
      </div>
    </Disclosure>
  );
}
