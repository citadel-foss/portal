# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## What this is

An OpenSwap taker desktop app: Tauri 2 (Rust) backend + React 18/TypeScript/Vite/Tailwind v4
frontend. It's a rewrite of an older Electron app (`../taker-app`), using the `openswap` crate
(`../coinswap`, named `coinswap` before upstream PR #988) directly as a library instead of the old
app's N-API/FFI bridge.

Design docs live in `.Codex/` (gitignored, local-only — never commit them): `DESIGN.md`
(overall architecture), `BACKEND.md` (crate integration guide + every verified gotcha),
`FRONTEND.md` (UX spec + Electron-to-Tauri command mapping), `DESIGN_SYSTEM.md` (exact
colors/typography), `API_FINDINGS.md`, `MAKER_INTEGRATION.md` (plan for porting maker
functionality from `../maker-dashboard` — not yet implemented). Read the relevant one before
deep backend or frontend
work — this file is intentionally short and does not duplicate their content.

## Commands

Frontend (run from repo root):
- `npm run tauri dev` — full app, real desktop window (Tauri + Vite together)
- `npm run dev` — Vite dev server alone, no Tauri/IPC bridge (only useful for pure UI iteration)

Whole stack (run from repo root):
- `npm run sync` — `scripts/sync.sh`: update the openswap crate, `npm install`, then verify
  (clippy, `cargo test`, `tsc`). Stops at the first failure. `npm run sync:dev` also starts the app.
  Note `npm update` is npm's own builtin and does **not** run this — it has to be `npm run sync`.

Backend (run from `src-tauri/`):
- `cargo check`
- `cargo clippy --all-targets`
- `cargo test`

There is no visual/E2E test harness. To verify a UI change without a real Tauri window, drive
the Vite dev server with Playwright and mock `window.__TAURI_INTERNALS__.invoke` (real IPC isn't
reachable outside the Tauri shell) — screenshot and compare, don't assume.

## Architecture

**Repo layout is the Tauri default**: `src/` (frontend) + `src-tauri/` (backend) at the repo
root — not a custom `frontend/`+`backend/` split. This was a deliberate choice so the whole
Tauri toolchain (CLI, docs, plugin ecosystem) works with zero friction.

**The Taker is embedded in-process, not run as a daemon.** The openswap crate's maker side
(`makerd`/`maker-cli`) uses a daemon + RPC-client split because a maker must keep listening for
swap requests even when no UI is open. The taker has no such requirement — it's a client, not a
server — so `AppState` (`core/src/state.rs`) holds the Takers in-process: one `TakerInstance`
per unlocked wallet, keyed by that wallet's data dir.

**Wallets belong to sessions, not to the process.** A session is a web browser session or, on
desktop, the single `DESKTOP_SESSION`. `AppState.bindings` maps each session to its wallet, and
every wallet command resolves `state.taker_for(session)` — so a second browser sees nothing
until it unlocks a wallet itself. Picking a wallet another session already has open **joins**
it (`join_taker`: the password is checked against the file, never a second `Taker::init`, which
would overwrite the running one's saves and fail its unfinished swaps). A wallet closes when its
last session leaves and no swap is running; the swap thread closes it when the swap settles
(finished or failed — recovery doesn't hold it open, it resumes at the next unlock). Closing
drops the `Taker` on a background thread (its `Drop` can take tens of seconds joining offer
fetches over Tor); reopening that wallet waits for the drop's final save.

**Threading model, and why it matters across `state.rs` + every file in `commands/`:**
- `TakerInstance.taker: Arc<Mutex<Option<Taker>>>` is locked for the *entire* duration of a running
  swap (`start_swap` can take hours) on a dedicated `std::thread`, never the `spawn_blocking`
  pool. Any command that needs the taker (offers, recovery status, prepare/start swap, …) must go
  through `try_lock_taker` (non-blocking) — never a bare `.lock()` — so a running swap makes those
  commands fail fast with `SwapInProgress` instead of freezing the whole UI for hours.
- `TakerInstance.wallet` and `TakerInstance.offer_sync` are handles cloned once at `init_taker`
  time specifically so wallet reads and offerbook syncs never contend with the taker mutex.
  Wallet commands go through this cached handle, never through `taker.taker`.

**Errors cross IPC through one envelope.** Crate errors aren't `Serialize`, so every one is
converted to `AppError` / `ErrorCode` (`error.rs`) before returning from a command. The frontend
switches on `code`, not `message`. **A wrong wallet password can still panic inside the crate
instead of returning `Result`** — every command that loads or restores a wallet must run the crate
call through `spawn_blocking` and classify the result via `from_wallet_join_error`, never generic
`AppError::internal`, or a mistyped password surfaces as an opaque internal error instead of
"wrong password". Upstream PR #993 made wallet *decryption* itself return
`WalletError::Security(SecurityError::Decryption)` rather than panicking, which `From<WalletError>`
maps to `WalletWrongPassword`; both paths have to stay covered.

**Commands are grouped by domain** in `src-tauri/src/commands/`, each registered in `lib.rs`'s
`invoke_handler!`. The frontend mirrors
this exactly in `src/api/commands.ts` + `types.ts` — the single typed boundary to the backend;
components must import from there, never call `invoke()` directly.

**Naming convention, now that both a taker and a maker live in this app** (see
`.Codex/MAKER_INTEGRATION.md`): a command file whose contents are taker-only is named
`taker_<domain>.rs` (`taker_wallet.rs`, `taker_swap.rs`, `taker_reports.rs`, `taker_logs.rs`);
maker-only files are `maker_<domain>.rs` (`maker.rs` for lifecycle, `maker_wallet.rs`,
`maker_reports.rs`, `maker_settings.rs`, `maker_logs.rs`). A file that's genuinely shared or
role-agnostic keeps a plain name — `market.rs`, `setup.rs` — don't prefix those. Cross-cutting
infrastructure (`state.rs`, `error.rs`, `types.rs`, `logging.rs`, `tor.rs`) stays as one file per
concern with a taker section and a maker section side by side, not duplicated per role.

**User-facing vocabulary is `wallet` and `router`, not `taker` and `maker`.** Everything the
user reads — UI text, `AppError` messages, native dialogs — plus every TypeScript identifier,
type, route and file under `src/` uses that vocabulary, and so do the IPC *payloads*: our own
DTOs in `types.rs` carry `router_id`/`routers`/`routers_count`, and Tauri command parameters are
`router_id`. What deliberately keeps the old words is the layer that isn't ours to rename: the
`#[tauri::command]` **names** (`poll_maker`, `init_taker`, …) and their `commands/*.rs` file
names, which stay aligned with the openswap crate's own `Taker`/`Maker` API. So
`commands.ts` reads `invoke("poll_maker")` from a function called `pollRouter` — that mismatch is
intentional, not a missed rename. `MakerSettingsDto.router_id` keeps a `makerId` serde alias
because it also parses maker-dashboard's own `makers.json`.

**Long-running swap execution uses Tauri events, not polling** — `swap://finished` /
`swap://failed`, emitted from the dedicated swap thread — a deliberate departure from the old
Electron app, which polled every 1-2s. `get_swap_progress` is a reconciliation snapshot for
resuming after a reload, not a live feed.

**Every launch starts at the connection gate** (`/connect`), ahead of the role picker, for both
roles: pick a chain backend and wait for Portal's Tor to reach 100% bootstrap. Next only unlocks
once a real chain query has answered. Rust owns whether that gate is satisfied, so a webview
reload lands where the user was.

The chain backend chosen at the gate is per session (`chain_backend.rs` keys it by session id),
and each Taker pins the route it was built against. A session joining a running wallet must be
on the same chain (`WalletNetworkMismatch` otherwise) and uses the running wallet's route.

**Both hosts require the owner password, always** — desktop, web, dev/loopback included. It is
one credential per data root (`core/src/security/owner.rs`), so desktop and a web server on the
same machine share it. Desktop enforces it in Rust: `require_sign_in` in `lib.rs` refuses every
command but the sign-in ones until `auth_login` succeeds; signed-in lasts until the app quits.
A fresh install
is claimed by its first visitor choosing the owner password (password + confirm, no setup
secret); only an Argon2id hash is stored, at `<root>/portal/auth/owner`. Sessions don't idle out:
one with a window open (a live SSE stream) lasts up to 7 days, one with no window open ends 2
hours after its last stream closed. Every way a session ends reaches `end_session`, which
takes it off its wallet.

**Past the gate, routing is gated by session state**, not a router guard: `src/store/session.ts`
(Zustand) tracks whether `init_taker` has succeeded. Not initialized → `SetupPage` (a single page:
scan the wallet folder, pick or create a wallet inline, then a wallet-password → init checklist
with popup-on-failure — see `.Codex/FRONTEND.md` for why this replaced a two-step wizard).
Initialized → the main app.

**Data dirs: `~/.openswap/takers/<name>/` per wallet, `~/.openswap/makers/<id>/` per router,
and everything app-level directly in `~/.openswap/`** (the router registry `makers.json`,
`portal/auth/owner`, `portal/transfers/`, the Tor folders, the app `debug.log`). Paths come from
`core/src/storage.rs` (`openswap_root`, `wallet_data_dir`, `maker_data_dir`, `makers_registry`).
Never share a wallet's dir: the crate keeps one `swap_tracker.cbor` and `offerbook.json` per
data dir, and every `Taker::init` fails the unfinished swaps it finds in that tracker whatever
wallet they belong to. No migration from older layouts. `.portal-open.lock` in each wallet dir
stops a second process on the same files from opening an open wallet. The crate CLIs need
`--data-directory ~/.openswap/takers/<name>` (or `makers/<id>`).

**Logs:** each wallet writes its own `takers/<name>/debug.log`; the app's `~/.openswap/debug.log`
gets the rest. `logging.rs` attributes a line to a wallet when the thread is inside that wallet's
`wallet_scope` (every wallet command sets one), when the line names the wallet, or when exactly
one wallet is open and no router is registered. The crate's background threads name no wallet,
so with several wallets open their lines stay in the app log rather than being guessed.

## Design/UX conventions

The real shipped Electron app's actual look (screenshots, shipped CSS) is the visual source of
truth — **not** `../coinswap-taker` or `../coinswap-maker`, which are unshipped Codex Design
prototypes with a different (orange-primary, sans-serif) palette that was never implemented.
This app is monospace (JetBrains Mono, bundled in `src/assets/fonts`), blue-primary, dark — see
`src/index.css`'s `@theme` block for exact values.

Only build what's explicitly requested or clearly present in the reference app — don't add
toggles, options, or "complete" extra functionality speculatively. Minimize clicks/steps; prefer
inline reveal over navigating to a new page; pre-fill defaults so the user starts from something
that works.

**Connection config is never persisted.** The chain backend and
Tor settings are seeded from constants in Rust on every launch and held in memory for the session
only — an edit is deliberately forgotten, so a node's RPC password is never at rest. Don't add
`localStorage` or a config file for them. The remaining `localStorage` keys are the wallet data
dir, dismissed import prompts, last-sync timestamps, and (web only) the CSRF token, which only
a login hands out — `/session` never returns it, because on Umbrel the session cookie reaches
every app on the host.

## Code quality

No code bloat: no duplicate function declarations across files, no dead/unused code left lying
around (delete it, don't comment it out), no chains of tiny one-call-site helper functions
(inline instead), no separate function for logic used exactly once that reads fine inline.

Comments are terse and WHY-only — never restate what the code does. Don't point comments at
`.Codex/*.md` (internal-only, not shared); once proper in-repo docs exist, point to those instead.
"WHY" means a code-level reason: a hidden constraint, a subtle invariant, a workaround for a
specific bug/API quirk, something that would surprise the next reader. It does NOT mean product
framing, scope decisions, or narrating that something was intentionally left out ("this is a
user-facing app, not a debug console", "we decided not to build X yet") — that belongs in
`.Codex/*.md` or a commit message, never in the code itself. If a comment would still make sense
if you deleted the surrounding code, it doesn't belong there.

UI primitives live in a few files grouped by function, not one file per component — these are
small design-system atoms.
Split a component back out only if it actually grows substantial.
