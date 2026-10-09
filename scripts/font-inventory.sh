#!/usr/bin/env bash
# Font licence inventory (NFR-040; release.md, "Go-Live Checklist").
#
# Confirms that only open-licensed fonts ship: every font binary in
# assets/fonts has a sibling licence text that declares the SIL Open Font
# License, and every licence text bundled beside a font is itself an OFL text.
# A commercial font or a non-OFL licence fails the gate, so a release never
# redistributes a font it may not (NFR-040).
#
# Run locally and in the release preflight; the same command gates a release.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

fonts_dir="assets/fonts"
[[ -d "$fonts_dir" ]] || { echo "error: ${fonts_dir} is missing" >&2; exit 1; }

shopt -s nullglob
fonts=("$fonts_dir"/*.ttf "$fonts_dir"/*.otf "$fonts_dir"/*.woff "$fonts_dir"/*.woff2)
if (( ${#fonts[@]} == 0 )); then
  echo "error: no font binaries found in ${fonts_dir}" >&2
  exit 1
fi

status=0
for font in "${fonts[@]}"; do
  base="$(basename "${font%.*}")"
  licence=""
  for candidate in "$fonts_dir/${base}"*.txt; do
    if grep -qi "SIL Open Font License" "$candidate"; then
      licence="$candidate"
      break
    fi
  done
  if [[ -z "$licence" ]]; then
    echo "error: $(basename "$font") has no SIL OFL licence text next to it" >&2
    status=1
    continue
  fi
  echo "ok: $(basename "$font") -> $(basename "$licence") (SIL OFL)"
done

# Every licence text bundled beside the fonts must itself be an OFL text.
for licence in "$fonts_dir"/*.txt; do
  if ! grep -qi "SIL Open Font License" "$licence"; then
    echo "error: $(basename "$licence") is not a SIL Open Font License text" >&2
    status=1
  fi
done

(( status == 0 )) || { echo "font inventory failed" >&2; exit 1; }
echo "font inventory: every bundled font is SIL OFL"
