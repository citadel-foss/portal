#!/usr/bin/env bash
# Keep the git dependency pinned throughout every compatibility build.
set -euo pipefail

if [[ ! "${OPENSWAP_SHA:-}" =~ ^[0-9a-f]{40}$ ]]; then
  echo "::error::OPENSWAP_SHA must be a full lowercase Git commit SHA."
  exit 1
fi

cargo update -p openswap --precise "${OPENSWAP_SHA}"
metadata="${RUNNER_TEMP}/portal-cargo-metadata.json"
cargo metadata --locked --format-version 1 > "${metadata}"
jq -er --arg sha "${OPENSWAP_SHA}" '
  [.packages[] | select(.name == "openswap")] |
  if length == 1 and (.[0].source // "" | endswith("#" + $sha))
  then "Resolved OpenSwap: " + .[0].source
  else error("Cargo did not resolve the selected OpenSwap commit")
  end
' "${metadata}"
