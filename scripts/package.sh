#!/usr/bin/env bash
# Build the shipped binaries for a target triple and emit each one's SHA-256
# digest.
#
# Usage: scripts/package.sh <target-triple> [out-dir]
#   scripts/package.sh x86_64-unknown-linux-gnu dist
#
# Vectr ships two binaries (architecture.md, "Command-Line Interface" and "MCP
# Server"): the `vectr` command-line tool (C-004) and the `vectr-mcp` server
# (C-005). Both are built and packaged together, so a release artifact can be
# reproduced locally with the same command the release workflow runs
# (NFR-025).
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

target="${1:?usage: package.sh <target-triple> [out-dir]}"
out="${2:-dist}"

# The binaries a user installs. vectr-eval is maintainer-only and not shipped
# (release.md, "Rollout Phases & Feature Flags").
binaries=(vectr vectr-mcp)

cargo build --release --locked --target "$target" -p vectr-cli -p vectr-mcp

mkdir -p "$out"

# sha256sum on Linux and Git-Bash, shasum on macOS. The release job also
# produces one combined SHA256SUMS file, so a missing local tool is not fatal.
digest() {
  local name="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    ( cd "$out" && sha256sum "$name" > "$name.sha256" )
  elif command -v shasum >/dev/null 2>&1; then
    ( cd "$out" && shasum -a 256 "$name" > "$name.sha256" )
  else
    echo "warning: no SHA-256 tool found; checksum will be emitted by the release job" >&2
  fi
}

for name in "${binaries[@]}"; do
  src="target/${target}/release/${name}"
  dest="${out}/${name}-${target}"
  if [[ "$target" == *windows* ]]; then
    src="${src}.exe"
    dest="${dest}.exe"
  fi
  cp "$src" "$dest"
  digest "$(basename "$dest")"
  echo "packaged ${dest}"
done
