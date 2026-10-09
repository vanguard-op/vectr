---
name: vectr
description: >
  Author Vectr vector graphics from a natural-language request: logos,
  wordmarks, icons and icon sets, badges, diagrams, and illustrations from a
  simple mark to a very complex, many-element composition. Use this skill
  whenever the user asks for a vector graphic, logo, or icon set and Vectr is
  the target toolchain, including when they only describe the picture and never
  say "Vectr": read the scene schema, author the scene, validate it, render a
  preview, and refine the result — building a complex illustration up in
  verified parts — authoring, validating, rendering and exporting, editing,
  restyling through palette tokens, and debugging. Also use
  it to wire the Vectr tools into an agent. Do not use it for photographic or
  bitmap image generation, for hand-writing raw SVG or HTML/CSS, or for graphics
  in another tool's format.
version: 0.1.0-pre.2
license: MIT OR Apache-2.0
compatibility: Requires the Vectr toolchain — the `vectr` CLI or the `vectr-mcp` server — available to the agent.
---

# Vectr

Turn a described graphic into a Vectr scene: a JSON document that compiles to a
render model and exports as SVG or PNG. Read the schema, author the scene,
validate, render, look at the render, and fix what is wrong. The schema and the
tools are the source of truth; this skill is the workflow around them.

**Read `references/authoring-guide.md` before authoring your first scene, and
treat it as the single source of the procedure.** It holds the end-to-end
walkthrough, the worked examples for a simple mark and for a very complex
illustration, the rules the schema does not state, the default set for
ambiguous requests, and the inspect-and-correct loop. This file only orients
you: the workflow, the tool surface, and the version check.

A request's complexity sets the scene's scope: a detailed illustration is
composed in full from its elements, never simplified to a simple mark. A simple
mark is authored in one pass; a complex graphic is built up in verified parts —
each part authored as a reusable definition, verified on its own, then composed
one at a time. The guide carries the method.

For a request that implies depth or several parts, follow the guide's **Author
for depth and structure**: order parts back to front and group them by depth
layer, place parts relatively by transform rather than by absolute coordinates,
join each part at a shared anchor, and — when the host exposes research tools
and the request names a concrete subject — research the subject before
authoring. If no research capability is available, author from your own
knowledge and state that the subject could not be researched rather than
stalling.

## Workflow

Progress:
- [ ] 1. Compare the skill and tool versions (see **Version compatibility**).
- [ ] 2. Scaffold a project if there is none: `vectr init`.
- [ ] 3. Read the schema for the types you will write.
- [ ] 4. Author the scene document as `scenes/<id>.json`.
- [ ] 5. Validate. Correct and re-validate until it passes.
- [ ] 6. Compile with `--check` to confirm references resolve.
- [ ] 7. Render a PNG preview and look at it against the request.
- [ ] 8. Correct the scene and re-render if it does not match, then export the final SVG and PNG.

For a complex graphic, steps 4 to 8 become the build-up method: decompose the
request into named parts, author each part as a reusable definition
(`definitions/<id>.json`), verify it alone with `render-part` / `vectr render`,
then compose the verified parts into the scene one at a time, rendering the
composition after each addition. Follow the guide's **Build a complex graphic up
in verified parts**.

Never render before validation passes, and never export from a scene that failed
to compile. A failed step stops the pipeline; there is no partial output.

## Tool surface

Prefer the MCP tools when the host exposes them; otherwise use the CLI. Both
produce identical results.

| Step | MCP tool | CLI |
|---|---|---|
| Read the contract | `schema` `{form?, type?}` | `vectr schema [--compact] [--type <name>]` |
| Validate | `validate` `{scene?, draft?, project?}` | `vectr validate <scene> [--json]` |
| Compile | `compile` `{scene?, draft?, project?}` | `vectr compile <scene> [--check] [--out <file>]` |
| Render | `render` `{scene?, draft?, project?, format, out?, width?, height?, density?, background?}` | `vectr export <scene> --format svg\|png [--out <file>] [--width <n>] [--height <n>] [--density <n>] [--background <color\|transparent>]` |
| Render a part | `render-part` `{part, project?, format, out?, width?, height?, density?, background?}` | `vectr render <part> --format svg\|png [--out <file>] [--width <n>] [--height <n>] [--density <n>] [--background <color\|transparent>]` |
| Scaffold | — | `vectr init [dir]` |

`render-part` and `vectr render` preview one part on its own — a reusable
definition or a named element subtree, addressed by its identifier — framed to
the part's own bounds or to a requested size; the CLI prints the frame it used.

An MCP tool failure is a result with `isError: true` and a body
`{code, message, location, diagnostics}`; read the whole `diagnostics` list, not
just `message`. A CLI failure exits non-zero, prints a diagnostic with its
location, and writes nothing: `1` invalid scene, `2` usage or unreadable input,
`3` compilation failure, `4` export dependency missing, `5` output I/O failure.

Both the CLI's `<scene>` argument and an MCP tool's `scene` argument are a scene
identifier resolved among the project's scenes (`scenes/<id>.json`), not a file
path; omit it to use the project's `defaultSceneId`. An MCP tool also accepts an
inline scene document through `draft` (a JSON object or JSON text), used as a
draft instead of a project scene: it never becomes or reads the default, and its
assets resolve against `project` or the server's project context. Naming both
`scene` and `draft` in one call is malformed; a project that names no default
reports that no scene was selected rather than choosing among its scenes.

## Output contract

Deliver one Vectr scene document that conforms to the published schema, with
`formatVersion` `"0.2"`. Validate it before compiling and render only on
success. Exports default to `dist/<scene-id>.<ext>`; `--out`/`out` chooses
another path. For a complex graphic, deliver the scene together with the
reusable definitions it places (`definitions/<id>.json`); the scene plus its
definitions is the deliverable.

## When authoring fails

Validation and compilation return diagnostics that name each problem and its
location — severity, code, message, and a JSON path or element id. Read them,
correct the scene, and re-validate. **Retry authoring once**: if the scene still
fails after that correction, report the failure with its diagnostics and produce
no output; never export a partial or guessed result. If the authoring model
itself is unavailable, report that authoring cannot proceed and produce no
output. The guide lists the common findings and their fixes.

## Version compatibility

This skill targets Vectr `0.1.0-pre.2` and scene `formatVersion` `0.2`. Before
authoring, compare the installed tool's version with the skill's: run
`vectr --version` (which prints `vectr 0.1.0-pre.2`), or read `serverInfo.version` from
the MCP `initialize` response. If they differ, report the mismatch and name both
versions instead of authoring against a tool the skill was not written for. If
`vectr schema` reports `E_SCHEMA_VERSION`, the published contract and the
installed tool disagree; report that the same way.

## References

| File | Read it when |
|---|---|
| `references/authoring-guide.md` | Before authoring your first scene; the end-to-end walkthrough, worked examples (including the build-up method for a complex graphic), the depth and structure directives, the default set, and the inspect-and-correct loop. The single source of the procedure. |
| `assets/scene.template.json` | As the starting point for a new scene. |
