import { AlertTriangle, Check, Copy, Eye, KeyRound } from "lucide-react";
import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { Card, Modal, Notice } from "../ui/display";
import { Button, PasswordField, SegmentedToggle, TextField } from "../ui/inputs";
import { copyText } from "../../lib/clipboard";
import { protectScreen } from "../../platform";
import type { RecoveryPhrase } from "../../api/types";

type Reveal = (walletPassword: string) => Promise<RecoveryPhrase>;
type Confirm = (walletPassword: string) => Promise<void>;
type Subject = "wallet" | "router";

const HIDE_AFTER_MS = 2 * 60 * 1000;
const CHECKED_WORDS = 3;

function message(e: unknown, fallback: string): string {
  return (e as { message?: string })?.message ?? fallback;
}

/** `count` distinct word positions, in phrase order. */
function pickPositions(length: number, count: number): number[] {
  const all = Array.from({ length }, (_, i) => i);
  for (let i = all.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [all[i], all[j]] = [all[j], all[i]];
  }
  return all.slice(0, count).sort((a, b) => a - b);
}

/**
 * Password first, then the words, held only in this component's state. They go the moment the
 * user hides them, closes, leaves the window, or after two minutes, and showing them again asks
 * the password again. With `confirm`, the user re-types a few of them before the phrase counts
 * as written down.
 */
function RecoveryPhrasePanel({
  subject,
  reveal,
  confirm,
}: {
  subject: Subject;
  reveal: Reveal;
  confirm?: Confirm;
}) {
  const [password, setPassword] = useState("");
  const [words, setWords] = useState<string[] | null>(null);
  const [checking, setChecking] = useState<number[] | null>(null);
  const [typed, setTyped] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hiddenNote, setHiddenNote] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  function hide(note: string | null) {
    setWords(null);
    setChecking(null);
    setTyped([]);
    setPassword("");
    setCopied(false);
    setHiddenNote(note);
  }

  // Bumped on every departure and on unmount, so a reveal still in flight when the user leaves
  // or closes is dropped rather than shown, and never turns protection on with nothing to undo it.
  const departures = useRef(0);
  const shown = words !== null;
  const shownRef = useRef(shown);
  shownRef.current = shown;
  useEffect(() => {
    const away = () => {
      departures.current += 1;
      if (shownRef.current) hide("Hidden when you left the window.");
    };
    const onVisibility = () => document.visibilityState === "hidden" && away();
    document.addEventListener("visibilitychange", onVisibility);
    window.addEventListener("blur", away);
    return () => {
      departures.current += 1;
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("blur", away);
    };
  }, []);

  useEffect(() => {
    if (!shown) return;
    const timer = setTimeout(() => hide("Hidden after two minutes."), HIDE_AFTER_MS);
    return () => {
      clearTimeout(timer);
      void protectScreen(false).catch(() => {});
    };
  }, [shown]);

  async function show() {
    if (!password) return setError(`Enter the ${subject} wallet password.`);
    setBusy(true);
    setError(null);
    setHiddenNote(null);
    const departure = departures.current;
    try {
      const phrase = await reveal(password);
      if (departures.current !== departure) return hide("Hidden when you left the window.");
      // On before the words render. A host that cannot protect the window still shows them:
      // the protection narrows exposure, it is not what keeps the phrase secret.
      await protectScreen(true).catch(() => {});
      if (departures.current !== departure) {
        void protectScreen(false).catch(() => {});
        return hide("Hidden when you left the window.");
      }
      setWords(phrase.words.split(" "));
    } catch (e) {
      setError(message(e, "Could not show the recovery phrase."));
    } finally {
      setBusy(false);
    }
  }

  async function copy() {
    if (!words || !(await copyText(words.join(" ")))) return;
    setCopied(true);
    setTimeout(() => setCopied(false), 1600);
  }

  async function check() {
    if (!words || !checking || !confirm) return;
    const matches = checking.every((position, i) => typed[i]?.trim().toLowerCase() === words[position]);
    if (!matches) return setError("Those words do not match. Check what you wrote down.");
    setBusy(true);
    setError(null);
    try {
      await confirm(password);
      hide(null);
    } catch (e) {
      setError(message(e, "Could not save. Try again."));
    } finally {
      setBusy(false);
    }
  }

  if (!words)
    return (
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          void show();
        }}
      >
        {hiddenNote && <p className="text-[11.5px] text-subtle">{hiddenNote}</p>}
        <PasswordField
          label={subject === "wallet" ? "Wallet password" : "Router wallet password"}
          autoComplete="current-password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          error={error ?? undefined}
        />
        <div>
          <Button type="submit" size="sm" loading={busy}>
            <Eye size={14} strokeWidth={2} /> Show recovery phrase
          </Button>
        </div>
      </form>
    );

  if (checking)
    return (
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          void check();
        }}
      >
        <p className="text-[12.5px] leading-5 text-muted">
          Type these words from what you wrote down.
        </p>
        <div className="grid grid-cols-3 gap-2 max-[520px]:grid-cols-1">
          {checking.map((position, i) => (
            <TextField
              key={position}
              label={`Word ${position + 1}`}
              autoComplete="off"
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              className="font-mono"
              value={typed[i] ?? ""}
              onChange={(e) => {
                const next = [...typed];
                next[i] = e.target.value;
                setTyped(next);
              }}
            />
          ))}
        </div>
        {error && <p className="text-[11.5px] text-danger">{error}</p>}
        <div className="flex flex-wrap gap-2">
          <Button type="submit" size="sm" loading={busy} disabled={typed.filter(Boolean).length < checking.length}>
            <Check size={14} strokeWidth={2} /> Confirm
          </Button>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={() => {
              setChecking(null);
              setError(null);
            }}
          >
            Show the words again
          </Button>
        </div>
      </form>
    );

  return (
    <div className="flex flex-col gap-4">
      <ol className="grid select-none grid-cols-3 gap-2 font-mono text-[12.5px] max-[520px]:grid-cols-2">
        {words.map((word, i) => (
          <li
            key={i}
            className="flex items-baseline gap-2 rounded-control border border-line bg-surface-raised px-3 py-2"
          >
            <span className="w-5 text-right text-[10.5px] text-subtle">{i + 1}</span>
            <span className="text-foreground">{word}</span>
          </li>
        ))}
      </ol>
      <div className="flex flex-wrap gap-2">
        {confirm && (
          <Button
            size="sm"
            onClick={() => {
              setTyped([]);
              setError(null);
              setChecking(pickPositions(words.length, CHECKED_WORDS));
            }}
          >
            <Check size={14} strokeWidth={2} /> I have written them down
          </Button>
        )}
        <Button variant="secondary" size="sm" onClick={() => void copy()}>
          {copied ? <Check size={14} strokeWidth={2} /> : <Copy size={14} strokeWidth={2} />}
          {copied ? "Copied" : "Copy words"}
        </Button>
        <Button variant="secondary" size="sm" onClick={() => hide(null)}>
          Hide
        </Button>
      </div>
    </div>
  );
}

const PhraseWarning = () => (
  <Notice tone="warning" icon={<AlertTriangle size={18} strokeWidth={2} />}>
    <p className="font-semibold text-warning">Anyone with these words can take the funds.</p>
    <p className="mt-1 text-muted">Write them on paper. Never type them into a website or share them.</p>
  </Notice>
);

/** Shows the phrase on request, from the wallet page or a router's overview. */
export function ShowRecoveryPhrase({ subject, reveal }: { subject: Subject; reveal: Reveal }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <p className="text-[12px] leading-5 text-muted">
        The words that restore {subject === "wallet" ? "this wallet" : "this router's wallet"} on
        any device.
      </p>
      <Button size="sm" className="mt-4" onClick={() => setOpen(true)}>
        <KeyRound size={14} strokeWidth={2} /> Show recovery phrase
      </Button>
      {open && (
        <Modal title="Recovery phrase" onClose={() => setOpen(false)}>
          <PhraseWarning />
          <RecoveryPhrasePanel subject={subject} reveal={reveal} />
        </Modal>
      )}
    </>
  );
}

const RecheckContext = createContext<() => void>(() => {});

/** Asks the nearest gate again, after an error says the phrase still has to be saved. */
export function useRecheckRecoveryPhrase(): () => void {
  return useContext(RecheckContext);
}

/**
 * Holds `children` back until the phrase is confirmed as written down. A failed ask is not
 * "saved": it waits for a retry instead of letting the user through.
 */
export function RecoveryPhraseGate({
  saved,
  reveal,
  confirm,
  subject,
  children,
}: {
  saved: () => Promise<boolean>;
  reveal: Reveal;
  confirm: Confirm;
  subject: Subject;
  children: ReactNode;
}) {
  // `undefined` while asking.
  const [isSaved, setIsSaved] = useState<boolean | undefined>(undefined);
  const [askError, setAskError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const recheck = useRef(() => setAttempt((n) => n + 1)).current;

  useEffect(() => {
    let live = true;
    setAskError(null);
    void saved()
      .then((answer) => live && setIsSaved(answer))
      .catch((e) => live && setAskError(message(e, "Could not reach Portal.")));
    return () => {
      live = false;
    };
  }, [saved, attempt]);

  if (askError)
    return (
      <div className="flex h-full flex-col items-center px-8 py-10">
        <Card className="flex w-full max-w-lg flex-col gap-4 border-line-strong p-7">
          <p className="text-[12.5px] text-muted">{askError}</p>
          <Button onClick={recheck}>Try again</Button>
        </Card>
      </div>
    );
  if (isSaved === undefined) return null;
  if (!isSaved)
    return (
      <div className="flex h-full flex-col items-center overflow-y-auto px-8 py-10">
        <Card className="flex w-full max-w-lg shrink-0 flex-col gap-5 border-line-strong p-7">
          <div>
            <h1 className="font-header text-[19px] font-bold text-foreground">
              Save your recovery phrase
            </h1>
            <p className="mt-1 text-[12.5px] leading-5 text-muted">
              These words are the only way to restore this {subject} if this device is lost. Write
              them down on paper, in order. You can show them again later with the password.
            </p>
          </div>
          <PhraseWarning />
          <RecoveryPhrasePanel
            subject={subject}
            reveal={reveal}
            confirm={async (password) => {
              await confirm(password);
              setIsSaved(true);
            }}
          />
        </Card>
      </div>
    );
  return <RecheckContext.Provider value={recheck}>{children}</RecheckContext.Provider>;
}

/** Counted, not checked: the backend judges the words and their checksum. */
function wordCount(phrase: string): number {
  return phrase.trim() ? phrase.trim().split(/\s+/).length : 0;
}

/** The lengths the word boxes offer, and so the only ones a restore accepts. */
const LENGTHS = [12, 24] as const;

export function phraseIsComplete(phrase: string): boolean {
  return (LENGTHS as readonly number[]).includes(wordCount(phrase));
}

function words(text: string): string[] {
  return text.trim().toLowerCase().split(/\s+/).filter(Boolean);
}

/**
 * One box per word. Several words typed or pasted into any box spread across the boxes — a whole
 * phrase from the first — so a phrase copied from anywhere lands in one go. `onChange` gets the
 * phrase once every box is filled, and an empty string until then.
 */
export function RecoveryPhraseInput({
  value,
  onChange,
}: {
  value: string;
  onChange: (next: string) => void;
}) {
  // The boxes are the state: blanks between words do not survive a round trip through `value`,
  // which only seeds them.
  const [boxes, setBoxes] = useState<string[]>(() => {
    const seed = words(value);
    return Array.from({ length: seed.length > 12 ? 24 : 12 }, (_, i) => seed[i] ?? "");
  });
  const inputs = useRef<(HTMLInputElement | null)[]>([]);
  const [tooLong, setTooLong] = useState<number | null>(null);
  const filled = boxes.filter(Boolean).length;

  function update(next: string[]) {
    setTooLong(null);
    setBoxes(next);
    onChange(next.every(Boolean) ? next.join(" ") : "");
  }

  function resize(length: number) {
    update(Array.from({ length }, (_, i) => boxes[i] ?? ""));
  }

  function enter(index: number, text: string) {
    const typed = words(text);
    if (typed.length <= 1) {
      const next = [...boxes];
      next[index] = typed[0] ?? "";
      update(next);
      // A space after a word is the cue to move on, as it would be in a single field.
      if (typed.length === 1 && /\s$/.test(text)) inputs.current[index + 1]?.focus();
      return;
    }
    const whole = (LENGTHS as readonly number[]).includes(typed.length);
    const length = whole ? typed.length : boxes.length;
    const start = whole ? 0 : index;
    // Cut short or shifted, a phrase of another length would only fail its checksum later.
    if (start + typed.length > length) {
      setTooLong(typed.length);
      return;
    }
    const next = Array.from({ length }, (_, i) => boxes[i] ?? "");
    typed.forEach((word, i) => (next[start + i] = word));
    update(next);
    inputs.current[Math.min(start + typed.length, length - 1)]?.focus();
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-3">
        <span className="text-[12.5px] font-medium text-muted">
          Recovery phrase
          <span className="ml-1 text-danger" aria-hidden="true">
            *
          </span>
        </span>
        <SegmentedToggle
          groupId="phrase-length"
          subdued
          value={String(boxes.length)}
          onChange={(length) => resize(Number(length))}
          options={LENGTHS.map((n) => ({ value: String(n), label: `${n} words` }))}
        />
      </div>
      <ol
        className={`grid gap-2 max-[520px]:grid-cols-2 ${boxes.length > 12 ? "grid-cols-4" : "grid-cols-3"}`}
      >
        {boxes.map((word, i) => (
          <li
            key={i}
            className="flex items-center gap-2 rounded-control border border-line bg-surface-raised px-2.5 focus-within:border-primary focus-within:shadow-ring"
          >
            <span className="w-5 shrink-0 text-right text-[10.5px] text-subtle">{i + 1}</span>
            <input
              ref={(el) => {
                inputs.current[i] = el;
              }}
              aria-label={`Word ${i + 1}`}
              type="text"
              autoComplete="off"
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              value={word}
              onChange={(e) => enter(i, e.target.value)}
              onPaste={(e) => {
                e.preventDefault();
                enter(i, e.clipboardData.getData("text"));
              }}
              onKeyDown={(e) => {
                if (e.key === "Backspace" && !word && i > 0) inputs.current[i - 1]?.focus();
              }}
              className="h-9 min-w-0 flex-1 bg-transparent font-mono text-[12.5px] text-foreground outline-none"
            />
          </li>
        ))}
      </ol>
      {tooLong ? (
        <span className="text-[11.5px] text-danger">
          That is {tooLong} words. Portal restores 12- and 24-word phrases.
        </span>
      ) : (
        <span className="text-[11.5px] text-subtle">
          {filled} of {boxes.length} words · paste the whole phrase into any box
        </span>
      )}
    </div>
  );
}
