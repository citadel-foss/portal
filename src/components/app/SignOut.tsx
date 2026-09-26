import { DoorOpen } from "lucide-react";
import { useState } from "react";
import { session } from "../../platform";
import { IconButton } from "../ui/display";

/**
 * Ends this session. Its wallet closes unless another browser is on it or a swap is running;
 * routers keep running.
 *
 * Reloads rather than resetting stores one by one: every store is this tab's view of a session
 * that no longer exists, and a reload clears all of them, including any added later.
 */
export function SignOut() {
  const [busy, setBusy] = useState(false);

  async function signOut() {
    if (busy) return;
    setBusy(true);
    try {
      await session.logout();
    } finally {
      window.location.hash = "#/login";
      window.location.reload();
    }
  }

  return (
    <IconButton
      onClick={() => void signOut()}
      disabled={busy}
      label="Sign out"
      icon={<DoorOpen size={16} strokeWidth={1.8} />}
    />
  );
}
