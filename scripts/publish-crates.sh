#!/usr/bin/env bash
# Publish the product crates to crates.io in dependency order (D-006).
#
# Usage: scripts/publish-crates.sh
#   CARGO_REGISTRY_TOKEN must be set to a crates.io API token.
#
# The release workflow runs this on a version tag after the revision passes
# continuous integration. Publishing a dependent crate strips the `path` from
# its manifest, so the registry version of every dependency must exist first;
# the crates are therefore published leaf-first and each publish retries while
# the crates.io index catches up. `vectr-eval` is maintainer-only and is not
# published (release.md, "Rollout Phases & Feature Flags").
#
# A version already on crates.io is skipped, so a re-run after a partial publish
# completes the release instead of failing on the crates already uploaded.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

[[ -n "${CARGO_REGISTRY_TOKEN:-}" ]] || {
  echo "error: CARGO_REGISTRY_TOKEN is not set" >&2
  exit 1
}

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
[[ -n "$version" ]] || { echo "error: could not read the workspace version" >&2; exit 1; }

# Leaf-first: vectr-core, then the crates that depend on it.
crates=(vectr-core vectr-project vectr-cli vectr-mcp)

max_attempts=10
for crate in "${crates[@]}"; do
  if curl -fsS -A "vectr-release (https://github.com/vanguard-op/vectr)" \
      "https://crates.io/api/v1/crates/${crate}/${version}" >/dev/null 2>&1; then
    echo "==> ${crate} ${version} is already published; skipping"
    continue
  fi

  echo "==> publishing ${crate} ${version}"
  attempt=1
  while true; do
    if cargo publish -p "$crate" --locked; then
      break
    fi
    if (( attempt >= max_attempts )); then
      echo "error: failed to publish ${crate} after ${attempt} attempts" >&2
      exit 1
    fi
    echo "retrying ${crate} (${attempt}/${max_attempts}); waiting for the index"
    sleep 15
    attempt=$(( attempt + 1 ))
  done
done

echo "published: ${crates[*]} ${version}"
