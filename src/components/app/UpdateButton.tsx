import { Download } from "lucide-react";
import { useEffect, useState } from "react";
import { checkForUpdate, installUpdate } from "../../api/commands";
import { subscribe } from "../../api/transport";
import { isAppError, type AvailableUpdate, type UpdateProgress } from "../../api/types";
import { capabilities, openExternal } from "../../platform";
import { Button } from "../ui/inputs";
import { Modal } from "../ui/display";

const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

const megabytes = (bytes: number) => (bytes / 1_000_000).toFixed(0);

/** `0.1.0-beta.36+20261017` → `0.1.0-beta.36`; the date is shown from `builtAt`. */
const buildName = (version: string) => version.split("+")[0];

/**
 * Shown only once a newer build is on the release. Restarting is the update's last step, so
 * it waits for the user's click rather than happening on its own.
 */
export function UpdateButton() {
  const [update, setUpdate] = useState<AvailableUpdate | null>(null);
  const [open, setOpen] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!capabilities.selfUpdate) return;
    // A failed check is retried on the next interval; nothing is wrong with this build.
    const check = () => void checkForUpdate().then(setUpdate).catch(() => {});
    check();
    const id = setInterval(check, CHECK_INTERVAL_MS);
    return () => clearInterval(id);
  }, []);

  useEffect(() => {
    if (!installing) return;
    const stop = subscribe<UpdateProgress>("update://progress", setProgress);
    return () => void stop.then((fn) => fn());
  }, [installing]);

  if (!update) return null;

  async function install() {
    setError(null);
    setProgress(null);
    setInstalling(true);
    try {
      // Resolves once the new build is in place; the quit overlay takes over from here.
      await installUpdate();
    } catch (e) {
      setError(isAppError(e) ? e.message : String(e));
      setInstalling(false);
    }
  }

  const close = () => {
    if (!installing) setOpen(false);
  };

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="lift inline-flex h-9 items-center gap-2 rounded-control border border-primary/40 bg-primary/[0.08] px-3 text-[12px] font-semibold text-primary outline-none hover:border-primary/60 hover:bg-primary/[0.14] focus-visible:shadow-ring"
      >
        <Download size={14} strokeWidth={2} />
        Update
      </button>
      {open && (
        <Modal
          title="Update Portal"
          onClose={close}
          footer={
            <>
              <Button variant="ghost" onClick={close} disabled={installing}>
                Not now
              </Button>
              {update.installable ? (
                <Button onClick={() => void install()} loading={installing}>
                  Update and restart
                </Button>
              ) : (
                <Button onClick={() => void openExternal(update.releasePage)}>
                  Open download page
                </Button>
              )}
            </>
          }
        >
          <p className="text-[12.5px] leading-5 text-muted">
            Build{" "}
            <span className="font-mono text-foreground">{buildName(update.version)}</span> from{" "}
            {new Date(update.builtAt).toLocaleDateString()} is available.
          </p>
          <p className="text-[12.5px] leading-5 text-muted">
            {update.installable
              ? "Portal downloads it, closes your wallets and restarts into it. Your wallets and settings are kept."
              : "This copy of Portal can't replace itself. Download the new build and install it over this one; your wallets and settings are kept."}
          </p>
          {installing && (
            <p className="font-mono text-[12px] text-foreground">
              Downloading
              {progress &&
                ` ${megabytes(progress.downloaded)}${progress.total ? ` / ${megabytes(progress.total)}` : ""} MB`}
              …
            </p>
          )}
          {error && <p className="text-[12.5px] leading-5 text-danger">{error}</p>}
        </Modal>
      )}
    </>
  );
}
