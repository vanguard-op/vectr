# Authoring Vectr scenes

This is the procedure for turning a described graphic into a Vectr scene: plain
JSON that compiles to a render model and exports as SVG or PNG. Work through it
in order. It is written to be read alongside the published schema, which is the
source of truth for every type and every allowed value. The depth for each step
is in the skill's on-demand references, read only when the step needs them:

| Reference | Read it when |
|---|---|
| `rules.md` | While authoring: the rules the schema does not state. |
| `reusable-parts.md` | When a section is a reusable part: definitions and instances. |
| `depth-and-structure.md` | When a request implies depth or several parts. |
| `inspect-and-correct.md` | After a render: comparing it to the request and correcting it. |
| `defaults.md` | When the request leaves something open. |
| `licensing.md` | Before shipping generated graphics. |

The skill ships worked examples as files under `examples/`, one per document,
kept apart from `assets/` so an example is never mistaken for a copyable asset:
`examples/habit-logo.json` (a simple mark), `examples/alpine-lake.json` (a
compositionally complex illustration), `examples/pine.json` and
`examples/cloud.json` (reusable definitions it places), and
`examples/skyline.json` (one definition placed twice). Read them when a step
points at one.

This guide targets `vectr` 0.1.0-pre.2 and scene `formatVersion` `0.2`.

## 0. Check the tool and the contract

Confirm the installed tool before authoring:

```sh
vectr --version        # prints: vectr 0.1.0-pre.2
```

If it prints a different version, stop and report the mismatch, naming both
versions: this guide was written for 0.1.0-pre.2, and a scene written against a
different contract may not compile. The same applies if `vectr schema` fails
with `E_SCHEMA_VERSION` — the installed tool and the published contract
disagree; report it rather than working around it. Over MCP, read
`serverInfo.version` from the `initialize` response instead of shelling out.

## 1. What a project holds

A project is a directory with `vectr.project.json` and one folder per document
kind. A command names a scene by its **identifier** — the scene's `id` — and its
document is `scenes/<id>.json`, so a scene's file is named for its identifier.
The other documents (palette, stroke profile, recipe, gradient, font) are found
by the `id` they declare, whatever their file is called.

| Document | Folder | Addressed or referenced by |
|---|---|---|
| Scene | `scenes/<id>.json` | the identifier a command names, else the project's `defaultSceneId` |
| Palette | `palettes/` | `scene.paletteId`, and a paint `ref` with `kind: "token"` |
| StrokeProfile | `strokes/` | `element.stroke.profileId` |
| StyleRecipe | `recipes/` | `scene.recipeId`, else the project's `defaultRecipeId` |
| Gradient | `gradients/` | a paint `ref` with `kind: "gradient"` |
| Definition | `definitions/<id>.json` | an `instance` element's `definitionRef` |
| Asset (font) | `assets/` | `element.fontId` on a text element |

A command that omits the scene uses the project's `defaultSceneId`; a command
that names one always operates on that scene alone, whatever the default says,
resolving the project's style assets for it. A project that names no default
reports that no scene was selected rather than choosing among its scenes, and a
default that resolves to no document names the missing scene. Both are errors —
the tools never pick a scene by accident.

An MCP tool addresses a scene by the same rules: `scene` is the identifier, and
omitting both `scene` and `draft` uses the default. It also accepts a scene
document sent inline as `draft` — a JSON object or JSON text — used as a draft
instead of a project scene: it never becomes or reads the default, and its
assets resolve against `project` or the server's project context. A call that
names both `scene` and `draft` is malformed (`E_MALFORMED`), since the two are
mutually exclusive.

Run the tools from inside the project so the root is found. If there is no
project yet, scaffold one with `vectr init [dir]`; it writes the configuration,
a starter scene at `scenes/example.json` that the project names as its default,
a default recipe, the entity folders
`scenes/ palettes/ strokes/ gradients/ recipes/ definitions/ assets/ dist/`, and
a minimal agent guide at the project root. Running it again reports the project
as already initialized and leaves every file untouched.

## 2. The method: sketch the whole, then refine its sections

Every request runs the same method, whatever its complexity. A section is any
unit the work divides into — a group, an instance, or a scene — and a reusable
definition is one kind of section, not the required unit. Complexity changes the
number of turns the loop takes, never the method.

1. **Sketch the whole at low fidelity.** Put the broad sections in their rough
   places and validate the sketch. The sketch fixes the composition before any
   one section is detailed.
2. **Take up one section.** Refine it as one unit, then validate it.
3. **Verify it in isolation.** Render the section on its own and inspect it
   against the request; correct it and re-render until it is right.
4. **Integrate it.** Place the verified section into the whole, and render the
   whole so far. Verify it before taking up the next section.
5. **Repeat** steps 2 to 4 for each section, then validate the whole scene and
   export.

The sections, the kind of each section, the recipe, the palette, and the depth
handling are deduced from the prompt, which describes the picture and prescribes
no structure. When the request leaves the sections open, use the documented
default decomposition: background and sky; the midground masses (land, water,
large structures); the repeating or reused objects (trees, clouds, ripples,
steps); then the foreground detail (reeds, stones, flowers, text). A request
that does not decompose cleanly still gets this decomposition rather than a
stall.

Author and verify a section that depends on another after the part it sits on,
and record that order. Never advance the whole past an unverified section: if a
section fails verification, correct it and re-verify it before it is integrated.
A composition failure names the composition step, not the parts. The whole is
complete when every section is placed and no part dropped, and it compiles
deterministically. If an increment expands past the tool's element limit, the
error names the increment and refuses the whole rather than truncating it;
reduce the section's repetition or nesting, or split it, and re-verify.

A section is verified by structural validation and by rendering it on its own
(`render-part` / `vectr render`); an increment is verified by rendering the whole
so far. When a section is placed more than once, author each part as a reusable
definition once and place it wherever it is needed without re-authoring it; read
`reusable-parts.md`.

Prefer the MCP tools when the host exposes them; otherwise use the CLI. Both
produce identical results. In the CLI, `<scene>` is a scene identifier resolved
among the project's scenes (`scenes/<id>.json`), not a file path; omit it to use
the project's default scene.

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
the part's own bounds or to a requested size; the CLI prints the frame it used.

An MCP tool failure is a result with `isError: true` and a body
`{code, message, location, diagnostics}`; read the whole `diagnostics` list, not
just `message`. A CLI failure exits non-zero, prints a diagnostic with its
location, and writes nothing.

## 3. Read the schema for the types you will write

Do not author from memory. Read the contract, in full or one type at a time:

```sh
vectr schema                        # the whole contract, indented
vectr schema --compact              # the same, minified for machine reading
vectr schema --type Scene           # one type's properties and allowed values
vectr schema --type Element
vectr schema --type Geometry
vectr schema --type Transform
vectr schema --type Paint
vectr schema --type Stroke
vectr schema --type Constraint
vectr schema --type Palette
vectr schema --type StrokeProfile
vectr schema --type StyleRecipe
vectr schema --type Gradient
vectr schema --type Definition
vectr schema --type Binding
```

Type names are matched case-insensitively; an unknown name is refused with the
closest names listed. The whole contract is large, so for a basic scene read
`Scene`, `Element`, `Geometry`, `Transform`, `Palette`, and `StrokeProfile`
rather than dumping everything into context. Over MCP the same reads are
`schema` with `{"type": "Element"}` or `{"form": "compact"}`.

Read `rules.md` while you author: it carries the rules the engine enforces that
the schema does not spell out — the stricter-than-`required` parser, the shared
element/definition id namespace, paints as tokens or gradients, the stroke
profile-plus-paint pair, paint order, text anchoring, recipes, constraints, and
parameter references.

## 4. Sketch the whole at low fidelity

Start from the skeleton below or from the starter scene (`scenes/example.json`,
identifier `example`), and edit it. The skeleton of every valid scene:

```json
{
  "id": "habit-logo",
  "projectId": "project",
  "name": "Habit logo",
  "formatVersion": "0.2",
  "paletteId": "brand",
  "title": "Habit tracker logo",
  "canvas": { "width": 512, "height": 512, "background": "transparent" },
  "elements": []
}
```

Keep `id`s short and stable: each element's `id` is unique across the project,
sharing one namespace with the project's reusable definitions, and a child's
`parentId` names it. Every element repeats `sceneId` with the scene's `id`. Save
the scene as `scenes/<id>.json` — the file is named for the scene's `id` — and
address it by that identifier on the command line.

Use a named palette rather than hard-coding colour per element, so the whole
graphic restyles by editing one value. `palettes/brand.json`:

```json
{
  "id": "brand",
  "projectId": "project",
  "name": "Brand",
  "tokens": [
    { "name": "accent", "value": "#4f46e5", "description": "Primary accent" },
    { "name": "ink", "value": "#0f172a", "description": "Primary text" },
    { "name": "paper", "value": "#ffffff", "description": "Surfaces and snow" },
    { "name": "sky", "value": "#bae6fd", "description": "Sky" },
    { "name": "dawn", "value": "#fde68a", "description": "Dawn glow" },
    { "name": "mist", "value": "#e2e8f0", "description": "Haze" },
    { "name": "sun", "value": "#f59e0b", "description": "Sun" },
    { "name": "ridge", "value": "#475569", "description": "Far ridge" },
    { "name": "rock", "value": "#64748b", "description": "Near peak" },
    { "name": "snow", "value": "#f8fafc", "description": "Snow" },
    { "name": "pine", "value": "#15803d", "description": "Pine foliage" },
    { "name": "pine-far", "value": "#2f6b4f", "description": "Distant foliage" },
    { "name": "pine-dark", "value": "#14532d", "description": "Deep foliage and shore" },
    { "name": "trunk", "value": "#7c2d12", "description": "Trunks and timber" },
    { "name": "water", "value": "#0284c7", "description": "Lake water" },
    { "name": "water-light", "value": "#38bdf8", "description": "Ripples" },
    { "name": "stone", "value": "#94a3b8", "description": "Stepping stones" },
    { "name": "reed", "value": "#4d7c0f", "description": "Reeds" }
  ]
}
```

A stroke profile carries geometry only. `strokes/hairline.json`:

```json
{
  "id": "hairline",
  "projectId": "project",
  "name": "Hairline",
  "width": 0,
  "cap": "round",
  "join": "round"
}
```

Geometry notes worth keeping in mind:

- A `rect` and an `ellipse` are placed by the `x`/`y` origin of their bounding
  box and sized by `width`/`height`; the ellipse is inscribed in that box, and
  `rx`/`ry` round a rect's corners.
- A `polygon` and a `line` carry `points` — an array of `[x, y]` pairs.
- A `path` carries `pathData` (SVG path syntax); an `alongPath` element uses
  `pathData` as the guide its children follow.
- A composition element acts on its children — the elements whose `parentId` is
  the composition's `id`. `group` moves its children together; `repeat` lays
  `count` copies of each child in a row along the element's local x-axis,
  `spacing` apart, the first at the origin; `alongPath` places `count` copies
  evenly along the guide and turns each to the path's direction; `boolean` folds
  the children with `operation` (`union`, `subtract` removes later children from
  the first, `intersect` keeps their common area); `offset` outlines each child
  by `distance` (positive outward); and `projection` maps the children onto
  `axis` (`x`, `y`, or `isometric`). A grid is a row placed inside a group and
  translated into further rows.

## 5. Refine one section at a time

Take up one section and refine it in its own frame. A section may be a group, an
instance, or a scene:

- **A group.** Refine the elements that move together as one section: give them
  a common `group` parent and set their `parentId`. The group's `transform`
  carries the whole section.
- **An instance.** When a section is a reusable part, author it once as a
  definition (`definitions/<id>.json`), verify it on its own, then place it with
  an `instance` element, binding the parameters this use varies. Read
  `reusable-parts.md`.
- **A scene.** A section that is a whole drawing of its own is authored as its
  own scene document, placed nowhere, and exported on its own.

For a request that implies depth or several parts, read
`depth-and-structure.md` before refining: it directs back-to-front paint order
and depth grouping, relative placement by transform, parts joined at shared
anchors, and researching a named subject. Check the render against it in
`inspect-and-correct.md`.

Author each part in its own local frame and place it by a transform; reserve
absolute coordinates for the canvas and the root placement. Name the anchor each
part joins at and give both parts the same value there, so the composition meets
rather than leaving a seam. Inspect the composed result at each anchor for a gap
or an overlap, and correct the anchor rather than nudging one part by eye.

## 6. Worked examples

The skill ships worked examples as files under `examples/`, one per document.
Read the one closest to the request and adapt it, rather than starting from
nothing:

| File | What it shows |
|---|---|
| `examples/habit-logo.json` | A simple mark: a group holding a rounded badge, a check dot, and a wordmark, all painted from palette tokens. |
| `examples/alpine-lake.json` | A compositionally complex illustration: nested groups, `repeat`, `boolean`, `alongPath`, `offset`, `projection`, and a constraint, with depth layers ordered back to front and parts placed by instance transforms. |
| `examples/pine.json` | The reusable definition the complex illustration places, authored with its trunk base at the origin so every placement joins the shore at a shared anchor. |
| `examples/skyline.json` | One definition placed twice with different parameter bindings. |
| `examples/cloud.json` | The reusable definition `skyline.json` places. |

The complex illustration is the reference for the hard end of the range: it
places parts relatively rather than by absolute coordinates, orders depth layers
back to front, and joins parts at shared anchors. A detailed request is not a
reason to simplify, and the language carries complexity through composition
rather than a wider set of shape kinds. Never reduce a detailed request to a single mark or drop the parts it
names; a detailed request is authored in full.

## 7. Validate, then compile

Name the scene by its identifier; the document is `scenes/<id>.json`:

```sh
vectr validate habit-logo
vectr compile habit-logo --check
```

Omitting the identifier uses the project's default scene, so `vectr validate`
alone validates that one. `validate` checks the scene against the contract and
the project's references — a palette token, stroke profile, gradient, or font
that does not resolve is an error naming the element. `compile --check` confirms
the scene resolves to a render model without writing anything. On success both
exit `0` and print nothing; a broken scene exits non-zero with diagnostics.

An identifier no scene document provides, a project that names no default when
the scene is omitted, and a `defaultSceneId` that resolves to no document each
report `E_SCENE` and exit `2`, naming the scene or the missing default.

Use `vectr validate --json habit-logo` (or the MCP `validate` result) for a
machine-readable array:

```json
[{"severity":"error","code":"E_SCHEMA","message":"`opacity` must be between 0 and 1","location":{"jsonPath":"/elements/0/opacity"}}]
```

The MCP tools return the same diagnostics in the tool body under `diagnostics`,
with `isError: true` on failure: call `validate` with `{"scene": "habit-logo"}`,
or `{"draft": {...}}` to validate an inline document. Exit codes: `0` success,
`1` invalid scene, `2` usage or unreadable input, `3` compilation failure, `4`
export dependency missing, `5` output I/O failure.

## 8. Render, inspect, and correct

Render the section you are refining on its own first, then the whole:

```sh
vectr render pine --format png --out dist/pine.png          # the section alone
vectr export habit-logo --format png --out dist/habit-logo.png --width 512 --height 512
```

Open the PNG and compare it against the request. Reading the image is the step
that catches what a structural check cannot. Export the final deliverables once
the preview matches:

```sh
vectr export habit-logo --format svg --out dist/habit-logo.svg
vectr export habit-logo --format png --out dist/habit-logo.png --width 512 --height 512
```

Exports default to `dist/<scene-id>.<ext>`; `--out`/`out` chooses another path.
`--density` applies to PNG only; passing it with `--format svg` is a usage error.
`--background <color|transparent>` overrides the canvas background for that
export.

### Inspect and correct

After each render, read `inspect-and-correct.md`: it maps a symptom in the render
to its likely cause and the correction, and directs a re-render until the render
matches the request. When a section fails verification, correct it and re-verify
it alone before integrating it; when the whole fails, correct the section's use
or placement, not the verified parts.

## 9. When authoring fails

Validation diagnostics name the problem and its location. The usual findings:

- `E_SCHEMA` — a missing required field, a wrong type, or an out-of-range value;
  the `jsonPath` points at it. The most common cause is a partial `transform` or
  a missing `geometry`.
- `E_PARSE` — the document is not valid JSON. Reparse the file.
- `E_FORMAT_VERSION` — `formatVersion` is not `"0.2"`; set it and retry.
- `E_INVALID_COLOR` — a canvas background or export background is not a colour
  SVG supports and not `transparent`.
- `E_PROJECT_ASSET` — a referenced palette, recipe, gradient, or stroke
  document is missing, unreadable, or declares a different `id`; check the
  folder and the `id`.
- `E_SCENE` — the scene identifier the command named has no document at
  `scenes/<id>.json`, the project names no default scene, or its
  `defaultSceneId` resolves to no document; run the command from inside the
  project and check the identifier.
- `E_MALFORMED` — an MCP call named both a `scene` identifier and an inline
  `draft`; the two are mutually exclusive, so name one.
- `E_SCHEMA_TYPE` — a `schema --type` name does not exist; pick one of the
  listed similar names.
- `E_CYCLE` — elements reference each other as parents; break the loop.
- `E_DEFINITION` — an `instance` references a definition that does not resolve;
  check `definitions/<id>.json` and the `definitionRef` id.
- `E_DEFINITION_CYCLE` — two definitions place each other, directly or through a
  chain; break the loop.
- `E_BINDING` — an instance binds a parameter the definition does not declare,
  or a parameter has neither a binding nor a default; fix the binding or give the
  parameter a default.
- `E_PART` — a part identifier resolves to no definition or element in the
  project; check the id and run the command from inside the project.
- `W_UNUSED_DEFINITION` — a definition no scene places renders nothing; place it
  or remove it. It is a warning, not an error.
- An undefined-token, undefined-stroke, undefined-gradient, or missing-font
  error — the element names something the project does not provide; add it or
  fix the reference.

Correct the scene from the diagnostics and re-validate. **Retry once.** If the
scene still fails after that correction, report the failure together with its
diagnostics and produce no output. **Never export** from a scene that did not
validate and compile. If the authoring model is unavailable, stop and report
that authoring cannot proceed and produce no output; never emit a placeholder
asset.
