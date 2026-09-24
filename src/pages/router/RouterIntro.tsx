import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { checkTor, initRouter } from "../../api/commands";
import { Card } from "../../components/ui/display";
import { Checklist, type CheckState } from "../../components/ui/Checklist";
import { Button, PasswordField, TextField } from "../../components/ui/inputs";
import { validateNewPassword } from "../../lib/password-policy";
import { withMinDelay } from "../../lib/timing";
import { ROUTER_ID_PATTERN } from "./router-defaults";
import { AdvancedFields, FidelityFields, useRouterForm } from "./RouterForm";
import { DashboardImport } from "./DashboardImport";

// Each step usually resolves far quicker than it can be read.
const MIN_STEP_MS = 900;

type Stage = "name" | "creating";

interface Steps {
  tor: CheckState;
  create: CheckState;
}

const IDLE: Steps = { tor: "idle", create: "idle" };

/**
 * Shown instead of the dashboard when no routers are registered. The whole of new-router
 * setup lives here rather than behind a link: the bond amount and timelock are the two values
 * that cannot be changed once the bond exists, so they are on the page by default, and
 * everything that a Settings tab can change afterwards sits under Advanced.
 */
export function RouterIntro({ onImported }: { onImported: () => void }) {
  const navigate = useNavigate();
  const form = useRouterForm();
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [passwordConfirm, setPasswordConfirm] = useState("");
  const [stage, setStage] = useState<Stage>("name");
  const [steps, setSteps] = useState<Steps>(IDLE);
  const [error, setError] = useState<string | null>(null);

  const trimmed = name.trim();
  const malformed = trimmed.length > 0 && !ROUTER_ID_PATTERN.test(trimmed);
  const passwordError = validateNewPassword(password, passwordConfirm);

  // Same reason as AddRouterPage: the button alone cannot say why it will not press.
  const blockedReason = !trimmed
    ? "Enter a router name to continue."
    : malformed
      ? "Fix the router name to continue."
      : (passwordError ??
        (form.blocked ? "Resolve the warning under Advanced settings to continue." : null));

  async function create() {
    const config = !trimmed || malformed || passwordError ? null : form.config(trimmed, password);
    if (!config) return;
    setStage("creating");
    setError(null);
    setSteps({ ...IDLE, tor: "running" });

    // Sent straight back to init_maker, which overrides them from Portal's own Tor anyway;
    // the router settings still carry the pair until that sweep lands.
    let torPorts = { socksPort: 0, controlPort: 0 };
    try {
      await withMinDelay(
        (async () => {
          const status = await checkTor();
          if (!(status.reachable && status.authenticated)) {
            throw new Error(status.error ?? "Tor control port unreachable.");
          }
          torPorts = { socksPort: status.socksPort ?? 0, controlPort: status.controlPort ?? 0 };
        })(),
        MIN_STEP_MS,
      );
      setSteps((s) => ({ ...s, tor: "passed", create: "running" }));

      // Tor's live ports win over whatever the form last saw: init_maker overrides them from
      // Portal's own Tor anyway, and a restart moves them.
      await withMinDelay(initRouter({ ...config, ...torPorts }), MIN_STEP_MS);
      setSteps((s) => ({ ...s, create: "passed" }));
      setPassword("");
      setPasswordConfirm("");
      // Created but not yet bonded: setup starts it and walks the deposit.
      navigate(`/router/${encodeURIComponent(trimmed)}/setup`);
    } catch (e) {
      setSteps((s) => ({
        tor: s.tor === "running" ? "failed" : s.tor,
        create: s.create === "running" ? "failed" : s.create,
      }));
      setError((e as { message?: string })?.message ?? "Could not create the router.");
    }
  }

  return (
    <div className="mx-auto w-full max-w-xl">
      <Card className="border-line-strong">
        {stage === "name" ? (
          <>
            <div className="p-8 text-left">
              <TextField
                label="Router name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && void create()}
                placeholder="my-router"
                autoFocus
                required
                error={malformed ? "Letters, numbers, hyphens and underscores only." : undefined}
                hint={malformed ? undefined : "Names the router and its wallet."}
              />
              <div className="mt-4 flex flex-col gap-3">
                <PasswordField
                  label="Wallet password"
                  autoComplete="new-password"
                  required
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                />
                <PasswordField
                  label="Confirm wallet password"
                  autoComplete="new-password"
                  required
                  value={passwordConfirm}
                  onChange={(e) => setPasswordConfirm(e.target.value)}
                  error={password || passwordConfirm ? passwordError : undefined}
                />
                <p className="text-[11.5px] leading-5 text-subtle">
                  Portal encrypts every router wallet it creates. This password cannot be recovered
                  if it is lost.
                </p>
              </div>
              <div className="mt-5 flex flex-col gap-4 border-t border-line pt-5">
                <FidelityFields form={form} />
                <AdvancedFields form={form} routerId={trimmed} />
              </div>
            </div>
            <div className="border-t border-line px-8 py-5">
              <Button
                className="w-full"
                disabled={!trimmed || malformed || Boolean(passwordError) || form.blocked}
                onClick={() => void create()}
              >
                Create router
              </Button>
              {blockedReason && (
                <p className="mt-3 text-center text-[11.5px] text-subtle">{blockedReason}</p>
              )}
            </div>
          </>
        ) : (
          <>
            <div className="p-8 text-left">
              <Checklist
                steps={[
                  { label: "Checking Tor", state: steps.tor },
                  { label: `Creating ${trimmed}`, state: steps.create },
                ]}
              />
            </div>
            {error && (
              <div className="border-t border-line px-8 py-5 text-left">
                <p className="text-[12.5px] text-danger">{error}</p>
                <div className="mt-4 flex gap-3">
                  <Button variant="secondary" onClick={() => { setStage("name"); setSteps(IDLE); setError(null); }}>
                    Change name
                  </Button>
                  <Button className="flex-1" onClick={() => void create()}>
                    Retry
                  </Button>
                </div>
              </div>
            )}
          </>
        )}
      </Card>

      <div className="mt-4">
        <DashboardImport onImported={onImported} />
      </div>
    </div>
  );
}
