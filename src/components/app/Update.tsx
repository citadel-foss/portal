import { ArrowDownToLine } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { checkUpdate, installUpdate, updateBlockers, updateChanges } from "../../api/commands";
import { subscribe } from "../../api/transport";
import type { Change, Changes, QuitBlockers, UpdateInfo, UpdateProgress } from "../../api/types";
import { present } from "../../lib/error-policy";
import { awaitRestart, capabilities, openExternal } from "../../platform";
import { IconButton, Modal, Notice } from "../ui/display";
import { Button } from "../ui/inputs";

/** The app can stay open in the tray for days, so a check on launch alone goes stale. */
const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

/** Stopping a router elsewhere in the app re-enables the update this soon. */
const BLOCKERS_POLL_MS = 3000;

/** `0.1.0-beta.72+20261125` reads as `0.1.0-beta.72`; the date adds nothing to the choice. */
const short = (version: string) => version.split("+")[0];

/** Header button that appears once a newer build is published, and the dialog that installs it. */
export function UpdateButton() {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  // The offer the dialog was opened on, so a later check cannot unmount it mid-install.
  const [offer, setOffer] = useState<UpdateInfo | null>(null);
  const closeDialog = useCallback(() => setOffer(null), []);

  useEffect(() => {
    if (!capabilities.selfUpdate) return;
    // A failed check only means no button this time; the next one tries again.
    const check = () => void checkUpdate().then(setInfo, () => {});
    check();
    const id = setInterval(check, CHECK_INTERVAL_MS);
    return () => clearInterval(id);
  }, []);

  return (
    <>
      {info?.available && (
        <IconButton
          onClick={() => setOffer(info)}
          label="Update available"
          className="border-primary/40 text-primary hover:text-primary"
          icon={<ArrowDownToLine size={16} strokeWidth={1.8} />}
        />
      )}
      {offer && <UpdateDialog info={offer} onClose={closeDialog} />}
    </>
  );
}

function UpdateDialog({ info, onClose }: { info: UpdateInfo; onClose: () => void }) {
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [error, setError] = useState<{ message: string; guidance?: string } | null>(null);
  const [restarting, setRestarting] = useState(false);
  const [changes, setChanges] = useState<Changes | { failed: string } | null>(null);
  const [blockers, setBlockers] = useState<QuitBlockers | null>(null);

  useEffect(() => {
    const stop = subscribe<UpdateProgress>("update://progress", setProgress);
    return () => void stop.then((fn) => fn());
  }, []);

  useEffect(() => {
    updateChanges().then(setChanges, (e) =>
      setChanges({ failed: present(e, "Couldn't load what changed.").message }),
    );
  }, []);

  useEffect(() => {
    if (!info.inPlace || installing) return;
    let live = true;
    let timer: ReturnType<typeof setTimeout>;
    // Sequential, not an interval: one check can wait out a wallet sync.
    const poll = async () => {
      // Unknown is not blocked: the install checks again itself.
      const next = await updateBlockers().catch(() => null);
      if (!live) return;
      setBlockers(next);
      timer = setTimeout(() => void poll(), BLOCKERS_POLL_MS);
    };
    void poll();
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [info.inPlace, installing]);

  const running = blockers
    ? [
        ...blockers.runningRouters.map((id) => `router ${id}`),
        ...(blockers.swapRunning ? ["a swap"] : []),
        ...(blockers.recoveryRunning ? ["a recovery"] : []),
      ]
    : [];
  const blocked = running.length > 0;

  async function install() {
    setError(null);
    setProgress(null);
    setInstalling(true);
    try {
      await installUpdate();
    } catch (e) {
      setError(present(e, "The update couldn't be installed."));
      setInstalling(false);
      return;
    }
    setRestarting(true);
    try {
      await awaitRestart();
    } catch (e) {
      setRestarting(false);
      setInstalling(false);
      setError(present(e, "Portal didn't come back after the update."));
    }
  }

  // Stable, so download progress re-rendering the dialog doesn't re-run the modal's focus trap.
  const close = useCallback(() => {
    if (!installing) onClose();
  }, [installing, onClose]);

  const percent =
    progress?.total ? Math.min(100, Math.round((progress.downloaded / progress.total) * 100)) : null;

  if (restarting) {
    // Portalled: the header's backdrop-filter would otherwise confine a fixed overlay to itself.
    return createPortal(
      <div className="fixed inset-0 z-[55] flex flex-col items-center justify-center gap-3 bg-surface">
        <p className="font-header text-[15px] font-bold text-foreground">Restarting Portal…</p>
        <p className="max-w-sm text-center text-[12.5px] leading-5 text-muted">
          Routers and wallets close first, then Portal{" "}
          <span className="whitespace-nowrap">{short(info.latest)}</span> starts. Sign in again
          once it's back.
        </p>
      </div>,
      document.body,
    );
  }

  return (
    <Modal
      title="Update available"
      wide
      onClose={close}
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={installing}>
            Later
          </Button>
          {(!info.inPlace || error) && (
            <Button
              variant={info.inPlace ? "ghost" : "primary"}
              onClick={() => void openExternal(info.releaseUrl)}
              disabled={installing}
            >
              Download
            </Button>
          )}
          {info.inPlace && (
            <Button
              onClick={() => void install()}
              loading={installing}
              disabled={blocked && !installing}
            >
              {installing
                ? percent === null
                  ? "Downloading…"
                  : `Downloading… ${percent}%`
                : error
                  ? "Try again"
                  : "Update and restart"}
            </Button>
          )}
        </>
      }
    >
      <p className="text-[12.5px] leading-5 text-muted">
        Portal <span className="text-foreground">{short(info.latest)}</span> is out. You're on{" "}
        {short(info.current)}.
      </p>
      <ChangeList changes={changes} />
      <p className="text-[12.5px] leading-5 text-muted">
        {info.inPlace
          ? "Portal downloads it, checks it against the release's checksum, and restarts. Open wallets close first, the same as quitting, and everyone signed in has to sign in again."
          : "This copy of Portal can't replace itself. Download the new build and install it over this one."}
      </p>
      {info.inPlace && blocked && !installing && (
        <Notice tone="warning">
          Stop {list(running)} first. Updating restarts Portal, which would cut{" "}
          {running.length === 1 ? "it" : "them"} off.
        </Notice>
      )}
      {error && (
        <p className="text-[12.5px] leading-5 text-danger">
          {error.message}
          {error.guidance && ` ${error.guidance}`}
        </p>
      )}
    </Modal>
  );
}

/** "a", "a and b", "a, b and c". */
function list(items: string[]) {
  return items.length < 2
    ? items.join("")
    : `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

function ChangeList({ changes }: { changes: Changes | { failed: string } | null }) {
  if (changes === null) {
    return <p className="text-[12.5px] leading-5 text-subtle">Loading what changed…</p>;
  }
  if ("failed" in changes) {
    return (
      <p className="text-[12.5px] leading-5 text-subtle">
        {changes.failed} The release page lists every build.
      </p>
    );
  }
  if (changes.portal.length === 0 && changes.openswap.length === 0) {
    return (
      <p className="text-[12.5px] leading-5 text-subtle">
        {changes.complete
          ? "No merged pull requests between these builds."
          : "The release doesn't list changes this far back. The release page lists every build."}
      </p>
    );
  }
  return (
    <div className="max-h-64 space-y-3 overflow-y-auto rounded-card border border-line px-4 py-3">
      <ChangeGroup title="Portal" changes={changes.portal} />
      <ChangeGroup title="OpenSwap" changes={changes.openswap} />
      {!changes.complete && (
        <p className="text-[12px] leading-5 text-subtle">
          Only the newest changes are listed. The release page has the rest.
        </p>
      )}
    </div>
  );
}

function ChangeGroup({ title, changes }: { title: string; changes: Change[] }) {
  if (changes.length === 0) return null;
  return (
    <section>
      <h3 className="mb-1.5 font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle">
        {title}
      </h3>
      <ul className="space-y-1">
        {changes.map((change) => (
          <li key={change.number} className="flex gap-2 text-[12.5px] leading-5 text-muted">
            <span className="min-w-0 flex-1">{change.title}</span>
            <button
              type="button"
              onClick={() => void openExternal(change.url)}
              className="shrink-0 text-primary hover:underline"
            >
              #{change.number}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
