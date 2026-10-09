#!/usr/bin/env bash
# Require that the exact revision a release is built from passed continuous
# integration (release.md, "Environments & Promotion": "a release is built from
# the exact revision that passed continuous integration").
#
# Usage: scripts/verify-ci.sh <sha> [repo]
#
# Environment:
#   GH_TOKEN / GITHUB_TOKEN  required: a token with `actions: read`.
#   VERIFY_CI_TIMEOUT        optional: seconds to wait for CI to finish
#                            (default 1800).
#   VERIFY_CI_INTERVAL       optional: seconds between polls (default 15).
#
# The tagged revision's continuous-integration run may still be in flight when
# a release is triggered, so this waits for the CI run on that revision to
# finish and then requires it to have succeeded, instead of failing the moment
# the tag is pushed. A revision whose CI run completes without a success, or
# that never gets one, is refused: there is no promotion straight to users.
set -euo pipefail

sha="${1:?usage: verify-ci.sh <sha> [repo]}"
repo="${2:-${GITHUB_REPOSITORY:-}}"
[[ -n "$repo" ]] || {
  echo "error: no repository given (set GITHUB_REPOSITORY or pass it as the second argument)" >&2
  exit 1
}

[[ -n "${GH_TOKEN:-${GITHUB_TOKEN:-}}" ]] || {
  echo "error: GH_TOKEN/GITHUB_TOKEN is not set" >&2
  exit 1
}
command -v gh >/dev/null 2>&1 || {
  echo "error: the GitHub CLI (gh) is not installed" >&2
  exit 1
}

timeout="${VERIFY_CI_TIMEOUT:-1800}"
interval="${VERIFY_CI_INTERVAL:-15}"
workflow="ci.yml"

deadline=$(( SECONDS + timeout ))
last=""

while :; do
  # A transient API failure must not fail the release, so retry until the
  # deadline rather than aborting on the first error.
  if conclusions="$(gh api \
      "repos/${repo}/actions/workflows/${workflow}/runs?head_sha=${sha}&per_page=100" \
      --jq '.workflow_runs[].conclusion' 2>/dev/null)"; then
    success=0
    pending=0
    failed=0
    while IFS= read -r conclusion; do
      case "$conclusion" in
        success) success=$((success + 1)) ;;
        "" | null) pending=$((pending + 1)) ;;
        *) failed=$((failed + 1)) ;;
      esac
    done <<< "$conclusions"

    last="success=${success} pending=${pending} failed=${failed}"
    echo "CI on ${sha}: ${last}"

    if (( success > 0 )); then
      echo "ok: ${sha} passed continuous integration"
      exit 0
    fi
    if (( pending == 0 && failed > 0 )); then
      echo "error: no successful continuous-integration run for ${sha}; refusing to release" >&2
      exit 1
    fi
  else
    echo "warning: could not read CI runs for ${sha}; retrying" >&2
  fi

  if (( SECONDS >= deadline )); then
    echo "error: timed out after ${timeout}s waiting for CI on ${sha} (${last:-no runs seen}); refusing to release" >&2
    exit 1
  fi
  sleep "$interval"
done
