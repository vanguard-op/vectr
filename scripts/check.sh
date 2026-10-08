#!/usr/bin/env bash
# Canonical verification gate for the Vectr workspace.
#
# This is the single source of truth for "the checks": the same script runs
# locally and in CI (.github/workflows/ci.yml), so a failure is reproducible
# with identical commands. Run it before pushing.
#
# See DELIVERY.md, "Key commands".
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

echo "==> cargo fmt --all -- --check"
cargo fmt --all -- --check

echo "==> cargo clippy --workspace --all-targets --locked -- -D warnings"
cargo clippy --workspace --all-targets --locked -- -D warnings

echo "==> cargo build --workspace --locked"
cargo build --workspace --locked

echo "==> cargo test --workspace --locked"
cargo test --workspace --locked

# The phase-gate acceptance suite is its own crate outside the product
# workspace (`members = ["crates/*"]`), so the workspace test run above does
# not reach it. It is part of the gate: a failure here fails the build
# (NFR-001, NFR-011).
echo "==> cargo test --manifest-path tests/acceptance/Cargo.toml --locked"
cargo test --manifest-path tests/acceptance/Cargo.toml --locked
