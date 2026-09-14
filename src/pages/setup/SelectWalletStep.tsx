import { subscribe } from "../../api/transport";
import { capabilities, pickDirectory, pickFile, selectBackup } from "../../platform";
import { FolderOpen, FolderPlus, Plus, RotateCcw } from "lucide-react";
import { motion } from "framer-motion";
import { useEffect, useMemo, useState, type KeyboardEvent } from "react";
import { initWallet, listWallets, restoreWallet, syncOfferbook } from "../../api/commands";
import { isAppError } from "../../api/types";
import type { InitResult, RestoreSelection } from "../../api/types";
import { Card, Modal, WalletCard } from "../../components/ui/display";
import { Button, PasswordField, TextField } from "../../components/ui/inputs";
import { Checklist } from "../../components/ui/Checklist";
import { IntroStage } from "../../components/ui/IntroStage";
import { MIN_WALLET_PASSWORD_LENGTH } from "../../lib/password-policy";
import { withMinDelay } from "../../lib/timing";
import { walletIdentity } from "../../lib/wallet-identity";
import {
  getDefaultDataDir,
  getDefaultWalletsDir,
  loadDataDir,
  saveDataDir,
  type WalletChoice,
} from "./types";

interface SelectWalletStepProps {
  onSuccess: (result: InitResult, restored: boolean) => void;
}

type ViewMode = "grid" | "unlock" | "create" | "restore" | "checking";
interface CheckFailure {
  message: string;
}

interface InitProgress {
  /** Index into `INIT_STEPS`; -1 until `init_taker` reports its first phase. */
  phase: number;
  note: string | null;
  failed: boolean;
}

/** Mirrors the phase indices `logging.rs` emits on `wallet://init-phase`. */
const INIT_STEPS = [
  "Connecting to the chain backend",
  "Unlocking wallet",
  "Starting the contract watcher",
  "Loading the offerbook",
];

/** Phase `Taker::init` is inside when it rejects a password, so a wrong one fails that row. */
const UNLOCK_STEP = 1;

// The same curve IntroStage enters on, so the grid inherits the stage's motion signature.
const RISE = [0.16, 1, 0.3, 1] as const;

const IDLE_PROGRESS: InitProgress = { phase: -1, note: null, failed: false };

function randomWalletName() {
  return `taker-wallet-${Math.floor(100000 + Math.random() * 900000)}`;
}

function parentDir(path: string) {
  return path.replace(/[/\\][^/\\]*$/, "");
}

function basename(path: string) {
  return path.split(/[/\\]/).pop() ?? path;
}

// Local checks (RPC/Tor/wallet unlock) often resolve in well under 100ms,
// which makes the sequential checklist flash by unreadably.
const MIN_STEP_MS = 900;

// The crate refuses to create an unencrypted wallet, so this is a floor, not a style rule.

const CAPTIONS: Record<ViewMode, string> = {
  grid: "Select your wallet",
  unlock: "Unlock your wallet",
  create: "Create a new wallet",
  restore: "Restore an encrypted backup",
  checking: "Setting things up",
};

function onEnter(fn: () => void) {
  return (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") fn();
  };
}

export function SelectWalletStep({ onSuccess }: SelectWalletStepProps) {
  const [dataDir, setDataDir] = useState<string | undefined>(loadDataDir);
  // null while a folder scan is in flight. The intro plays over it rather than waiting.
  const [wallets, setWallets] = useState<string[] | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>("grid");

  const [selectedWallet, setSelectedWallet] = useState<string | null>(null);
  const [unlockPassword, setUnlockPassword] = useState("");

  const [createName, setCreateName] = useState(randomWalletName());
  const [createPassword, setCreatePassword] = useState("");
  const [createConfirm, setCreateConfirm] = useState("");
  const [restoreSelection, setRestoreSelection] = useState<RestoreSelection | null>(null);
  const [restoreName, setRestoreName] = useState(randomWalletName());
  const [restorePassword, setRestorePassword] = useState("");

  const [progress, setProgress] = useState<InitProgress>(IDLE_PROGRESS);
  const [failure, setFailure] = useState<CheckFailure | null>(null);
  const [pendingWallet, setPendingWallet] = useState<WalletChoice | null>(null);
  const [retryPassword, setRetryPassword] = useState("");

  useEffect(() => {
    refreshWallets(dataDir);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    // Phases only advance: `Taker::init`'s recovery pass re-logs lines the earlier phases also
    // emit, and a late one must not walk the checklist backwards.
    const unlisten = subscribe<{ phase: number; note: string | null }>("wallet://init-phase", (event) =>
      setProgress((current) => ({
        phase: Math.max(current.phase, event.phase),
        note: event.note,
        failed: false,
      })),
    );
    return () => void unlisten.then((stop) => stop());
  }, []);

  async function refreshWallets(dir?: string) {
    setWallets(null);
    // An unreadable folder is indistinguishable from an empty one here, and both lead to the
    // same place: the create form.
    const found = await listWallets(dir).catch(() => []);
    setWallets(found);
    // With nothing to unlock, creating is the only way forward — open the form directly
    // rather than making the user dismiss an empty-state panel first. Back still reaches
    // the grid, so Change location / Load wallet stay available.
    if (found.length === 0) return setViewMode("create");
    // A one-wallet grid is a single card whose only purpose is to be clicked, so skip it and
    // ask for the password directly. The unlock view carries the same location/create/load
    // actions, so nothing becomes unreachable.
    if (found.length === 1) return selectWallet(found[0]);
    setViewMode("grid");
  }

  function selectWallet(name: string) {
    setSelectedWallet(name);
    setUnlockPassword("");
    setViewMode("unlock");
  }

  async function changeLocation() {
    const path = await pickDirectory(dataDir ?? (await getDefaultDataDir()));
    if (path === null) return;
    setDataDir(path);
    saveDataDir(path);
    refreshWallets(path);
  }

  async function loadWalletFile() {
    const path = await pickFile(dataDir ?? (await getDefaultWalletsDir()));
    if (path === null) return;
    // Wallet files live at <data_dir>/wallets/<name> — if this file is
    // outside the current data dir, adopt its parent as the new data dir.
    // Wallet files live at <data_dir>/wallets/<name>; both hosts hand back a POSIX-or-Windows
    // path, so the split is done here rather than through a native path API.
    const walletsDir = parentDir(path);
    const newDataDir = parentDir(walletsDir);
    setDataDir(newDataDir);
    saveDataDir(newDataDir);
    selectWallet(basename(path));
  }

  async function beginRestore() {
    try {
      const selection = await selectBackup();
      setRestoreSelection(selection);
      setRestoreName(randomWalletName());
      setRestorePassword("");
      setViewMode("restore");
    } catch (e) {
      if ((e as { code?: string })?.code !== "USER_CANCELLED") {
        setFailure({ message: (e as { message?: string })?.message ?? "Could not select the backup." });
      }
    }
  }

  // Validated as the user types so the submit stays disabled, rather than accepting the click
  // and reporting what's wrong afterwards.
  const canCreate =
    createName.trim().length > 0 && createPassword.length >= MIN_WALLET_PASSWORD_LENGTH && createPassword === createConfirm;

  function submitCreate() {
    if (!canCreate) return;
    runChecks({ mode: "create", walletName: createName.trim(), password: createPassword });
  }

  function submitUnlock() {
    if (!selectedWallet) return;
    runChecks({ mode: "load", walletName: selectedWallet, password: unlockPassword });
  }

  function submitRestore() {
    if (!restoreSelection || !restoreName.trim()) return;
    runChecks({
      mode: "restore",
      walletName: restoreName.trim(),
      selectionId: restoreSelection.selectionId,
      displayName: restoreSelection.displayName,
      password: restorePassword || undefined,
    });
  }

  async function runChecks(wallet: WalletChoice) {
    setPendingWallet(wallet);
    setFailure(null);
    setViewMode("checking");
    // A restore runs its own sync before `init_taker` arms the phase watcher, so it starts
    // behind the first reported phase rather than on it.
    setProgress({ phase: wallet.mode === "restore" ? -1 : 0, note: null, failed: false });

    try {
      const result = await withMinDelay(
        (async () => {
          if (wallet.mode === "restore") {
            await restoreWallet(wallet.walletName, undefined, wallet.selectionId, wallet.password, dataDir);
          }
          return initWallet({
            walletName: wallet.walletName,
            walletPassword: wallet.password,
            connectionType: "tor",
            dataDir,
          });
        })(),
        MIN_STEP_MS,
      );
      setProgress({ phase: INIT_STEPS.length, note: null, failed: false });
      // Kick off a real offerbook sync now, in the background, so the Market page has fresh
      // router data by the time the user looks at it — not just whatever offerbook.json had from
      // the last session. Not awaited: this can take 30-60s+ and shouldn't block navigation.
      void syncOfferbook().catch(() => {});
      // restore_wallet completes its own sync_and_save before init_taker, so a
      // successful restore already satisfies the mandatory first scan.
      onSuccess(result, wallet.mode === "restore");
    } catch (e) {
      const err = isAppError(e) ? e : null;
      // `init_taker` is what checks the password, so a wrong one failed at the unlock step
      // however far the phase watcher had got; anything else failed where it stopped.
      const wrongPassword = err?.code === "WALLET_WRONG_PASSWORD";
      setProgress((current) => ({
        phase: wrongPassword ? UNLOCK_STEP : Math.max(current.phase, 0),
        note: null,
        failed: true,
      }));
      setFailure({
        message:
          wrongPassword ? "Incorrect password. Try again." : (err?.message ?? "Something went wrong."),
      });
    }
  }

  function retry() {
    if (!pendingWallet) return;
    // Only an actual entry replaces the password: a create or restore that failed for some
    // other reason must keep the one it was given rather than retry with an empty field.
    runChecks(retryPassword ? { ...pendingWallet, password: retryPassword } : pendingWallet);
  }

  function cancelFailure() {
    setFailure(null);
    // A failure raised before any attempt (backup picker) has no pending wallet, and the
    // current view is already the one that can correct it.
    if (!pendingWallet) return;
    setViewMode(
      pendingWallet.mode === "create" ? "create" : pendingWallet.mode === "restore" ? "restore" : "unlock",
    );
  }

  const firstRun = wallets?.length === 0;
  const unlockIdentity = useMemo(() => walletIdentity(selectedWallet ?? ""), [selectedWallet]);

  const createFields = (
    <div className="flex flex-col gap-5 text-left">
      <TextField
        label="Wallet name"
        required
        value={createName}
        onChange={(e) => setCreateName(e.target.value)}
      />
      <PasswordField
        label="Password"
        required
        value={createPassword}
        onChange={(e) => setCreatePassword(e.target.value)}
        hint={
          createPassword.length > 0 && createPassword.length < MIN_WALLET_PASSWORD_LENGTH
            ? `At least ${MIN_WALLET_PASSWORD_LENGTH} characters. An unencrypted wallet is not permitted — losing this password means losing access to funds.`
            : undefined
        }
      />
      <PasswordField
        label="Confirm password"
        required
        value={createConfirm}
        onChange={(e) => setCreateConfirm(e.target.value)}
        onKeyDown={onEnter(submitCreate)}
        error={createConfirm.length > 0 && createConfirm !== createPassword ? "Passwords don't match." : undefined}
      />
    </div>
  );

  // Shared by the grid footer and the unlock footer: skipping the grid for a lone wallet must
  // not also skip the only route to a different folder, file, or new wallet.
  const walletActions = (
    <div className="mt-3 flex flex-wrap items-center justify-center gap-x-1 gap-y-0.5">
      {/* Both browse the host's filesystem, which a browser has no access to and no
          business seeing: the web host manages wallet locations itself. */}
      {capabilities.nativeFilePicker && (
        <>
          <Button variant="ghost" size="sm" className="px-2.5 text-[11.5px]" onClick={changeLocation}>
            <FolderOpen size={13} strokeWidth={1.8} />
            Change location
          </Button>
          <Button variant="ghost" size="sm" className="px-2.5 text-[11.5px]" onClick={loadWalletFile}>
            <FolderPlus size={13} strokeWidth={1.8} />
            Load wallet
          </Button>
        </>
      )}
      <Button variant="ghost" size="sm" className="px-2.5 text-[11.5px]" onClick={() => setViewMode("create")}>
        <Plus size={13} strokeWidth={1.8} />
        Create new wallet
      </Button>
      <Button variant="ghost" size="sm" className="px-2.5 text-[11.5px]" onClick={() => void beginRestore()}>
        <RotateCcw size={13} strokeWidth={1.8} />
        Restore backup
      </Button>
    </div>
  );

  const createActions = (
    <div className="flex gap-3">
      {/* Create is the default view with no wallets on disk, so this is the only route to
          Change location / Load wallet — label it for what it reaches, not as "Back". */}
      <Button variant="secondary" onClick={() => setViewMode("grid")}>
        {firstRun ? "Use existing" : "Back"}
      </Button>
      <Button className="flex-1" disabled={!canCreate} onClick={submitCreate}>
        Create &amp; continue
      </Button>
    </div>
  );

  // A restore syncs the wallet before `init_taker` starts, so it gets a row of its own ahead
  // of the phases the backend reports.
  const restoring = pendingWallet?.mode === "restore";
  const checklistSteps = restoring ? ["Restoring from backup", ...INIT_STEPS] : INIT_STEPS;
  const active = restoring ? progress.phase + 1 : progress.phase;
  const checklist = (
    <>
      <Checklist
        steps={checklistSteps.map((label, i) => ({
          label,
          state:
            i < active
              ? "passed"
              : i > active
                ? "idle"
                : progress.failed
                  ? "failed"
                  : "running",
        }))}
      />
      {/* Recovery has no step of its own — it does nothing on most launches — but it blocks on
          a block being mined, so once the steps have all ticked it is the only thing that can
          explain why the app has not opened yet. */}
      {progress.note && <p className="mt-6 text-center text-[12.5px] text-muted">{progress.note}</p>}
    </>
  );

  const failureModal = failure && (
    <Modal
      title="Couldn't unlock wallet"
      onClose={cancelFailure}
      footer={
        <>
          <Button variant="secondary" onClick={cancelFailure}>
            Cancel
          </Button>
          <Button onClick={retry}>Retry</Button>
        </>
      }
    >
      <p className="text-[12.5px] text-danger">{failure.message}</p>

      <PasswordField
        label="Password"
        value={retryPassword}
        onChange={(e) => setRetryPassword(e.target.value)}
        onKeyDown={onEnter(retry)}
        placeholder="Enter wallet password"
        autoFocus
      />
    </Modal>
  );

  return (
    <>
      <IntroStage
        lead="Welcome to"
        accent="Portal"
        caption={CAPTIONS[viewMode]}
        // Withheld mid-checklist: leaving then would abandon an init that is already running.
        back={viewMode === "checking" ? undefined : { to: "/launch", label: "Portal", title: "Back to start" }}
        className="min-h-screen"
      >
        {/* The grid needs room for two wallet cards abreast; every other view is a single
            column and looks stranded at that width. */}
        <div className={`mx-auto w-full ${viewMode === "grid" ? "max-w-2xl" : viewMode === "unlock" ? "max-w-xl" : "max-w-lg"}`}>
          {/* Panelled so the contents read as one object against the empty screen. Actions sit
              in a footer inside it rather than loose underneath, which would leave the box
              looking unfinished at the bottom. */}
          {/* Hairlined on the two views that are a single focal object: the unlock prompt, and
              the checklist the user sits and watches. The grid and create form are lists of
              choices, where a lit panel edge competes with the choices themselves. */}
          <Card className={`border-line-strong ${viewMode === "unlock" || viewMode === "checking" ? "hairline" : ""}`}>
            {viewMode === "grid" && (
              <>
                <div className="p-8">
                  {wallets === null ? (
                    <p className="text-center text-[13px] text-muted">Looking for wallets…</p>
                  ) : wallets.length === 0 ? (
                    <div className="rounded-card border border-dashed border-line-strong px-8 py-10 text-center">
                      <p className="text-[14px] font-medium text-foreground">No wallets found</p>
                      <p className="mt-1 text-[12.5px] text-muted">
                        Create a new wallet to get started, or point the app at a different folder.
                      </p>
                      <Button className="mt-5" onClick={() => setViewMode("create")}>
                        Create new wallet
                      </Button>
                    </div>
                  ) : (
                    <div className="flex flex-wrap justify-center gap-4">
                      {wallets.map((name, i) => (
                        <motion.div
                          key={name}
                          initial={{ opacity: 0, y: 10 }}
                          animate={{ opacity: 1, y: 0 }}
                          transition={{ duration: 0.42, delay: i * 0.07, ease: RISE }}
                        >
                          <WalletCard name={name} onClick={() => selectWallet(name)} />
                        </motion.div>
                      ))}
                    </div>
                  )}
                </div>
                <div className="border-t border-line px-8 pb-5 pt-1">{walletActions}</div>
              </>
            )}

            {viewMode === "unlock" && selectedWallet && (
              <>
                <div className="px-8 pb-8 pt-9">
                  {/* Concentric rings read as an aperture closed over the wallet, and give the
                      emblem a size the bare name never had. Tinted by the wallet's own identity
                      so this screen and its card in the grid are recognisably the same wallet. */}
                  <div className="relative mx-auto grid h-16 w-16 place-items-center">
                    <span className="absolute -inset-7 rounded-full border" style={{ borderColor: unlockIdentity.edge, opacity: 0.3 }} />
                    <span className="absolute -inset-3.5 rounded-full border" style={{ borderColor: unlockIdentity.edge, opacity: 0.6 }} />
                    <span
                      className="absolute -inset-3.5 rounded-full"
                      style={{ background: `radial-gradient(circle, ${unlockIdentity.glow}, transparent 72%)` }}
                    />
                    <span
                      className="relative grid h-16 w-16 place-items-center rounded-full border font-header text-[19px] font-bold"
                      style={{ color: unlockIdentity.ink, background: unlockIdentity.fill, borderColor: unlockIdentity.edge }}
                    >
                      {unlockIdentity.monogram}
                    </span>
                  </div>

                  <p className="mt-6 truncate text-center font-header text-[20px] font-bold text-foreground">
                    {selectedWallet}
                  </p>
                  {/* The exact file about to be opened. Cheap reassurance in a wallet, and it is
                      the only place the user can confirm which folder they are pointed at. */}
                  {dataDir && (
                    <p className="mt-1.5 truncate text-center font-mono text-[11px] text-subtle" title={`${dataDir}/wallets/${selectedWallet}`}>
                      {dataDir}/wallets/{selectedWallet}
                    </p>
                  )}

                  <div className="mt-7 text-left">
                    <PasswordField
                      label="Password"
                      value={unlockPassword}
                      onChange={(e) => setUnlockPassword(e.target.value)}
                      onKeyDown={onEnter(() => unlockPassword && submitUnlock())}
                      placeholder="Enter wallet password"
                      autoFocus
                    />
                  </div>
                </div>
                <div className="border-t border-line px-8 py-5">
                  <div className="flex gap-3">
                    {/* Only worth offering when there is something else to go back to — with one
                        wallet on disk the grid is a single card that leads straight back here. */}
                    {(wallets?.length ?? 0) > 1 && (
                      <Button variant="secondary" className="flex-1" onClick={() => setViewMode("grid")}>
                        Back
                      </Button>
                    )}
                    <Button className="flex-1" disabled={!unlockPassword} onClick={submitUnlock}>
                      Unlock
                    </Button>
                  </div>
                  {walletActions}
                </div>
              </>
            )}

            {viewMode === "restore" && restoreSelection && (
              <>
                <div className="flex flex-col gap-5 p-8 text-left">
                  <div className="rounded-card border border-line bg-surface-raised px-4 py-3">
                    <p className="font-mono text-[10px] uppercase tracking-widest text-subtle">Selected backup</p>
                    <p className="mt-1 text-[12.5px] text-foreground">{restoreSelection.displayName}</p>
                  </div>
                  <TextField
                    label="New wallet name"
                    value={restoreName}
                    onChange={(e) => setRestoreName(e.target.value)}
                  />
                  <PasswordField
                    label="Backup password"
                    placeholder="Leave empty only for a legacy plaintext backup"
                    value={restorePassword}
                    onChange={(e) => setRestorePassword(e.target.value)}
                    onKeyDown={onEnter(submitRestore)}
                  />
                  <p className="text-[11.5px] leading-5 text-warning">
                    Portal always creates encrypted backups. An empty password is supported only
                    when importing an older backup created elsewhere.
                  </p>
                </div>
                <div className="flex gap-3 border-t border-line px-8 py-5">
                  <Button variant="secondary" onClick={() => setViewMode("grid")}>Cancel</Button>
                  <Button className="flex-1" disabled={!restoreName.trim()} onClick={submitRestore}>
                    Restore &amp; continue
                  </Button>
                </div>
              </>
            )}

            {viewMode === "create" && (
              <>
                <div className="p-8">{createFields}</div>
                <div className="border-t border-line px-8 py-5">{createActions}</div>
              </>
            )}

            {viewMode === "checking" && <div className="p-8">{checklist}</div>}
          </Card>

          {viewMode === "grid" && dataDir && (
            <p className="mt-3 text-center text-[11.5px] text-subtle">{dataDir}/wallets</p>
          )}
        </div>
      </IntroStage>
      {failureModal}
    </>
  );
}
