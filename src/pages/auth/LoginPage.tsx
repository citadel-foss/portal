import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { Card } from "../../components/ui/display";
import { Button, PasswordField, TextField } from "../../components/ui/inputs";
import { IntroStage } from "../../components/ui/IntroStage";
import { session } from "../../platform";
import { useSessionStore } from "../../store/session";

/**
 * The web host's front door. Nothing behind it is reachable unauthenticated, so this is the
 * first thing a self-hosted install shows.
 *
 * A fresh install has no owner: the first visitor claims it with the one-time secret printed
 * where the server was started. That secret is not the password — it exists so an install
 * reachable on a network cannot be claimed by whoever finds it first.
 */
export function LoginPage() {
  const hasOwner = useSessionStore((s) => s.hasOwner);
  const setAuthenticated = useSessionStore((s) => s.setAuthenticated);
  const navigate = useNavigate();

  const [secret, setSecret] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const claiming = !hasOwner;
  const ready = claiming
    ? secret.trim().length > 0 && password.length >= 8 && password === confirm
    : password.length > 0;

  async function submit() {
    if (!ready || busy) return;
    setBusy(true);
    setError(null);
    try {
      if (claiming) await session.claim(secret.trim(), password);
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

  return (
    <IntroStage
      lead={claiming ? "Set up" : "Welcome to"}
      accent="Portal"
      caption={claiming ? "Claim this installation" : "Sign in to your server"}
      className="min-h-screen"
    >
      <div className="mx-auto w-full max-w-md">
        <Card className="hairline border-line-strong">
          <div className="flex flex-col gap-5 p-8 text-left">
            {claiming && (
              <>
                <p className="text-[12.5px] leading-5 text-muted">
                  This server has no owner yet. Paste the one-time setup secret shown where you
                  started it, then choose the password you'll sign in with from now on.
                </p>
                <TextField
                  label="Setup secret"
                  required
                  value={secret}
                  onChange={(e) => setSecret(e.target.value)}
                />
              </>
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
              {busy ? "Working…" : claiming ? "Claim and continue" : "Sign in"}
            </Button>
          </div>
        </Card>
      </div>
    </IntroStage>
  );
}
