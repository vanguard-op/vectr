#!/usr/bin/env bash
# The no-rasterizer edge case for the embedded library (FEAT-021, C-002).
#
# PNG export sits behind vectr-core's default `rasterizer` feature. A build
# without it must still validate, compile and export SVG, and must report PNG's
# missing capability as a structured error that names it rather than failing
# silently (FEAT-021, "No rasterizer available for PNG"; nfr.md, "Availability &
# Reliability", "Rasterizer unavailable").
#
# Those paths are guarded by `#[cfg(not(feature = "rasterizer"))]`, so the
# workspace test run — which compiles the rasterizer in — never reaches them.
# This runs the crate's tests with default features off, the only configuration
# in which the missing-capability branch exists. Run locally and in CI; the same
# command, so a failure is reproducible.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

cargo test -p vectr-core --no-default-features --locked
