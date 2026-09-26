import { LogOut } from "lucide-react";
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { lockWallet } from "../../api/commands";
import { IconButton } from "../ui/display";
import { useSessionStore } from "../../store/session";
import { useToastStore } from "../../store/toast";
import { useTxNoticeStore } from "../../store/tx-notifications";
import { useUnresolvedStore } from "../../store/unresolved";
import { useWalletCacheStore } from "../../store/wallet-cache";

/**
 * Takes this window off its wallet and returns to the wallet picker, so a different one can be
 * unlocked without restarting Portal.
 *
 * Not a sign-out: on the web the session stays, and any router keeps running. The wallet itself
 * stays open while another browser is on it or a swap is running, and otherwise finishes closing
 * in the background — which is why this returns at once instead of waiting on it.
 */
export function SwitchWallet() {
  const reset = useSessionStore((s) => s.reset);
  const resetCache = useWalletCacheStore((s) => s.reset);
  const resetUnresolved = useUnresolvedStore((s) => s.reset);
  const resetNotices = useTxNoticeStore((s) => s.reset);
  const navigate = useNavigate();
  const [busy, setBusy] = useState(false);

  async function switchWallet() {
    if (busy) return;
    setBusy(true);
    try {
      await lockWallet();
    } catch (e) {
      useToastStore.getState().pushFailure(e, "Could not close the wallet.");
      setBusy(false);
      return;
    }
    // Only after the backend has let go: clearing first would leave the UI claiming no wallet
    // while this session was still on one.
    resetCache();
    resetUnresolved();
    resetNotices();
    reset();
    // The wallet picker, not the role chooser — the role is not in question. Same target
    // `RequireWallet` falls back to, so the two cannot race to different pages.
    navigate("/setup", { replace: true });
  }

  return (
    <IconButton
      onClick={() => void switchWallet()}
      disabled={busy}
      label="Close wallet"
      icon={<LogOut size={16} strokeWidth={1.8} />}
    />
  );
}
