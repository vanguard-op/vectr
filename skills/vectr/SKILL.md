---
name: vectr
description: >
  Author Vectr vector graphics from a natural-language request: logos,
  wordmarks, icons and icon sets, badges, diagrams, and illustrations from a
  simple mark to a very complex, many-element composition. Use this skill
  whenever the user asks for a vector graphic, logo, or icon set and Vectr is
  the target toolchain, including when they only describe the picture and never
  say "Vectr": read the scene schema, author the scene, validate it, render a
  preview, and refine the result by the one method — sketch the whole, then
  refine its sections one at a time — for authoring, validating, rendering and
  exporting, editing, restyling through palette tokens, and debugging. Also use
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

This entry point is always read; the depth for each step is in the on-demand
references below, loaded only when the step needs them.

## The one method

Every request runs the same method, whatever its complexity: **sketch the whole
at low fidelity, then refine its sections one at a time, integrating and
verifying each before the next.** A section is any unit the work divides into —
a group, an instance, or a scene — deduced from the prompt, which describes the
picture and prescribes no structure. A reusable definition is one kind of
section, not the required unit. Complexity changes the number of turns, never
the method. A detailed request is authored in full and is never simplified to a
simple mark.

A section is verified by structural validation and by rendering it on its own
(`render-part` / `vectr render`); an increment by rendering the whole so far.
Export only when the whole passes. If no research capability is available for a
named subject, author from your own knowledge and state that the subject could
not be researched rather than stalling.

## Workflow

Progress:
- [ ] 1. Compare the skill and tool versions (see **Version compatibility**).
- [ ] 2. Scaffold a project if there is none: `vectr init`.
- [ ] 3. Read the schema for the types you will write.
- [ ] 4. Sketch the whole at low fidelity: the broad sections in their rough
      places, and validate the sketch.
- [ ] 5. Take up one section, refine it, and validate it.
- [ ] 6. Render the section on its own and correct it until it matches.
- [ ] 7. Integrate the verified section, render the whole so far, and verify it
      before the next section. Repeat 5–7 for each section.
- [ ] 8. Validate the whole scene, then export the final SVG and PNG.

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
| Render a section | `render-part` `{part, project?, format, out?, width?, height?, density?, background?}` | `vectr render <part> --format svg\|png [--out <file>] [--width <n>] [--height <n>] [--density <n>] [--background <color\|transparent>]` |
| Scaffold | — | `vectr init [dir]` |

`render-part` and `vectr render` preview one section on its own — a reusable
definition or a named element subtree, addressed by its identifier — framed to
its own bounds or to a requested size; the CLI prints the frame it used.

An MCP tool failure is a result with `isError: true` and a body
`{code, message, location, diagnostics}`; read the whole `diagnostics` list. A
CLI failure exits non-zero, prints a diagnostic with its location, and writes
nothing: `1` invalid scene, `2` usage or unreadable input, `3` compilation
failure, `4` export dependency missing, `5` output I/O failure.

A scene argument — the CLI's `<scene>` or an MCP tool's `scene` — is an
identifier resolved among the project's scenes (`scenes/<id>.json`), not a file
path; omit it to use the project's `defaultSceneId`. An MCP tool also accepts an
inline `draft` document instead of a project scene: it never becomes or reads
the default, and naming both `scene` and `draft` is malformed.

## Output contract

Deliver one Vectr scene document that conforms to the published schema, with
`formatVersion` `"0.2"`, together with every reusable definition it places. The
scene is complete when every section is integrated and verified; nothing is
dropped. Validate it before compiling and render only on success. Exports
default to `dist/<scene-id>.<ext>`; `--out`/`out` chooses another path.

## When authoring fails

Validation and compilation return diagnostics that name each problem and its
location. Read them, correct the scene, and re-validate. **Retry authoring
once**: if the scene still fails, report the failure with its diagnostics and
produce no output; never export a partial or guessed result. If the authoring
model is unavailable, report that authoring cannot proceed and produce no
output. `references/failures.md` lists the common findings and their fixes.

## Version compatibility

This skill targets Vectr `0.1.0-pre.2` and scene `formatVersion` `0.2`. Before
authoring, compare the installed tool's version with the skill's: run
`vectr --version` (which prints `vectr 0.1.0-pre.2`), or read `serverInfo.version`
from the MCP `initialize` response. If they differ, report the mismatch and name
both versions instead of authoring against a tool the skill was not written for.
If `vectr schema` reports `E_SCHEMA_VERSION`, the published contract and the
installed tool disagree; report that the same way.

## References

Read a reference only when its step needs it.

| File | Read it when |
|---|---|
| `references/rules.md` | While authoring: the project layout, the language rules, and starter snippets. |
| `references/reusable-parts.md` | A section is a reusable part: definitions and instances. |
| `references/depth-and-structure.md` | The request implies depth or several parts. |
| `references/inspect-and-correct.md` | After a render: compare it to the request and correct it. |
| `references/defaults.md` | The request leaves something open. |
| `references/failures.md` | Validation or compilation reports a finding. |
| `references/licensing.md` | Before shipping generated graphics. |
| `assets/scene.template.json` | As the starting point for a new scene. |

Worked examples, one file each, from a simple mark to a complex illustration.
Read the one closest to the request and adapt it:

| Example | What it shows |
|---|---|
| `examples/habit-logo.md` | A simple mark: badge, check, and wordmark in one group, painted from tokens. |
| `examples/alpine-lake.md` | A compositionally complex illustration: nested groups, every composition primitive, depth ordered back to front, parts placed by instance transforms. |
| `examples/pine.md` | The reusable definition the illustration places, its origin at the trunk base. |
| `examples/skyline.md` | One definition placed twice with different bindings. |
| `examples/cloud.md` | The reusable definition `skyline.md` places. |
