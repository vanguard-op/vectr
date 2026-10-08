#!/usr/bin/env python3
"""Deterministic checks for the Vectr skill artifacts (FEAT-020).

The skill's value is that a model can follow the authoring guide against the
published schema and the installed tool. These checks pin that contract: the
frontmatter and JSON are well-formed, the guide stands alone as the single
source of the authoring procedure (the scaffold embeds it verbatim, so it must
not depend on the skill's other files), and the guide's own worked example — and
the scene template — validate, compile, and export through the real toolchain.

Nothing here grades authored scenes; measuring cross-model authoring quality is
the evaluation harness's job. This only proves the shipped examples still work.

Usage:
    verify_examples.py [--skill-dir DIR] [--vectr PATH] [--skip-toolchain]

Exit codes: 0 all checks pass, 1 a check failed, 2 bad usage or tooling missing.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

DESCRIPTION_LIMIT = 1024
SEMVER = re.compile(r"^\d+\.\d+\.\d+(?:[-+].*)?$")


class CheckError(Exception):
    pass


def parse_frontmatter(text: str) -> dict[str, str]:
    if not text.startswith("---"):
        raise CheckError("SKILL.md does not start with YAML frontmatter")
    end = text.find("\n---", 3)
    if end == -1:
        raise CheckError("SKILL.md frontmatter is not closed")
    header = text[3:end]
    fields: dict[str, str] = {}
    key = None
    for line in header.splitlines():
        if not line.strip():
            continue
        match = re.match(r"^([A-Za-z0-9_-]+):\s*(.*)$", line)
        if match:
            key, value = match.group(1), match.group(2).strip()
            fields[key] = "" if value in (">", "|") else value
        elif key and line.startswith((" ", "\t")):
            fields[key] = (fields.get(key, "") + " " + line.strip()).strip()
    return fields


def fenced_json_blocks(text: str) -> list[dict]:
    blocks = []
    for match in re.finditer(r"```json\s*\n(.*?)```", text, re.DOTALL):
        try:
            blocks.append(json.loads(match.group(1)))
        except json.JSONDecodeError as error:
            raise CheckError(f"a fenced json block does not parse: {error}")
    return blocks


def check_static(skill_dir: Path) -> list[str]:
    notes = []
    frontmatter = parse_frontmatter((skill_dir / "SKILL.md").read_text())
    name = frontmatter.get("name")
    if name != skill_dir.name:
        raise CheckError(f"frontmatter name {name!r} does not match folder {skill_dir.name!r}")
    description = frontmatter.get("description", "")
    if not description:
        raise CheckError("frontmatter is missing a description")
    if len(description) > DESCRIPTION_LIMIT:
        raise CheckError(f"description is {len(description)} chars; limit {DESCRIPTION_LIMIT}")
    version = frontmatter.get("version")
    if version and not SEMVER.match(version):
        raise CheckError(f"version {version!r} is not semver")
    notes.append(f"frontmatter ok (description {len(description)} chars, version {version})")

    for json_file in ["assets/scene.template.json", "evals/evals.json"]:
        json.loads((skill_dir / json_file).read_text())
        notes.append(f"{json_file} is valid JSON")
    evals = json.loads((skill_dir / "evals/evals.json").read_text())
    if evals.get("skill_name") != skill_dir.name:
        raise CheckError("evals.json skill_name does not match the skill folder")

    guide = (skill_dir / "references/authoring-guide.md").read_text()

    # The guide is the single source of the authoring procedure: it is what the
    # scaffold embeds into a project, so it must carry the whole workflow, name
    # the versions it targets, and stand alone — a scaffolded project has none
    # of the skill's other files.
    for command in ("vectr schema", "vectr validate", "vectr compile", "vectr export"):
        if command not in guide:
            raise CheckError(f"the guide does not teach `{command}`")
    for section in (
        "Inspect and correct",
        "Retry once",
        "Never export",
        "Defaults for an ambiguous request",
        "Licensing",
    ):
        if section not in guide:
            raise CheckError(f"the guide is missing `{section}`")
    if version and version not in guide:
        raise CheckError(f"the guide does not name the tool version {version!r}")
    for skill_only in ("SKILL.md", "assets/scene.template.json", "evals/"):
        if skill_only in guide:
            raise CheckError(
                f"the guide references the skill-only {skill_only!r}; it must stand "
                "alone so the scaffold can embed it verbatim"
            )

    blocks = fenced_json_blocks(guide)
    palette = next((b for b in blocks if "tokens" in b), None)
    stroke = next((b for b in blocks if {"cap", "join", "width"} <= set(b)), None)
    scenes = [b for b in blocks if "elements" in b]
    scene = max(scenes, key=lambda b: len(b["elements"])) if scenes else None
    if palette is None or stroke is None or scene is None:
        raise CheckError("the authoring guide is missing a palette, stroke, or scene example")
    notes.append(f"guide carries {len(blocks)} json examples")
    return notes, palette, stroke, scene


def run(command: list[str], cwd: Path) -> subprocess.CompletedProcess:
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        raise CheckError(f"{' '.join(command)} exited {result.returncode}: {detail}")
    return result


def check_toolchain(vectr: str, template: dict, palette: dict, stroke: dict, scene: dict) -> list[str]:
    notes = []
    with tempfile.TemporaryDirectory(prefix="vectr-skill-verify-") as raw:
        project = Path(raw)
        (project / "palettes").mkdir()
        (project / "strokes").mkdir()
        (project / "scenes").mkdir()
        (project / "dist").mkdir()
        (project / "vectr.project.json").write_text("{}")
        (project / "palettes/brand.json").write_text(json.dumps(palette))
        (project / "strokes/hairline.json").write_text(json.dumps(stroke))
        scene_path = project / "scenes/logo.json"
        scene_path.write_text(json.dumps(scene))

        run([vectr, "validate", "scenes/logo.json"], project)
        notes.append("guide example validates")
        run([vectr, "compile", "scenes/logo.json", "--check"], project)
        notes.append("guide example compiles")
        run([vectr, "export", "scenes/logo.json", "--format", "svg", "--out", "dist/logo.svg"], project)
        if not (project / "dist/logo.svg").read_text().lstrip().startswith("<?xml"):
            raise CheckError("the exported SVG is empty")
        run(
            [vectr, "export", "scenes/logo.json", "--format", "png", "--out", "dist/logo.png",
             "--width", "256", "--height", "256"],
            project,
        )
        png = (project / "dist/logo.png").read_bytes()
        if png[:8] != b"\x89PNG\r\n\x1a\n":
            raise CheckError("the exported PNG is not a PNG")
        notes.append("guide example exports SVG and PNG")

        template_path = project / "scenes/template.json"
        template_path.write_text(json.dumps(template))
        run([vectr, "validate", "scenes/template.json"], project)
        notes.append("scene template validates")
    return notes


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--skill-dir", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--vectr", default="vectr", help="path to the vectr binary")
    parser.add_argument("--skip-toolchain", action="store_true", help="run only the static checks")
    args = parser.parse_args()

    try:
        notes, palette, stroke, scene = check_static(args.skill_dir)
        template = json.loads((args.skill_dir / "assets/scene.template.json").read_text())
        if not args.skip_toolchain:
            vectr = Path(args.vectr)
            if vectr.exists():
                vectr = vectr.resolve()
            elif shutil.which(args.vectr) is None:
                raise CheckError(f"the vectr binary `{args.vectr}` was not found")
            notes += check_toolchain(str(vectr), template, palette, stroke, scene)
    except CheckError as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    for note in notes:
        print(f"ok - {note}")
    print("verify_examples: all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
