import { AlertTriangle, Check, Copy, Eye } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Card, Notice } from "../ui/display";
import { Button, SegmentedToggle } from "../ui/inputs";
import { copyText } from "../../lib/clipboard";
import type { RecoveryPhrase } from "../../api/types";

/**
 * Shown once, right after a wallet or router is created. The crate hands the phrase out a single
 * time, so nothing here is skippable: Continue waits for the words to be revealed and the user to
 * say they wrote them down.
 */
function RecoveryPhraseScreen({
  words,
  subject,
  onSaved,
}: {
  words: string;
  subject: "wallet" | "router";
  onSaved: () => Promise<void>;
}) {
  const [revealed, setRevealed] = useState(false);
  const [written, setWritten] = useState(false);
  const [copied, setCopied] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const list = words.split(" ");

  async function copy() {
    if (!(await copyText(words))) return;
    setCopied(true);
    setTimeout(() => setCopied(false), 1600);
  }

  async function finish() {
    setSaving(true);
    setSaveError(null);
    try {
      await onSaved();
    } catch (e) {
      setSaveError((e as { message?: string })?.message ?? "Could not continue. Try again.");
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="flex h-full flex-col items-center overflow-y-auto px-8 py-10">
      <Card className="flex w-full max-w-lg shrink-0 flex-col gap-5 border-line-strong p-7">
        <div>
          <h1 className="font-header text-[19px] font-bold text-foreground">
            Save your recovery phrase
          </h1>
          <p className="mt-1 text-[12.5px] leading-5 text-muted">
            These {list.length} words are the only way to restore this {subject} if this device is
            lost. Write them down on paper, in order.
          </p>
        </div>

        <div className="relative">
          <ol
            className={`grid grid-cols-3 gap-2 font-mono text-[12.5px] max-[520px]:grid-cols-2 ${
              revealed ? "" : "pointer-events-none select-none blur-md"
            }`}
            aria-hidden={!revealed}
          >
            {list.map((word, i) => (
              <li
                key={i}
                className="flex items-baseline gap-2 rounded-control border border-line bg-surface-raised px-3 py-2"
              >
                <span className="w-5 text-right text-[10.5px] text-subtle">{i + 1}</span>
                <span className="text-foreground">{word}</span>
              </li>
            ))}
          </ol>
          {!revealed && (
            <div className="absolute inset-0 grid place-items-center">
              <Button variant="secondary" size="sm" onClick={() => setRevealed(true)}>
                <Eye size={14} strokeWidth={2} /> Reveal words
              </Button>
            </div>
          )}
        </div>

        {revealed && (
          <div>
            <Button variant="secondary" size="sm" onClick={() => void copy()}>
              {copied ? <Check size={14} strokeWidth={2} /> : <Copy size={14} strokeWidth={2} />}
              {copied ? "Copied" : "Copy words"}
            </Button>
          </div>
        )}

        <Notice tone="warning" icon={<AlertTriangle size={18} strokeWidth={2} />}>
          <p className="font-semibold text-warning">
            Portal shows these words once. They cannot be shown again after you continue.
          </p>
          <p className="mt-1 text-muted">
            Anyone with them can take the funds. Never type them into a website or share them.
          </p>
        </Notice>

        <label className="flex cursor-pointer items-start gap-2.5 text-[12.5px] text-muted">
          <input
            type="checkbox"
            className="mt-0.5 accent-primary"
            checked={written}
            disabled={!revealed}
            onChange={(e) => setWritten(e.target.checked)}
          />
          I have written these words down.
        </label>

        {saveError && <p className="text-[11.5px] text-danger">{saveError}</p>}
        <Button disabled={!revealed || !written} loading={saving} onClick={() => void finish()}>
          Continue
        </Button>
      </Card>
    </div>
  );
}

/**
 * Holds `children` back while a new wallet's or router's phrase waits to be saved. Asked on every
 * mount, so a reload before confirming lands on the words again. A failed ask is not "none": the
 * words would stay held unseen, so it waits for a retry instead of letting the user through.
 */
export function RecoveryPhraseGate({
  ask,
  confirm,
  subject,
  children,
}: {
  ask: () => Promise<RecoveryPhrase | null>;
  confirm: (id: string) => Promise<void>;
  subject: "wallet" | "router";
  children: ReactNode;
}) {
  // `undefined` while asking.
  const [phrase, setPhrase] = useState<RecoveryPhrase | null | undefined>(undefined);
  const [askError, setAskError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let live = true;
    setAskError(null);
    void ask()
      .then((words) => live && setPhrase(words))
      .catch((e) => live && setAskError((e as { message?: string })?.message ?? "Could not reach Portal."));
    return () => {
      live = false;
    };
  }, [ask, attempt]);

  if (askError)
    return (
      <div className="flex h-full flex-col items-center px-8 py-10">
        <Card className="flex w-full max-w-lg flex-col gap-4 border-line-strong p-7">
          <p className="text-[12.5px] text-muted">{askError}</p>
          <Button onClick={() => setAttempt((n) => n + 1)}>Try again</Button>
        </Card>
      </div>
    );
  if (phrase === undefined) return null;
  if (phrase)
    return (
      <RecoveryPhraseScreen
        words={phrase.words}
        subject={subject}
        onSaved={async () => {
          await confirm(phrase.id);
          setPhrase(null);
        }}
      />
    );
  return <>{children}</>;
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
