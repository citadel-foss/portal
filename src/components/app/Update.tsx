import { ArrowDownToLine } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { checkUpdate, installUpdate } from "../../api/commands";
import { subscribe } from "../../api/transport";
import type { UpdateInfo, UpdateProgress } from "../../api/types";
import { present } from "../../lib/error-policy";
import { capabilities, openExternal } from "../../platform";
import { IconButton, Modal } from "../ui/display";
import { Button } from "../ui/inputs";

/** The app can stay open in the tray for days, so a check on launch alone goes stale. */
const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

/** `0.1.0-beta.72+20261125` reads as `0.1.0-beta.72`; the date adds nothing to the choice. */
const short = (version: string) => version.split("+")[0];

/** Header button that appears once a newer build is published, and the dialog that installs it. */
export function UpdateButton() {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [open, setOpen] = useState(false);
  const closeDialog = useCallback(() => setOpen(false), []);

  useEffect(() => {
    if (!capabilities.selfUpdate) return;
    // A failed check only means no button this time; the next one tries again.
    const check = () => void checkUpdate().then(setInfo, () => {});
    check();
    const id = setInterval(check, CHECK_INTERVAL_MS);
    return () => clearInterval(id);
  }, []);

  if (!info?.available) return null;
  return (
    <>
      <IconButton
        onClick={() => setOpen(true)}
        label="Update available"
        className="border-primary/40 text-primary hover:text-primary"
        icon={<ArrowDownToLine size={16} strokeWidth={1.8} />}
      />
      {open && <UpdateDialog info={info} onClose={closeDialog} />}
    </>
  );
}

function UpdateDialog({ info, onClose }: { info: UpdateInfo; onClose: () => void }) {
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [error, setError] = useState<{ message: string; guidance?: string } | null>(null);

  useEffect(() => {
    const stop = subscribe<UpdateProgress>("update://progress", setProgress);
    return () => void stop.then((fn) => fn());
  }, []);

  async function install() {
    setError(null);
    setProgress(null);
    setInstalling(true);
    try {
      // On success the quit overlay takes over and the app relaunches; nothing to reset here.
      await installUpdate();
    } catch (e) {
      setError(present(e, "The update couldn't be installed."));
      setInstalling(false);
    }
  }

  // Stable, so download progress re-rendering the dialog doesn't re-run the modal's focus trap.
  const close = useCallback(() => {
    if (!installing) onClose();
  }, [installing, onClose]);

  const percent =
    progress?.total ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100)) : null;

  return (
    <Modal
      title="Update available"
      onClose={close}
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={installing}>
            Later
          </Button>
          {info.inPlace ? (
            <Button onClick={() => void install()} loading={installing}>
              {installing
                ? percent === null
                  ? "Downloading…"
                  : `Downloading… ${percent}%`
                : "Update and restart"}
            </Button>
          ) : (
            <Button onClick={() => void openExternal(info.releaseUrl)}>Download</Button>
          )}
        </>
      }
    >
      <p className="text-[12.5px] leading-5 text-muted">
        Portal <span className="text-foreground">{short(info.latest)}</span> is out. You're on{" "}
        {short(info.current)}.
      </p>
      <p className="text-[12.5px] leading-5 text-muted">
        {info.inPlace
          ? "Portal downloads it, checks it against the release's checksum, and restarts. Open wallets close first, the same as quitting."
          : "This copy of Portal can't replace itself. Download the new build and install it over this one."}
      </p>
      {error && (
        <p className="text-[12.5px] leading-5 text-danger">
          {error.message}
          {error.guidance && ` ${error.guidance}`}
        </p>
      )}
    </Modal>
  );
}
