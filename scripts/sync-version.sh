#!/usr/bin/env bash
# Single-source the release version and the scene format version.
#
# Two values are declared once and every other copy is derived from them:
#
#   * the release version, in Cargo.toml's `[workspace.package].version`; and
#   * the scene format version, in
#     crates/vectr-core/src/scene/version.rs (`CURRENT_FORMAT_VERSION`).
#
# The derived copies are the crate dependency pins, the acceptance crate and
# both lockfiles, the README, the agent skill, and both copies of the authoring
# guide. Bumping a release means editing the one source and running this script,
# never editing the copies by hand.
#
# Usage:
#   scripts/sync-version.sh          # rewrite the derived copies to match
#   scripts/sync-version.sh --check  # fail (non-zero) if any copy has drifted
#
# The continuous-integration gate runs `--check` (scripts/check.sh), so a
# version bumped in one place but not another fails the build rather than
# publishing a crate whose manifest requires the previous release (release.md,
# "Environments & Promotion": a release is built from the exact revision that
# passed continuous integration).
#
# The format version also appears in the shipped skill's scene template and its
# evaluation prompts (skills/vectr/assets, skills/vectr/evals), which this
# script does not own; the acceptance suite validates the template against the
# built tool, so a stale copy is caught there.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

check=0
case "${1:-}" in
  --check) check=1 ;;
  "") ;;
  *)
    echo "usage: sync-version.sh [--check]" >&2
    exit 2
    ;;
esac

# The single sources. Cargo.toml's first `version = "..."` is the
# `[workspace.package]` version every crate inherits; version.rs declares the
# one format version the build writes and reads (C-001).
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
[[ -n "$version" ]] || {
  echo "error: could not read the workspace version from Cargo.toml" >&2
  exit 1
}

format_version="$(sed -n 's/^pub const CURRENT_FORMAT_VERSION: &str = "\(.*\)";$/\1/p' \
  crates/vectr-core/src/scene/version.rs | head -n1)"
[[ -n "$format_version" ]] || {
  echo "error: could not read CURRENT_FORMAT_VERSION from crates/vectr-core/src/scene/version.rs" >&2
  exit 1
}

status=0

# apply <file> <sed-expression>... — rewrite <file> in place so it matches the
# single source. Under --check, print the difference and record a failure
# instead. `cmp` keeps an already-correct file untouched, so the script is
# idempotent and a write run shows only the files that actually changed.
apply() {
  local file="$1"
  shift
  local tmp
  tmp="$(mktemp)"
  sed -E "$@" "$file" > "$tmp"
  if cmp -s "$file" "$tmp"; then
    rm -f "$tmp"
    return 0
  fi
  if (( check )); then
    echo "error: ${file} has drifted from the single-sourced version" >&2
    diff -u "$file" "$tmp" >&2 || true
    rm -f "$tmp"
    status=1
    return 0
  fi
  # Write through the original file so its mode survives: `mv` would replace it
  # with mktemp's 0600, stripping the executable bit from a script.
  cat "$tmp" > "$file"
  rm -f "$tmp"
  echo "updated ${file}"
}

# apply_lock <file> <member-names> — set the version of each named workspace
# member's `[[package]]` entry, leaving every third-party version untouched.
apply_lock() {
  local file="$1" members="$2"
  local tmp
  tmp="$(mktemp)"
  awk -v version="$version" -v members="$members" '
    BEGIN { n = split(members, m, " ") }
    /^name = / {
      name = $0
      sub(/^name = "/, "", name)
      sub(/"$/, "", name)
      member = 0
      for (i = 1; i <= n; i++) if (name == m[i]) member = 1
    }
    /^version = / && member { sub(/version = ".*"/, "version = \"" version "\"") }
    { print }
  ' "$file" > "$tmp"
  if cmp -s "$file" "$tmp"; then
    rm -f "$tmp"
    return 0
  fi
  if (( check )); then
    echo "error: ${file} has drifted from the single-sourced version" >&2
    diff -u "$file" "$tmp" >&2 || true
    rm -f "$tmp"
    status=1
    return 0
  fi
  cat "$tmp" > "$file"
  rm -f "$tmp"
  echo "updated ${file}"
}

# --- release version -------------------------------------------------------

# The explicit version on a path dependency keeps it from registering as a `*`
# wildcard (NFR-020) but must track the workspace version, so a published crate
# requires the release it ships with.
p_manifest_core='s|(vectr-core = .*version = ")[^"]*(")|\1'"$version"'\2|'
p_manifest_project='s|(vectr-project = .*version = ")[^"]*(")|\1'"$version"'\2|'

for manifest in crates/vectr-cli/Cargo.toml crates/vectr-mcp/Cargo.toml; do
  apply "$manifest" -e "$p_manifest_core" -e "$p_manifest_project"
done
apply crates/vectr-project/Cargo.toml -e "$p_manifest_core"

# The acceptance crate is its own workspace and names its own version.
apply tests/acceptance/Cargo.toml -e 's|^(version = ")[^"]*(")|\1'"$version"'\2|'

p_readme_install='s|(--version )[^ ]*|\1'"$version"'|g'
p_readme_status='s|(Pre-release, `)[^`]*`|\1'"$version"'`|'
apply README.md -e "$p_readme_install" -e "$p_readme_status"

p_skill_frontmatter='s|^(version: ).*$|\1'"$version"'|'
p_skill_targets='s|(This skill targets Vectr `)[^`]*`|\1'"$version"'`|'
p_skill_prints='s|(which prints `vectr )[^`]*`|\1'"$version"'`|'
apply skills/vectr/SKILL.md \
  -e "$p_skill_frontmatter" -e "$p_skill_targets" -e "$p_skill_prints"

p_guide_targets='s|(This guide targets `vectr` )[^ ]*|\1'"$version"'|'
p_guide_prints='s|(# prints: vectr )[^ ]*|\1'"$version"'|'
p_guide_written='s|(versions: this guide was written for )[^,]*|\1'"$version"'|'
apply skills/vectr/references/authoring-guide.md \
  -e "$p_guide_targets" -e "$p_guide_prints" -e "$p_guide_written"
# The crate carries its own copy of the guide so it packages standalone; a test
# keeps it byte-identical to the skill's reference (FEAT-020).
apply crates/vectr-project/references/authoring-guide.md \
  -e "$p_guide_targets" -e "$p_guide_prints" -e "$p_guide_written"

apply scripts/package-skill.sh \
  -e 's|(#   scripts/package-skill.sh dist )[^ ]*|\1'"$version"'|'

apply_lock Cargo.lock "vectr-cli vectr-core vectr-eval vectr-mcp vectr-project"
apply_lock tests/acceptance/Cargo.lock "vectr-acceptance vectr-core"

# --- scene format version --------------------------------------------------

# The guides and the skill state the format version in prose and in the worked
# scenes; the README's scene example carries it too.
f_json='s|("formatVersion": ")[^"]*(")|\1'"$format_version"'\2|g'
f_fv_quoted='s|(`formatVersion` `")[^"]*(")|\1'"$format_version"'\2|g'
f_xattr='s|(`x-vectr-formatVersion: ")[^"]*(")|\1'"$format_version"'\2|g'
f_mustbe='s|(`formatVersion` must be `")[^"]*(")|\1'"$format_version"'\2|g'
f_isnot='s|(`formatVersion` is not `")[^"]*(")|\1'"$format_version"'\2|g'
f_scene='s|(scene `formatVersion` `)[^`]*`|\1'"$format_version"'`|g'

apply README.md -e "$f_json"
apply skills/vectr/SKILL.md -e "$f_fv_quoted" -e "$f_scene"
apply skills/vectr/references/authoring-guide.md \
  -e "$f_json" -e "$f_fv_quoted" -e "$f_xattr" -e "$f_mustbe" -e "$f_isnot" -e "$f_scene"
apply crates/vectr-project/references/authoring-guide.md \
  -e "$f_json" -e "$f_fv_quoted" -e "$f_xattr" -e "$f_mustbe" -e "$f_isnot" -e "$f_scene"

if (( status != 0 )); then
  echo "version sync check failed: run scripts/sync-version.sh and commit the result" >&2
  exit 1
fi

if (( check )); then
  echo "ok: every derived copy matches vectr ${version} (format ${format_version})"
else
  echo "synced derived copies to vectr ${version} (format ${format_version})"
fi
