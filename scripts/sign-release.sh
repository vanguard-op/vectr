#!/usr/bin/env bash
# Sign a release artifact with a maintainer GPG key and export the matching
# public key, so a release is signed and verifiable on any repository
# visibility or billing plan (NFR-025, release.md "Go-Live Checklist").
#
# Usage: scripts/sign-release.sh <checksums-file> [public-key-name]
#   scripts/sign-release.sh dist/SHA256SUMS
#
# Environment:
#   GPG_PRIVATE_KEY  required: the ASCII-armored private signing key, supplied
#                    as a repository secret; never committed (NFR-026).
#   GPG_PASSPHRASE   optional: the key's passphrase, when it has one.
#
# Writes a detached ASCII-armored signature next to the file
# (`<file>.asc`) and the matching public key in the same directory
# (`<public-key-name>.asc`, default `vectr-signing-key.asc`). The same command
# runs locally and in the release workflow, so a signature is reproducible from
# the documented command and a missing key fails the release instead of
# publishing it unsigned.
#
# GitHub artifact attestations are only available for public repositories or
# GitHub Enterprise Cloud organizations, so they cannot sign a release from a
# private repository on a Free plan. A detached GPG signature over the
# published checksums signs the same artifacts everywhere.
set -euo pipefail

file="${1:?usage: sign-release.sh <checksums-file> [public-key-name]}"
key_name="${2:-vectr-signing-key}"

[[ -n "${GPG_PRIVATE_KEY:-}" ]] || {
  echo "error: GPG_PRIVATE_KEY is not set; refusing to publish an unsigned release" >&2
  exit 1
}
[[ -f "$file" ]] || { echo "error: ${file} does not exist" >&2; exit 1; }

command -v gpg >/dev/null 2>&1 || { echo "error: gpg is not installed" >&2; exit 1; }

out_dir="$(dirname "$file")"

# A throwaway keyring so importing the release key never touches a developer's
# own GnuPG home or keyring.
gnupg_home="$(mktemp -d)"
pass_file=""
cleanup() {
  rm -rf "$gnupg_home"
  [[ -n "$pass_file" ]] && rm -f "$pass_file"
}
trap cleanup EXIT
chmod 700 "$gnupg_home"
export GNUPGHOME="$gnupg_home"

printf '%s' "$GPG_PRIVATE_KEY" | gpg --batch --quiet --import

key_id="$(gpg --list-secret-keys --with-colons | awk -F: '/^sec:/ { print $5; exit }')"
[[ -n "$key_id" ]] || { echo "error: GPG_PRIVATE_KEY contains no secret key" >&2; exit 1; }

sign_flags=(--batch --yes --armor --detach-sign --local-user "$key_id")
if [[ -n "${GPG_PASSPHRASE:-}" ]]; then
  # Keep the passphrase off the command line (argv is world-readable).
  pass_file="$(mktemp)"
  chmod 600 "$pass_file"
  printf '%s' "$GPG_PASSPHRASE" > "$pass_file"
  sign_flags+=(--pinentry-mode loopback --passphrase-file "$pass_file")
fi

gpg "${sign_flags[@]}" --output "${file}.asc" "$file"
gpg --batch --armor --export "$key_id" > "${out_dir}/${key_name}.asc"

# Verify our own signature so a broken key or signature fails the release here
# rather than after publication.
gpg --batch --verify "${file}.asc" "$file" >/dev/null 2>&1

echo "signed ${file} -> ${file}.asc (key ${key_id})"
echo "public key -> ${out_dir}/${key_name}.asc"
