import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Card } from "../../components/ui/display";
import { Button, PasswordField } from "../../components/ui/inputs";
import { IntroStage } from "../../components/ui/IntroStage";
import { session } from "../../platform";
import { useSessionStore } from "../../store/session";

/**
 * The front door on both hosts. Nothing behind it is reachable before signing in, so this is
 * the first thing Portal shows.
 *
 * A fresh install has no owner: the first visitor chooses the owner password here.
 */
export function LoginPage() {
  const hasOwner = useSessionStore((s) => s.hasOwner);
  const setHasOwner = useSessionStore((s) => s.setHasOwner);
  const setAuthenticated = useSessionStore((s) => s.setAuthenticated);
  const navigate = useNavigate();
  // Asked here, not inherited: a reload lands on this route directly (sign-out, an expired
  // session), bypassing the session gate that would otherwise have learned whether an owner
  // exists — and a stale guess shows a sign-in form for a password nobody has set yet.
  const [checked, setChecked] = useState(false);

  useEffect(() => {
    void session
      .restore()
      .then((info) => {
        setHasOwner(info.hasOwner);
        if (info.authenticated) {
          setAuthenticated(true);
          navigate("/connect", { replace: true });
        }
      })
      // An unreachable server is reported by the form's own submit; nothing to decide here.
      .catch(() => {})
      .finally(() => setChecked(true));
  }, [navigate, setAuthenticated, setHasOwner]);

  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const claiming = !hasOwner;
  const ready = claiming
    ? password.length >= 8 && password === confirm
    : password.length > 0;

  async function submit() {
    if (!ready || busy) return;
    setBusy(true);
    setError(null);
    try {
      if (claiming) await session.claim(password);
      else await session.login(password);
      setAuthenticated(true);
      navigate("/connect", { replace: true });
    } catch (e) {
      setError((e as { message?: string })?.message ?? "Could not sign in.");
    } finally {
      setBusy(false);
    }
  }

  const onEnter = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") void submit();
  };

  // Nothing rather than a guess: the two forms ask for different things.
  if (!checked) return null;

  return (
    <IntroStage
      lead={claiming ? "Set up" : "Welcome to"}
      accent="Portal"
      caption={claiming ? "Choose your Portal password" : "Sign in to Portal"}
      className="min-h-screen"
    >
      <div className="mx-auto w-full max-w-md">
        <Card className="hairline border-line-strong">
          <div className="flex flex-col gap-5 p-8 text-left">
            {claiming && (
              <p className="text-[12.5px] leading-5 text-muted">
                Portal has no password yet. Choose the one you'll sign in with from now on — in
                this app and in Portal's web version on this machine.
              </p>
            )}
            <PasswordField
              label={claiming ? "New password" : "Password"}
              required
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              onKeyDown={claiming ? undefined : onEnter}
              hint={
                claiming
                  ? "At least 8 characters. This is separate from every wallet password."
                  : undefined
              }
            />
            {claiming && (
              <PasswordField
                label="Confirm password"
                required
                value={confirm}
                onChange={(e) => setConfirm(e.target.value)}
                onKeyDown={onEnter}
                error={confirm.length > 0 && confirm !== password ? "Passwords don't match." : undefined}
              />
            )}
            {error && <p className="text-[12.5px] text-danger">{error}</p>}
            <Button disabled={!ready || busy} onClick={() => void submit()}>
              {busy ? "Working…" : claiming ? "Set password and continue" : "Sign in"}
            </Button>
          </div>
        </Card>
      </div>
    </IntroStage>
  );
}
