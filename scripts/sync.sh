#!/usr/bin/env bash
# Brings the whole stack up to date in one pass: the openswap crate, the npm packages, then a
# full verify. Stops at the first failure — an openswap bump regularly needs hand edits on this
# side (renamed methods, changed report fields), so a clean update is not the same as a working
# build, and the verify is the part that tells them apart.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$1"; }

# The openswap commit Cargo.lock currently pins, read out of that package's block alone so a
# rev belonging to some other git dependency can't be picked up by mistake.
pinned_rev() {
  sed -n '/^name = "openswap"$/,/^\[\[package\]\]/p' Cargo.lock |
    sed -n 's/^source = .*#\([0-9a-f]\{40\}\)"$/\1/p' | head -1
}

before=$(pinned_rev)

step "Updating the openswap crate"
cargo update -p openswap
after=$(pinned_rev)
if [ -z "$after" ]; then
  echo "openswap is not pinned to a git revision in Cargo.lock" >&2
  exit 1
elif [ "$before" = "$after" ]; then
  echo "already at ${after:0:12}"
else
  echo "${before:0:12} -> ${after:0:12}"
fi

step "Installing npm packages"
npm install

step "Checking Rust"
cargo clippy --workspace --all-targets

step "Testing Rust"
cargo test --workspace

step "Checking TypeScript"
npm run typecheck

if [ "${1:-}" = "--dev" ]; then
  step "Starting the app"
  exec npm run tauri dev
fi

step "Stack is in sync"
if [ "$before" != "$after" ]; then
  echo "openswap moved to ${after:0:12} — check its log for behaviour changes the compiler can't see."
fi
echo "Run 'npm run tauri dev' to start the app, or 'npm run sync:dev' to do both next time."
