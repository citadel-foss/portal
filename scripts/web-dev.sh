#!/usr/bin/env bash
# Runs the web target: Vite serving the UI, Axum serving the API, one Ctrl+C stopping both.
# A shell script rather than a dev dependency — the repo already drives multi-step work this
# way, and one fewer node package is one fewer thing to install before the app will start.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

api_pid=""
ui_pid=""
cleanup() {
  # Kill each child's whole process group: `cargo run` execs the built binary as a grandchild,
  # and killing only the parent leaves it holding port 3000 against the next run.
  for pid in "$api_pid" "$ui_pid"; do
    [ -n "$pid" ] && kill -- "-$pid" 2>/dev/null
  done
  wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

step "Starting the API on http://127.0.0.1:3000"
# No --assets-dir: in development Vite serves the UI and proxies /api here, so the API has no
# static files of its own to hand out.
set -m
cargo run -p portal-web -- --access-profile development-loopback "$@" &
api_pid=$!
set +m

# Wait for it before starting the UI. Otherwise a backend that refused to start — most often
# because another Portal holds the data directory — leaves a UI with nothing behind it, which
# in the browser looks like being signed out rather than like a missing server.
printf '    waiting for the API'
for _ in $(seq 1 60); do
  if curl -sf -o /dev/null http://127.0.0.1:3000/health/live 2>/dev/null; then
    printf ' ready\n'
    break
  fi
  if ! kill -0 "$api_pid" 2>/dev/null; then
    printf '\n\033[1;31m==> The API exited. Its error is above — the UI was not started.\033[0m\n'
    exit 1
  fi
  printf '.'
  sleep 1
done

step "Starting the UI on http://localhost:1430"
set -m
npx vite --mode web &
ui_pid=$!
set +m

printf '\n\033[1mOpen http://localhost:1430\033[0m — Ctrl+C stops both.\n\n'
# `wait -n` needs bash 4; macOS ships 3.2, so poll instead. Either side exiting stops both.
while kill -0 "$api_pid" 2>/dev/null && kill -0 "$ui_pid" 2>/dev/null; do
  sleep 1
done
