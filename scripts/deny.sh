#!/usr/bin/env bash
# Dependency, advisory and licence gate (NFR-020, NFR-041).
#
# This is the single source of truth for the "security scan" and "license scan"
# named in the continuous-integration gate (release.md, "Environments &
# Promotion"). The same script runs locally, on every change in CI
# (.github/workflows/ci.yml), and against the exact released revision
# (.github/workflows/release.yml), so a finding is reproducible with one
# command instead of being visible only inside a CI action. The rules live in
# deny.toml.
#
# Requires cargo-deny. Install the pinned version with:
#   cargo install cargo-deny --version 0.20.2 --locked
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

if ! cargo deny --version >/dev/null 2>&1; then
  echo "error: cargo-deny is not installed; the security and licence scan cannot run" >&2
  echo "install it with: cargo install cargo-deny --version 0.20.2 --locked" >&2
  exit 1
fi

echo "==> cargo deny check (advisories, bans, licences, sources)"
cargo deny check
