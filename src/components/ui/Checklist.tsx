import { motion, useReducedMotion } from "framer-motion";
import { AlertCircle } from "lucide-react";

export type CheckState = "idle" | "running" | "passed" | "failed";

const EASE = [0.16, 1, 0.3, 1] as const;

/** Draws itself on rather than appearing, so completion registers as a moment. */
function DrawnCheck() {
  return (
    <svg viewBox="0 0 24 24" fill="none" className="h-3.5 w-3.5 text-success">
      <motion.path
        d="M5 12.5l4.5 4.5L19 7"
        stroke="currentColor"
        strokeWidth={2.6}
        strokeLinecap="round"
        strokeLinejoin="round"
        initial={{ pathLength: 0, opacity: 0 }}
        animate={{ pathLength: 1, opacity: 1 }}
        transition={{ duration: 0.34, ease: "easeOut" }}
      />
    </svg>
  );
}

function Marker({ state }: { state: CheckState }) {
  const reduceMotion = useReducedMotion();
  return (
    <span
      className={`relative flex h-6 w-6 flex-none items-center justify-center rounded-full border transition-colors duration-500 ${
        state === "passed"
          ? "border-success/55 bg-success/12"
          : state === "failed"
            ? "border-danger/60 bg-danger/12"
            : state === "running"
              ? "border-primary bg-primary/10"
              : "border-line bg-surface-raised"
      }`}
    >
      {state === "passed" && <DrawnCheck />}
      {state === "failed" && <AlertCircle size={13} strokeWidth={2.2} className="text-danger" />}
      {state === "running" && (
        <>
          <motion.span
            className="h-1.5 w-1.5 rounded-full bg-primary"
            animate={{ opacity: reduceMotion ? 1 : [1, 0.35, 1] }}
            transition={{ duration: 1.5, repeat: reduceMotion ? 0 : Infinity, ease: "easeInOut" }}
          />
          {/* Radar ring rather than a bar sliding on a loop: it reads as waiting instead of
              as progress it cannot actually measure. */}
          <motion.span
            className="absolute inset-0 rounded-full border border-primary"
            animate={{ scale: reduceMotion ? 1 : [1, 1.75], opacity: reduceMotion ? 0.35 : [0.55, 0] }}
            transition={{ duration: 1.7, repeat: reduceMotion ? 0 : Infinity, ease: "easeOut" }}
          />
        </>
      )}
      {state === "idle" && <span className="h-1.5 w-1.5 rounded-full bg-subtle/40" />}
    </span>
  );
}

/** Blinks on the marker's own rhythm, so the badge and the dot beside it read as one signal. */
function WaitingBadge({ text }: { text: string }) {
  const reduceMotion = useReducedMotion();
  return (
    <motion.span
      animate={{ opacity: reduceMotion ? 1 : [1, 0.4, 1] }}
      transition={{ duration: 1.5, repeat: reduceMotion ? 0 : Infinity, ease: "easeInOut" }}
      className="w-fit rounded-pill border border-warning/40 bg-warning/[0.1] px-2 py-0.5 font-mono text-[9.5px] uppercase tracking-[0.14em] text-warning"
    >
      {text}
    </motion.span>
  );
}

/**
 * A run of checks as a connected stepper. The spine's filled length is the progress signal,
 * so each row only has to say what it is rather than carry its own progress bar.
 */
export function Checklist({
  steps,
}: {
  steps: { label: string; state: CheckState; badge?: string }[];
}) {
  const reduceMotion = useReducedMotion();
  return (
    <div className="flex flex-col">
      {steps.map(({ label, state, badge }, i) => (
        <motion.div
          key={i}
          initial={{ opacity: reduceMotion ? 1 : 0, y: reduceMotion ? 0 : 8 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.4, delay: i * 0.07, ease: EASE }}
          className="relative flex gap-4"
        >
          <div className="flex flex-col items-center">
            <Marker state={state} />
            {i < steps.length - 1 && (
              <span className="relative my-1.5 w-px flex-1 bg-line">
                <motion.span
                  className="absolute inset-0 origin-top bg-success/60"
                  initial={{ scaleY: 0 }}
                  animate={{ scaleY: state === "passed" ? 1 : 0 }}
                  transition={{ duration: 0.5, ease: EASE }}
                />
              </span>
            )}
          </div>
          <span
            className={`flex flex-col items-start gap-1.5 text-[14px] font-medium transition-colors duration-500 ${
              // Sets the row's height, and so the length of the spine segment beside it.
              i < steps.length - 1 ? "pb-7" : ""
            } ${
              state === "idle"
                ? "text-subtle"
                : state === "failed"
                  ? "text-danger"
                  : state === "running"
                    ? "text-foreground"
                    : "text-muted"
            }`}
          >
            {label}
            {badge && <WaitingBadge text={badge} />}
          </span>
        </motion.div>
      ))}
    </div>
  );
}
