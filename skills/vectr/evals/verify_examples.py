#!/usr/bin/env python3
"""Deterministic checks for the Vectr skill artifacts (FEAT-020).

The skill's value is that a model can follow the authoring guide against the
published schema and the installed tool. These checks pin that contract: the
frontmatter and JSON are well-formed, the guide stands alone as the single
source of the authoring procedure (the scaffold embeds it verbatim, so it must
not depend on the skill's other files), and every worked scene the guide ships,
from the simple mark to the compositionally complex illustration, plus every
reusable definition and the scene template, validate, compile, and export through
the real toolchain.

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
    for command in ("vectr schema", "vectr validate", "vectr compile", "vectr export", "vectr render"):
        if command not in guide:
            raise CheckError(f"the guide does not teach `{command}`")

    # A complex request is built up in verified parts rather than authored in one
    # pass: the guide must teach reusable definitions and the part-scoped render
    # that verifies each one (FEAT-029, FEAT-030, FEAT-031).
    if "definitions/" not in guide:
        raise CheckError("the guide does not teach reusable definitions")
    if "Build a complex graphic up in verified parts" not in guide:
        raise CheckError("the guide does not direct the incremental build-up method")

    # A scene is addressed by its identifier, not by a file path: the document
    # is `scenes/<id>.json` and a command names the id, falling back to the
    # project's default scene (FEAT-016, D-032). The guide must teach that and
    # never show a scene file path as a command argument.
    if "scenes/<id>.json" not in guide or "defaultSceneId" not in guide:
        raise CheckError("the guide does not teach addressing a scene by its identifier")
    for line in guide.splitlines():
        if "scenes/" in line and any(
            command in line for command in ("vectr validate", "vectr compile", "vectr export")
        ):
            raise CheckError(f"the guide passes a scene path as a command argument: {line.strip()!r}")

    # An MCP tool addresses a scene the same way — by identifier, or the
    # project's default — and also accepts an inline draft. The guide must teach
    # the `draft` argument, that a draft is never the default, and that naming
    # `scene` and `draft` together is malformed (FEAT-019, C-005).
    if "draft" not in guide or "E_MALFORMED" not in guide:
        raise CheckError(
            "the guide does not teach the MCP inline `draft` argument and its "
            "mutual exclusion with `scene`"
        )
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
    # A scene is a document with a canvas and elements; a reusable definition is
    # a document with parameters, an origin, and elements but no canvas. Telling
    # them apart lets the toolchain check address each the way the guide does
    # (FEAT-029, FEAT-030).
    scenes = [
        b
        for b in blocks
        if isinstance(b, dict)
        and isinstance(b.get("elements"), list)
        and b["elements"]
        and "canvas" in b
    ]
    definitions = [
        b
        for b in blocks
        if isinstance(b, dict)
        and isinstance(b.get("elements"), list)
        and b["elements"]
        and "parameters" in b
        and "origin" in b
    ]
    if palette is None or stroke is None or not scenes:
        raise CheckError("the authoring guide is missing a palette, stroke, or scene example")
    if not definitions:
        raise CheckError("the authoring guide is missing a reusable definition example")
    notes.append(
        f"guide carries {len(blocks)} json examples, {len(scenes)} worked scenes, "
        f"and {len(definitions)} definitions"
    )

    # The shipped examples are one consistent project: element ids and definition
    # ids share one namespace across the whole project (FEAT-001, FEAT-030), so a
    # reader can drop the guide's scenes and definitions into one project with no
    # collision. Each example validates alone, so a collision is only visible
    # across them; check it here and fail rather than ship examples that cannot
    # coexist.
    seen: dict[str, str] = {}
    for kind, documents in (("scene", scenes), ("definition", definitions)):
        for document in documents:
            owner = f"{kind} `{document['id']}`"
            # A definition's own id shares the namespace; a scene's id does not.
            identifiers = [document["id"]] if kind == "definition" else []
            identifiers += [
                element["id"]
                for element in document["elements"]
                if isinstance(element, dict) and "id" in element
            ]
            for identifier in identifiers:
                if identifier in seen:
                    raise CheckError(
                        f"identifier `{identifier}` is used by {seen[identifier]} and "
                        f"{owner}; the shipped examples are not one consistent project"
                    )
                seen[identifier] = owner
    notes.append(f"the shipped examples share one namespace ({len(seen)} unique identifiers)")
    return notes, palette, stroke, scenes, definitions


def run(command: list[str], cwd: Path) -> subprocess.CompletedProcess:
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        raise CheckError(f"{' '.join(command)} exited {result.returncode}: {detail}")
    return result


def check_toolchain(
    vectr: str,
    template: dict,
    palette: dict,
    stroke: dict,
    scenes: list[dict],
    definitions: list[dict],
) -> list[str]:
    notes = []
    with tempfile.TemporaryDirectory(prefix="vectr-skill-verify-") as raw:
        project = Path(raw)
        (project / "palettes").mkdir()
        (project / "strokes").mkdir()
        (project / "definitions").mkdir()
        (project / "scenes").mkdir()
        (project / "dist").mkdir()
        # The examples are one project: it names the guide's palette as its
        # default so a definition rendered in isolation resolves its tokens
        # (FEAT-030, FEAT-031), and the first worked scene as the default scene.
        (project / "vectr.project.json").write_text(
            json.dumps(
                {
                    "defaultSceneId": scenes[0]["id"],
                    "defaultPaletteId": palette["id"],
                }
            )
        )
        (project / f"palettes/{palette['id']}.json").write_text(json.dumps(palette))
        (project / "strokes/hairline.json").write_text(json.dumps(stroke))
        for definition in definitions:
            (project / f"definitions/{definition['id']}.json").write_text(json.dumps(definition))

        # Every worked scene the guide ships runs the whole loop, so a simple
        # mark and a compositionally complex illustration are both pinned to the
        # real toolchain. A command addresses a scene by its identifier and the
        # document is `scenes/<id>.json` (FEAT-016, D-032).
        for scene in scenes:
            scene_id = scene["id"]
            (project / f"scenes/{scene_id}.json").write_text(json.dumps(scene))
            run([vectr, "validate", scene_id], project)
            run([vectr, "compile", scene_id, "--check"], project)
            run(
                [vectr, "export", scene_id, "--format", "svg", "--out", f"dist/{scene_id}.svg"],
                project,
            )
            if not (project / f"dist/{scene_id}.svg").read_text().lstrip().startswith("<?xml"):
                raise CheckError(f"the exported SVG for {scene_id} is empty")
            run(
                [vectr, "export", scene_id, "--format", "png", "--out", f"dist/{scene_id}.png",
                 "--width", "256", "--height", "256"],
                project,
            )
            png = (project / f"dist/{scene_id}.png").read_bytes()
            if png[:8] != b"\x89PNG\r\n\x1a\n":
                raise CheckError(f"the exported PNG for {scene_id} is not a PNG")
        notes.append(f"{len(scenes)} worked scenes validate, compile, and export SVG and PNG")

        # Every definition the guide ships renders on its own, the verification
        # step the build-up method turns on: the command parses and validates the
        # definition structurally and writes the part's isolated preview
        # (FEAT-029, FEAT-031).
        for definition in definitions:
            definition_id = definition["id"]
            run(
                [vectr, "render", definition_id, "--format", "svg",
                 "--out", f"dist/{definition_id}.svg"],
                project,
            )
            svg = (project / f"dist/{definition_id}.svg").read_text()
            if not svg.lstrip().startswith("<?xml"):
                raise CheckError(f"the part preview for {definition_id} is empty")
        notes.append(f"{len(definitions)} definitions render on their own")

        template_id = template["id"]
        template_path = project / f"scenes/{template_id}.json"
        template_path.write_text(json.dumps(template))
        run([vectr, "validate", template_id], project)
        notes.append("scene template validates")
    return notes


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--skill-dir", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--vectr", default="vectr", help="path to the vectr binary")
    parser.add_argument("--skip-toolchain", action="store_true", help="run only the static checks")
    args = parser.parse_args()

    try:
        notes, palette, stroke, scenes, definitions = check_static(args.skill_dir)
        template = json.loads((args.skill_dir / "assets/scene.template.json").read_text())
        if not args.skip_toolchain:
            vectr = Path(args.vectr)
            if vectr.exists():
                vectr = vectr.resolve()
            elif shutil.which(args.vectr) is None:
                raise CheckError(f"the vectr binary `{args.vectr}` was not found")
            notes += check_toolchain(str(vectr), template, palette, stroke, scenes, definitions)
    except CheckError as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    for note in notes:
        print(f"ok - {note}")
    print("verify_examples: all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
