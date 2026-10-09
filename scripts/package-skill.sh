#!/usr/bin/env bash
# Package the agent skill as a distributable archive and emit its SHA-256
# digest.
#
# Usage: scripts/package-skill.sh [out-dir] [version]
#   scripts/package-skill.sh dist 0.1.0-pre.1
#
# The skill (SKILL.md plus its references, assets and evals) is how a coding
# agent learns to author scenes (FEAT-020, architecture.md, "Agent Skill &
# Authoring Guide"). It is not a crate, so it ships as a release archive next to
# the binaries; the archive unpacks to a `vectr/` skill directory an agent host
# can load. `version` defaults to the workspace version, so the skill archive
# tracks the tool it was written for and a version mismatch is detectable.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

out="${1:-dist}"
version="${2:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)}"
[[ -n "$version" ]] || { echo "error: could not read the workspace version" >&2; exit 1; }

skill_dir="skills/vectr"
[[ -f "${skill_dir}/SKILL.md" ]] || { echo "error: ${skill_dir}/SKILL.md is missing" >&2; exit 1; }

archive="vectr-skill-${version}.tar.gz"
mkdir -p "$out"

# Reproducible archive: sorted entries and a fixed mtime/ownership, so the same
# skill content yields a byte-identical archive across runs (NFR-010). GNU tar
# supports these flags; BSD tar (macOS) does not, so add them only when
# available.
tar_flags=()
if tar --version 2>/dev/null | grep -q GNU; then
  tar_flags=(--sort=name --mtime='UTC 1980-01-01' --owner=0 --group=0 --numeric-owner)
fi

tar -czf "${out}/${archive}" "${tar_flags[@]}" -C skills vectr

if command -v sha256sum >/dev/null 2>&1; then
  ( cd "$out" && sha256sum "$archive" > "$archive.sha256" )
elif command -v shasum >/dev/null 2>&1; then
  ( cd "$out" && shasum -a 256 "$archive" > "$archive.sha256" )
else
  echo "warning: no SHA-256 tool found; checksum will be emitted by the release job" >&2
fi

echo "packaged ${out}/${archive}"
