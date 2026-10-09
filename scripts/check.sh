#!/usr/bin/env bash
# Canonical verification gate for the Vectr workspace.
#
# This is the single source of truth for "the checks": the same script runs
# locally and in CI (.github/workflows/ci.yml), so a failure is reproducible
# with identical commands. Run it before pushing.
#
# It covers every item the continuous-integration gate names (release.md,
# "Environments & Promotion"): the tests, the determinism check, the security
# scan and the license scan, plus the version-sync check. See DELIVERY.md,
# "Key commands".
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# The release version and the scene format version are declared once and derived
# everywhere else; a copy bumped in one place but not another would publish a
# crate whose manifest requires the previous release, or ship a guide that names
# the wrong contract. This is a pure text check, so it runs before the build.
echo "==> scripts/sync-version.sh --check"
scripts/sync-version.sh --check

echo "==> cargo fmt --all -- --check"
cargo fmt --all -- --check

echo "==> cargo clippy --workspace --all-targets --locked -- -D warnings"
cargo clippy --workspace --all-targets --locked -- -D warnings

# The security and license scan is fast and independent of the build, so it
# runs early: a dependency advisory or a license violation fails the gate
# before the long build rather than after it (NFR-020, NFR-041).
echo "==> scripts/deny.sh"
scripts/deny.sh

echo "==> cargo build --workspace --locked"
cargo build --workspace --locked

echo "==> cargo test --workspace --locked"
cargo test --workspace --locked

# The workspace tests compile the rasterizer in, so the no-rasterizer edge case
# (FEAT-021) never executes there: a build without it must still validate,
# compile and export SVG, and must report PNG's missing capability rather than
# failing silently (nfr.md, "Availability & Reliability"). This reruns the core
# crate's tests with the feature off, which is where those paths exist.
echo "==> scripts/no-rasterizer.sh"
scripts/no-rasterizer.sh

# The phase-gate acceptance suite is its own crate outside the product
# workspace (`members = ["crates/*"]`), so the workspace fmt, clippy and test
# runs above do not reach it. It is part of the gate: its formatting, lints and
# tests all fail the build here (NFR-001, NFR-011).
echo "==> cargo fmt --manifest-path tests/acceptance/Cargo.toml --all -- --check"
cargo fmt --manifest-path tests/acceptance/Cargo.toml --all -- --check

echo "==> cargo clippy --manifest-path tests/acceptance/Cargo.toml --all-targets --locked -- -D warnings"
cargo clippy --manifest-path tests/acceptance/Cargo.toml --all-targets --locked -- -D warnings

echo "==> cargo test --manifest-path tests/acceptance/Cargo.toml --locked"
cargo test --manifest-path tests/acceptance/Cargo.toml --locked

# NFR-010: identical input and seed produce byte-identical output across
# repeated runs. The acceptance suite checks this in-process; this runs the
# shipped binary end to end and compares the output hashes.
echo "==> scripts/determinism.sh"
scripts/determinism.sh
