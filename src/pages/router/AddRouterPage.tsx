import { ArrowLeft, Server } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { checkBackend, initRouter } from "../../api/commands";
import { Card } from "../../components/ui/display";
import { Button, LinkButton, PasswordField, TextField } from "../../components/ui/inputs";
import { validateNewPassword } from "../../lib/password-policy";
import { useToastStore } from "../../store/toast";
import { AdvancedFields, FidelityFields, useRouterForm } from "./RouterForm";
import { ROUTER_ID_PATTERN } from "./router-defaults";

/** Adding a router to a fleet that already has one. Same form as the first-run page, with the
 *  dashboard's chrome around it instead of the intro's. */
export function AddRouterPage() {
  const navigate = useNavigate();
  const pushToast = useToastStore((state) => state.push);
  const form = useRouterForm();

  const [routerId, setRouterId] = useState("");
  const [walletPassword, setWalletPassword] = useState("");
  const [walletPasswordConfirm, setWalletPasswordConfirm] = useState("");
  const [chain, setChain] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

  const trimmedId = routerId.trim();
  const malformedId = trimmedId.length > 0 && !ROUTER_ID_PATTERN.test(trimmedId);
  const walletPasswordError = validateNewPassword(walletPassword, walletPasswordConfirm);

  useEffect(() => {
    void checkBackend()
      .then((s) => setChain(s.chain ?? null))
      .catch(() => setChain(null));
  }, []);

  const config = useMemo(
    () =>
      !trimmedId || malformedId || walletPasswordError
        ? null
        : form.config(trimmedId, walletPassword),
    [trimmedId, malformedId, walletPassword, walletPasswordError, form],
  );

  // A disabled Create button with no explanation is the whole reason an empty password reads as
  // the form being broken. Names the first thing standing in the way, in form order.
  const blockedReason = !trimmedId
    ? "Enter a router ID to continue."
    : malformedId
      ? "Fix the router ID to continue."
      : walletPasswordError
        ? walletPasswordError
        : form.blocked
          ? "Resolve the warning under Advanced settings to continue."
          : !config
            ? "Check the values above to continue."
            : null;

  async function createRouter() {
    if (!config) return;
    setCreating(true);
    try {
      await initRouter(config);
      setWalletPassword("");
      setWalletPasswordConfirm("");
      pushToast("success", `${config.routerId} was created and registered.`);
      navigate(`/router/${encodeURIComponent(config.routerId)}/setup`);
    } catch (error) {
      pushToast("error", (error as { message?: string })?.message ?? "Could not create router.");
    } finally {
      setCreating(false);
    }
  }

  return (
    <div className="h-full overflow-y-auto p-8">
      <div className="mx-auto w-full max-w-[640px] pb-8">
        <header className="mb-6">
          <Link to="/router" className="mb-4 inline-flex items-center gap-2 font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle hover:text-foreground">
            <ArrowLeft size={14} />
            Back to routers
          </Link>
          <div className="flex items-center justify-between gap-4">
            <div className="flex items-center gap-3">
              <span className="grid h-11 w-11 place-items-center rounded-card bg-primary text-on-primary">
                <Server size={22} />
              </span>
              <div>
                <h1 className="font-header text-[29px] font-bold text-foreground">Add Router</h1>
                <p className="mt-1 text-[12.5px] text-muted">Created stopped — start it once the fidelity bond is funded.</p>
              </div>
            </div>
            {chain && (
              <span className="rounded-pill border border-primary/35 bg-primary/10 px-3 py-1.5 font-mono text-[10px] uppercase tracking-widest text-primary">
                {chain}
              </span>
            )}
          </div>
        </header>

        <Card className="border-line-strong">
          <div className="p-5">
            <TextField
              label="Router ID"
              placeholder="router-02"
              autoFocus
              required
              value={routerId}
              onChange={(e) => setRouterId(e.target.value)}
              error={malformedId ? "Letters, numbers, hyphens and underscores only." : undefined}
              hint={malformedId ? undefined : "Also names its wallet. Everything below is already set."}
            />
            <div className="mt-4 flex flex-col gap-3 border-t border-line pt-4">
              <PasswordField label="Wallet password" autoComplete="new-password" required value={walletPassword} onChange={(e) => setWalletPassword(e.target.value)} />
              <PasswordField
                label="Confirm wallet password"
                autoComplete="new-password"
                required
                value={walletPasswordConfirm}
                onChange={(e) => setWalletPasswordConfirm(e.target.value)}
                error={walletPassword || walletPasswordConfirm ? walletPasswordError : undefined}
              />
              <p className="text-[11.5px] leading-5 text-subtle">
                Portal encrypts every router wallet it creates. Losing this password can make its
                funds unrecoverable.
              </p>
            </div>
            <div className="mt-5 flex flex-col gap-4 border-t border-line pt-5">
              <FidelityFields form={form} />
              <AdvancedFields form={form} routerId={trimmedId} />
            </div>
          </div>

          <div className="border-t border-line px-5 py-4">
            <p className="text-[11.5px] leading-5 text-subtle">
              Ports, fees and limits can be changed later from the router's Settings tab. The
              fidelity bond is different: this bond keeps the amount and timelock set here, and
              edits apply to the next one.
            </p>
          </div>
        </Card>

        <div className="mt-4 flex items-center justify-end gap-3">
          {blockedReason && <p className="text-[11.5px] text-subtle">{blockedReason}</p>}
          <LinkButton to="/router" variant="secondary">Cancel</LinkButton>
          <Button onClick={() => void createRouter()} loading={creating} disabled={!config || form.blocked}>
            Create router
          </Button>
        </div>
      </div>
    </div>
  );
}
