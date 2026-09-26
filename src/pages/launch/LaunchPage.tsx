import { ArrowLeftRight, Server } from "lucide-react";
import { motion } from "framer-motion";
import { Link, Navigate } from "react-router-dom";
import { Background } from "../../components/ui/layout";
import { useSessionStore } from "../../store/session";
import logoUrl from "../../assets/logo.png";

const RISE = [0.16, 1, 0.3, 1] as const;

/**
 * Deliberately quiet, with no `IntroStage`: both branches play their own arrival
 * sequence (wallet unlock, first-router creation), and a third one here would make the
 * user sit through the wordmark twice before doing anything.
 */
function RoleCard({
  to,
  accent,
  icon,
  title,
  description,
  delay,
}: {
  to: string;
  accent?: "router";
  icon: React.ReactNode;
  title: string;
  description: string;
  delay: number;
}) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 16 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.55, delay, ease: RISE }}
      // Scopes hairline's and lift's primary-derived glow to the router accent.
      data-accent={accent}
      className="flex-1"
    >
      <Link
        to={to}
        className="hairline lift group flex h-full flex-col items-center gap-4 rounded-card border border-line bg-surface-raised/50 px-8 py-10 text-center outline-none hover:border-line-strong hover:bg-surface-raised/75 focus-visible:border-primary/60 focus-visible:shadow-ring"
      >
        <span className="grid h-12 w-12 place-items-center rounded-card border border-primary/25 bg-primary/[0.08] text-primary">
          {icon}
        </span>
        <span className="font-header text-[17px] font-bold text-foreground">
          {title}
        </span>
        <span className="max-w-[26ch] text-[12.5px] leading-relaxed text-muted">
          {description}
        </span>
        <span className="mt-auto pt-3 font-mono text-[10.5px] uppercase tracking-[0.18em] text-subtle transition-colors group-hover:text-primary">
          Continue →
        </span>
      </Link>
    </motion.div>
  );
}

export function LaunchPage() {
  const initialized = useSessionStore((s) => s.initialized);
  // Nothing to choose once a wallet is unlocked; a stale history entry must not strand it here.
  if (initialized) return <Navigate to="/" replace />;

  return (
    <div className="relative min-h-screen">
      <Background />
      <div className="relative grid min-h-screen place-items-center px-6 py-16">
        <div className="w-full max-w-3xl">
          <motion.header
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.5, ease: RISE }}
            className="text-center"
          >
            {/* White plate: the mark is two-tone, and its black half reads as a hole in the
                logo against a dark ground. */}
            <img
              src={logoUrl}
              alt=""
              aria-hidden="true"
              className="mx-auto h-11 w-11 rounded-full bg-white p-0.5 shadow-[0_6px_16px_-8px_rgba(0,0,0,0.9)]"
            />
            <h1 className="mt-5 font-header text-[26px] font-bold leading-none text-foreground">
              Portal
            </h1>
            <p className="mt-3 text-[13px] text-muted">
              Choose how you want to use it.
            </p>
          </motion.header>

          <div className="mt-9 flex flex-col gap-4 sm:flex-row">
            <RoleCard
              to="/setup"
              icon={<ArrowLeftRight size={22} strokeWidth={1.8} />}
              title="Wallet"
              description="Unlock wallets to perform routed swaps."
              delay={0.12}
            />
            <RoleCard
              to="/router"
              accent="router"
              icon={<Server size={22} strokeWidth={1.8} />}
              title="Router"
              description="Run swap routers and earn fees."
              delay={0.2}
            />
          </div>
        </div>
      </div>
    </div>
  );
}
