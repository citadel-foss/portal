import { Check, Save } from "lucide-react";
import { useEffect, useState } from "react";
import { checkBackend, revealRecoveryPhrase } from "../../api/commands";
import { ShowRecoveryPhrase } from "../../components/app/RecoveryPhrase";
import { createBackup } from "../../platform";
import type { BackendStatus } from "../../api/types";
import { Card } from "../../components/ui/display";
import { Button, PasswordField } from "../../components/ui/inputs";
import { useToastStore } from "../../store/toast";
import { formatNumber } from "../../lib/wallet-format";

const MIN_BACKUP_PASSWORD = 8;

/**
 * Chain setup lives on the connection gate, so what a running wallet still needs is here:
 * an encrypted backup, and where it is connected.
 */
export function WalletFooterCard() {
  const [status, setStatus] = useState<BackendStatus | null>(null);

  useEffect(() => {
    void checkBackend()
      .then(setStatus)
      .catch(() => setStatus(null));
  }, []);

  return (
    <Card className="border-line-strong">
      <div className="flex flex-wrap items-start justify-between gap-4 border-b border-line px-5 py-4">
        <div>
          <h2 className="font-header text-[14px] font-bold text-foreground">
            Wallet backup
          </h2>
          <p className="mt-1 text-[11px] text-muted">
            The recovery phrase, and an encrypted export for recovery or migration
          </p>
        </div>
        <span
          className={`inline-flex items-center gap-2 rounded-full border px-2.5 py-1 font-mono text-[9.5px] font-semibold uppercase tracking-[0.12em] ${
            status === null
              ? "border-line bg-surface-raised text-subtle"
              : status.reachable
                ? "border-success/35 bg-success/[0.08] text-success"
                : "border-danger/35 bg-danger/[0.08] text-danger"
          }`}
          title={status?.error ?? undefined}
        >
          <span
            className={`h-1.5 w-1.5 rounded-full ${
              status === null
                ? "bg-subtle"
                : status.reachable
                  ? "bg-success"
                  : "bg-danger"
            }`}
          />
          {status === null
            ? "Checking"
            : status.reachable
              ? `${status.chain ?? "connected"}${status.blocks !== undefined ? ` · ${formatNumber(status.blocks)}` : ""}`
              : "Not connected"}
        </span>
      </div>

      <div className="border-b border-line p-5">
        <ShowRecoveryPhrase subject="wallet" reveal={revealRecoveryPhrase} />
      </div>
      <div className="p-5">
        <BackupForm />
      </div>
    </Card>
  );
}

/** Wallet password, backup password, confirm, then the host saves the encrypted file. Without
 *  `routerId` it backs up the open wallet. */
export function BackupForm({ routerId }: { routerId?: string }) {
  const pushToast = useToastStore((s) => s.push);

  const [open, setOpen] = useState(false);
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [walletPassword, setWalletPassword] = useState("");
  const [error, setError] = useState<string | undefined>();
  const [busy, setBusy] = useState(false);

  function submit() {
    setError(undefined);
    if (!walletPassword) {
      return setError(`Enter the ${routerId === undefined ? "wallet" : "router wallet"} password.`);
    }
    if (!password) return setError("Please enter a backup password.");
    if (password.length < MIN_BACKUP_PASSWORD) {
      return setError(
        `Password must be at least ${MIN_BACKUP_PASSWORD} characters.`,
      );
    }
    if (password !== confirm) return setError("Passwords do not match.");
    void run();
  }

  async function run() {
    setBusy(true);
    try {
      const displayName = await createBackup(walletPassword, password, routerId);
      pushToast("success", `Encrypted backup created: ${displayName}`);
      setOpen(false);
    } catch (e) {
      // The native save dialog was dismissed; not a failure worth reporting.
      if ((e as { code?: string })?.code === "USER_CANCELLED") return;
      pushToast(
        "error",
        `Backup failed: ${(e as { message?: string })?.message ?? "unknown error"}`,
      );
    } finally {
      setPassword("");
      setConfirm("");
      setWalletPassword("");
      setBusy(false);
    }
  }

  return (
    <>
      <p className="text-[12px] leading-5 text-muted">
        The backup contains {routerId === undefined ? "your wallet's" : "this router wallet's"} keys.
        Protect it with a strong password and keep that password somewhere safe — the same one
        is required to restore it.
      </p>
      {!open ? (
        <Button size="sm" className="mt-4" onClick={() => setOpen(true)}>
          <Save size={14} strokeWidth={2} /> Create backup
        </Button>
      ) : (
        <div className="mt-4">
          <div className="mb-3">
            <PasswordField
              label={routerId === undefined ? "Wallet password" : "Router wallet password"}
              autoComplete="current-password"
              value={walletPassword}
              onChange={(e) => setWalletPassword(e.target.value)}
              hint="Unlocks the wallet's seed for the backup."
            />
          </div>
          <div className="grid grid-cols-2 gap-3 max-[620px]:grid-cols-1">
            <PasswordField
              label="Backup Password"
              placeholder="Enter password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
            <PasswordField
              label="Confirm Password"
              placeholder="Re-enter password"
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && submit()}
            />
          </div>
          {error && <p className="mt-2 text-[12px] text-danger">{error}</p>}
          <Button size="sm" className="mt-3" onClick={submit} loading={busy}>
            <Check size={14} strokeWidth={2} /> Confirm &amp; create backup
          </Button>
        </div>
      )}
    </>
  );
}
