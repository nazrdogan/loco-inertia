#!/usr/bin/env bash
# Everything that must pass before pushing: formatting, lints, Rust tests (unit, integration,
# form fuzzing), generated TypeScript types in sync, and the browser end-to-end tests.
#
#   scripts/check.sh            # all checks
#   SKIP_E2E=1 scripts/check.sh # without the browser tests (no Chrome / Node needed)
set -euo pipefail
cd "$(dirname "$0")/.."

step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

step "cargo fmt --check"
cargo fmt --all --check

step "cargo clippy"
cargo clippy --workspace --all-targets --all-features --quiet -- -D warnings

step "cargo test"
cargo test --workspace --all-features --quiet

# `cargo test` exports the props types (ts-rs); a diff means they were not committed.
step "generated TypeScript types are up to date"
if ! git diff --exit-code --stat -- examples/demo/frontend/src/types; then
  echo "Props types changed: commit examples/demo/frontend/src/types." >&2
  exit 1
fi

if [[ "${SKIP_E2E:-}" == "1" ]]; then
  step "end-to-end tests skipped (SKIP_E2E=1)"
  exit 0
fi

step "frontend build (typecheck, client, SSR)"
cd examples/demo/frontend
[[ -d node_modules ]] || npm ci
npm run build --silent

step "end-to-end tests (Playwright, Chrome)"
# Start fresh servers from this checkout instead of reusing a running dev server.
E2E_FRESH=1 npx playwright test --reporter=line
