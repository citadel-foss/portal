import { useEffect, useState } from "react";
import { subscribe } from "../../api/transport";
import { quitApp } from "../../api/commands";
import type { QuitBlockers } from "../../api/types";
import { Button } from "../ui/inputs";
import { Modal } from "../ui/display";

/**
 * Teardown is unbounded — a router closes by finishing its connections and a full wallet sync.
 * Both states exist so that wait reads as work rather than as a hung window.
 */
export function QuitShutdown() {
  const [blockers, setBlockers] = useState<QuitBlockers | null>(null);
  const [step, setStep] = useState<string | null>(null);

  useEffect(() => {
    const unlisteners = [
      subscribe<QuitBlockers>("app://quit-blocked", setBlockers),
      subscribe("app://quitting", () => {
        setBlockers(null);
        setStep("Shutting down");
      }),
      subscribe<string>("app://quit-progress", setStep),
    ];
    return () => {
      void Promise.all(unlisteners).then((fns) => fns.forEach((fn) => fn()));
    };
  }, []);

  if (step !== null) {
    return (
      <div className="fixed inset-0 z-[60] flex flex-col items-center justify-center gap-3 bg-surface">
        <p className="font-header text-[15px] font-bold text-foreground">{step}…</p>
        <p className="max-w-sm text-center text-[12px] leading-5 text-muted">
          Letting the router and wallet finish and close their wallets. Quitting before they
          are done is what leaves state to repair on the next launch.
        </p>
      </div>
    );
  }

  if (!blockers) return null;

  const running = [
    blockers.swapRunning ? "a swap" : null,
    blockers.recoveryRunning ? "a recovery" : null,
    blockers.runningRouters.length === 1
      ? `the router ${blockers.runningRouters[0]}`
      : blockers.runningRouters.length > 1
        ? `${blockers.runningRouters.length} routers`
        : null,
  ].filter(Boolean);

  return (
    <Modal
      title="Something is still running"
      onClose={() => setBlockers(null)}
      footer={
        <>
          <Button variant="ghost" onClick={() => setBlockers(null)}>
            Keep running
          </Button>
          <Button onClick={() => void quitApp()}>
            Quit anyway
          </Button>
        </>
      }
    >
      <p className="text-[12.5px] leading-5 text-muted">
        Quitting now stops {running.join(" and ")}.
      </p>
      {blockers.swapRunning && (
        <p className="text-[12.5px] leading-5 text-muted">
          The swap is recorded at its last completed phase, so the next launch picks it up
          for recovery — but it cannot be resumed where it left off, and the funds stay in
          their timelocked contracts until recovery clears them.
        </p>
      )}
      {blockers.recoveryRunning && (
        <p className="text-[12.5px] leading-5 text-muted">
          The claim transactions are built here, so quitting pauses recovery until the next
          launch. The funds stay safe in their contracts either way — they just sit there longer.
        </p>
      )}
      {blockers.runningRouters.length > 0 && (
        <p className="text-[12.5px] leading-5 text-muted">
          Routers finish serving their current connections before stopping, so this can take
          a moment.
        </p>
      )}
    </Modal>
  );
}
