#!/usr/bin/env bash
# Build one release binary for a target triple and emit its SHA-256 digest.
#
# Usage: scripts/package.sh <target-triple> <binary-name> [out-dir]
#   scripts/package.sh x86_64-unknown-linux-gnu vectr dist
#
# The release workflow (.github/workflows/release.yml) calls this once per
# platform, so a release artifact can be reproduced locally with the same
# command (NFR-025).
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

target="${1:?usage: package.sh <target-triple> <binary-name> [out-dir]}"
binary="${2:?usage: package.sh <target-triple> <binary-name> [out-dir]}"
out="${3:-dist}"

cargo build --release --locked --target "$target" -p vectr-cli

src="target/${target}/release/${binary}"
dest="${out}/${binary}-${target}"
if [[ "$target" == *windows* ]]; then
  src="${src}.exe"
  dest="${dest}.exe"
fi

mkdir -p "$out"
cp "$src" "$dest"

# sha256sum on Linux and Git-Bash, shasum on macOS. The release job also
# produces one combined SHA256SUMS file, so a missing local tool is not fatal.
name="$(basename "$dest")"
if command -v sha256sum >/dev/null 2>&1; then
  ( cd "$out" && sha256sum "$name" > "$name.sha256" )
elif command -v shasum >/dev/null 2>&1; then
  ( cd "$out" && shasum -a 256 "$name" > "$name.sha256" )
else
  echo "warning: no SHA-256 tool found; checksum will be emitted by the release job" >&2
fi

echo "packaged ${dest}"
