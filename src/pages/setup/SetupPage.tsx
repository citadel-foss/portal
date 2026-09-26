import { useNavigate } from "react-router-dom";
import type { InitResult } from "../../api/types";
import { startWalletSynchronization } from "../../lib/wallet-sync";
import { useSessionStore } from "../../store/session";
import { useToastStore } from "../../store/toast";
import { useWalletCacheStore } from "../../store/wallet-cache";
import { SelectWalletStep } from "./SelectWalletStep";

export function SetupPage() {
  const navigate = useNavigate();
  const setInitialized = useSessionStore((s) => s.setInitialized);
  const beginWalletSession = useWalletCacheStore((s) => s.beginSession);

  function completeSetup(result: InitResult, restored: boolean) {
    // Joining is instant where a fresh unlock takes a while, so say why.
    if (result.joined) {
      useToastStore
        .getState()
        .push(
          result.note ? "warning" : "success",
          result.note ?? "This wallet was already open in another browser, so this one joined it.",
        );
    }
    beginWalletSession(result.walletName, result.dataDir, restored);
    setInitialized(result);
    void startWalletSynchronization();
    // The router is always mounted now, so reaching the app is an explicit navigation
    // rather than a re-render of the session gate.
    navigate("/", { replace: true });
  }

  return (
    <div className="relative min-h-screen">
      {/* Framing and vertical placement are the step's own call: a first launch drops the card
          entirely and anchors to the top, so the intro can play against an empty screen. */}
      <div className="relative">
        <SelectWalletStep onSuccess={completeSetup} />
      </div>
    </div>
  );
}
