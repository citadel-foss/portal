import { LogOut } from "lucide-react";
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { lockWallet } from "../../api/commands";
import { present } from "../../lib/error-policy";
import { Checklist, type CheckState } from "../ui/Checklist";
import { IconButton, Modal } from "../ui/display";
import { Button } from "../ui/inputs";
import { useSessionStore } from "../../store/session";
import { useTxNoticeStore } from "../../store/tx-notifications";
import { useUnresolvedStore } from "../../store/unresolved";
import { useWalletCacheStore } from "../../store/wallet-cache";

/** How long the release may run before the wait is explained rather than merely shown. */
const SLOW_RELEASE_MS = 4_000;

const STEP_LABELS = [
  "Releasing the wallet",
  "Clearing this session",
  "Returning to your wallets",
];

/**
 * Releases the wallet and returns to the wallet picker, so a different one can be unlocked
 * without restarting Portal.
 *
 * Not a sign-out: on the web the session and the server stay up, and any router keeps running.
 *
 * Runs behind a modal because the first step is both the slow one and the silent one —
 * `shutdown_taker` cannot take the wallet until an in-flight sync lets go of it, which is
 * several seconds of a window that looks frozen if nothing says what is happening. The backend
 * refusing while a swap holds the wallet is reported on the step that failed, rather than as a
 * toast over a page that has already moved on.
 */
export function SwitchWallet() {
  const reset = useSessionStore((s) => s.reset);
  const resetCache = useWalletCacheStore((s) => s.reset);
  const resetUnresolved = useUnresolvedStore((s) => s.reset);
  const resetNotices = useTxNoticeStore((s) => s.reset);
  const navigate = useNavigate();
  const [step, setStep] = useState<number | null>(null);
  const [slow, setSlow] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function switchWallet() {
    if (step !== null) return;
    setError(null);
    setSlow(false);
    setStep(0);
    const explain = setTimeout(() => setSlow(true), SLOW_RELEASE_MS);
    try {
      await lockWallet();
    } catch (e) {
      const { message, guidance } = present(e, "Could not close the wallet.");
      setError(guidance ? `${message} ${guidance}` : message);
      return;
    } finally {
      clearTimeout(explain);
      setSlow(false);
    }

    // Only after the backend has actually let go: clearing first would leave the UI claiming
    // no wallet while the process still held one.
    setStep(1);
    resetCache();
    resetUnresolved();
    resetNotices();
    reset();

    setStep(2);
    // The wallet picker, not the role chooser — the role is not in question. Same target
    // `RequireWallet` falls back to, so the two cannot race to different pages.
    navigate("/setup", { replace: true });
  }

  const dismiss = () => setStep(null);
  const steps = STEP_LABELS.map((label, i) => {
    const state: CheckState =
      step === null || i > step
        ? "idle"
        : i < step
          ? "passed"
          : error !== null
            ? "failed"
            : "running";
    return { label, state };
  });

  return (
    <>
      <IconButton
        onClick={() => void switchWallet()}
        disabled={step !== null}
        label="Close wallet"
        icon={<LogOut size={16} strokeWidth={1.8} />}
      />
      {step !== null && (
        <Modal
          title={error !== null ? "Could not close the wallet" : "Closing your wallet"}
          // Dismissible only once it has stopped: closing this mid-release would leave the page
          // claiming a wallet the backend is in the middle of taking away.
          onClose={() => error !== null && dismiss()}
          footer={
            error !== null && (
              <Button variant="secondary" onClick={dismiss}>
                Back to wallet
              </Button>
            )
          }
        >
          <Checklist steps={steps} />
          {error !== null ? (
            <p className="text-[12.5px] leading-5 text-danger">{error}</p>
          ) : (
            slow && (
              <p className="text-[12.5px] leading-5 text-subtle">
                The wallet is finishing what it was already doing. It will let go on its own.
              </p>
            )
          )}
        </Modal>
      )}
    </>
  );
}
