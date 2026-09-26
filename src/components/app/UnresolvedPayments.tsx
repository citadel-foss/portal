import { AlertTriangle } from "lucide-react";
import { useState } from "react";
import { Button } from "../ui/inputs";
import { Identifier, Notice, SatsAmount } from "../ui/display";
import { formatRelativeTime } from "../../lib/wallet-format";
import { spendingBlocked, useUnresolvedStore } from "../../store/unresolved";

/**
 * Shown in place of a Send or Swap action when an earlier payment's outcome is unknown.
 *
 * It is not an error and offers no retry: retrying is the one action that could pay the same
 * person twice. In the ordinary case nobody reads this for long — the transaction is in the
 * mempool and the background recheck clears it within a minute.
 */
export function UnresolvedPayments({ verb }: { verb: "sending" | "swapping" }) {
  const blocking = useUnresolvedStore((s) => s.blocking);
  const checked = useUnresolvedStore((s) => s.checked);
  const held = useUnresolvedStore(spendingBlocked);
  const acknowledge = useUnresolvedStore((s) => s.acknowledge);
  const [working, setWorking] = useState<string | null>(null);

  if (!held) return null;

  // Held without a list to show: the journal could not be read, so whether anything is
  // outstanding is unknown. Say that, rather than leaving a disabled button unexplained.
  if (!checked) {
    return (
      <Notice tone="warning" icon={<AlertTriangle size={16} strokeWidth={2} />}>
        <p>
          Portal cannot currently tell whether an earlier payment is still unconfirmed, so{" "}
          {verb} is paused. It retries every minute.
        </p>
      </Notice>
    );
  }

  return (
    <Notice tone="warning" icon={<AlertTriangle size={16} strokeWidth={2} />}>
      <div className="flex flex-col gap-3">
        <p>
          {blocking.length === 1 ? "An earlier payment is" : `${blocking.length} earlier payments are`}{" "}
          still unconfirmed, so {verb} is paused. {verb === "sending" ? "Sending" : "Swapping"}{" "}
          again now could pay twice.
        </p>

        {blocking.map((op) => {
          const amount = op.request?.amountSats;
          const address = op.request?.address;
          // With a txid there is something to look for, so the background recheck will settle
          // it. Without one there is not, and only the owner can say what happened.
          const checkable = Boolean(op.result?.txid);
          return (
            <div key={op.operationId} className="flex flex-col gap-2 border-t border-line pt-3">
              <span className="text-[12.5px] text-foreground">
                {amount !== undefined && <SatsAmount sats={amount} />}
                {address && <> to <Identifier value={address} /></>}
                {" · "}
                {formatRelativeTime(op.createdAt)}
              </span>
              {checkable ? (
                <span className="text-[11.5px] text-subtle">
                  Portal is checking the chain every minute. This clears itself once the
                  transaction confirms.
                </span>
              ) : (
                <>
                  <span className="text-[11.5px] text-subtle">
                    Portal never recorded a transaction ID for this one, so it cannot check.
                    Confirm with the recipient, or look for the amount in your wallet history.
                  </span>
                  <Button
                    size="sm"
                    variant="secondary"
                    loading={working === op.operationId}
                    onClick={() => {
                      setWorking(op.operationId);
                      void acknowledge(op.operationId).finally(() => setWorking(null));
                    }}
                  >
                    It didn't go through — let me {verb === "sending" ? "send" : "swap"} again
                  </Button>
                </>
              )}
            </div>
          );
        })}
      </div>
    </Notice>
  );
}
