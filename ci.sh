#!/usr/bin/env bash
# Everything CI should run. Offline except for the advisory database and pnpm
# install; the live Tor tests are intentionally excluded.
set -euo pipefail

cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

if command -v cargo-audit >/dev/null 2>&1; then
  cargo audit
else
  echo "ci: cargo-audit not installed, skipping" >&2
fi

if command -v cargo-deny >/dev/null 2>&1; then
  cargo deny check
else
  echo "ci: cargo-deny not installed, skipping" >&2
fi

(
  cd ui
  if [ ! -d node_modules ]; then
    pnpm install --frozen-lockfile
  fi
  pnpm typecheck
  pnpm build
)
