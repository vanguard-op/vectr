#!/usr/bin/env bash
# Determinism check for the shipped CLI (NFR-010; release.md, "Environments &
# Promotion").
#
# The continuous-integration gate names a determinism check: identical input and
# seed must produce byte-identical output across repeated runs. The acceptance
# suite compares the render model in-process; this runs the shipped binary end
# to end over the fixtures corpus and compares the output hashes, so a
# non-deterministic export is caught the way a user would see it (NFR-010).
#
# Run locally and in CI; the same command, so a failure is reproducible.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# The fixtures project is the smoke corpus this check runs over.
project="fixtures"
[[ -f "${project}/vectr.project.json" ]] || {
  echo "error: ${project}/vectr.project.json is missing; cannot run the determinism check" >&2
  exit 1
}

# The canonical gate builds the workspace before this runs; build here too so
# the script is runnable on its own.
cargo build --locked -p vectr-cli >/dev/null

bin="$PWD/target/debug/vectr"
# On Windows the binary carries the `.exe` suffix; Git Bash finds it either way,
# but resolve the path explicitly so the executable test is accurate.
if [[ -x "${bin}.exe" ]]; then
  bin="${bin}.exe"
fi
[[ -x "$bin" ]] || { echo "error: ${bin} was not built" >&2; exit 1; }

# sha256sum on Linux and Git-Bash, shasum on macOS.
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    echo "error: no SHA-256 tool found" >&2
    exit 1
  fi
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Export each format twice in separate processes: repeated runs of the same
# input must be byte-identical (NFR-010). The project is found from the working
# directory, so each run happens inside the fixtures project.
for run in 1 2; do
  ( cd "$project" && "$bin" export --format svg --out "${work}/${run}.svg" ) >/dev/null
  ( cd "$project" && "$bin" export --format png --out "${work}/${run}.png" ) >/dev/null
done

status=0
for artifact in svg png; do
  first="$(sha256 "${work}/1.${artifact}")"
  second="$(sha256 "${work}/2.${artifact}")"
  if [[ "$first" == "$second" ]]; then
    echo "ok: ${artifact} output is byte-identical across runs (${first})"
  else
    echo "error: ${artifact} output differs between runs; export is not deterministic" >&2
    echo "  run 1: ${first}" >&2
    echo "  run 2: ${second}" >&2
    status=1
  fi
done

(( status == 0 )) || { echo "determinism check failed" >&2; exit 1; }
