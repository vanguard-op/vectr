# Authoring Vectr scenes

This is the procedure for turning a described graphic into a Vectr scene: plain
JSON that compiles to a render model and exports as SVG or PNG. Work through it
in order. It is written to be read alongside the published schema, which is the
source of truth for every type and every allowed value; this guide adds the
workflow and the rules the schema does not spell out.

This guide targets `vectr` 0.1.0 and scene `formatVersion` `0.2`.

## 0. Check the tool and the contract

Confirm the installed tool before authoring:

```sh
vectr --version        # prints: vectr 0.1.0
```

If it prints a different version, stop and report the mismatch, naming both
versions: this guide was written for 0.1.0, and a scene written against a
different contract may not compile. The same applies if `vectr schema` fails
with `E_SCHEMA_VERSION` — the installed tool and the published contract
disagree; report it rather than working around it. Over MCP, read
`serverInfo.version` from the `initialize` response instead of shelling out.

The scene language is at `formatVersion` `"0.2"`; the published schema carries
`x-vectr-formatVersion: "0.2"`.

## 1. What a project holds

A project is a directory with `vectr.project.json` and one folder per document
kind. Documents are referenced by `id`, never by file name, so a file can be
called anything:

| Document | Folder | Referenced by |
|---|---|---|
| Scene | `scenes/` | the file you run |
| Palette | `palettes/` | `scene.paletteId`, and a paint `ref` with `kind: "token"` |
| StrokeProfile | `strokes/` | `element.stroke.profileId` |
| StyleRecipe | `recipes/` | `scene.recipeId`, else the project's `defaultRecipeId` |
| Gradient | `gradients/` | a paint `ref` with `kind: "gradient"` |
| Asset (font) | `assets/` | `element.fontId` on a text element |

Run the tools from inside the project so the root is found. If there is no
project yet, scaffold one with `vectr init [dir]`; it writes the configuration,
a starter scene under `scenes/`, a default recipe, the entity folders
`scenes/ palettes/ strokes/ gradients/ recipes/ assets/ dist/`, and this guide
at the project root. Running it again reports the project as already
initialized and leaves every file untouched.

## 2. The workflow

1. Read the schema for the types you will write (`vectr schema`).
2. Author the scene as JSON.
3. Validate it (`vectr validate`). Correct and re-validate until it passes.
4. Compile with `--check` to confirm the project's references resolve
   (`vectr compile <scene> --check`).
5. Render a PNG preview and look at it against the request
   (`vectr export <scene> --format png`).
6. If it does not match, correct the scene and re-render, then export the final
   SVG and PNG (`vectr export <scene> --format svg`).

Never render before validation passes, and never export from a scene that failed
to compile. A failed step stops the pipeline; there is no partial output.

Prefer the MCP tools when the host exposes them; otherwise use the CLI. Both
produce identical results.

| Step | MCP tool | CLI |
|---|---|---|
| Read the contract | `schema` `{form?, type?}` | `vectr schema [--compact] [--type <name>]` |
| Validate | `validate` `{scene, project?}` | `vectr validate <scene> [--json]` |
| Compile | `compile` `{scene, project?}` | `vectr compile <scene> [--check] [--out <file>]` |
| Render | `render` `{scene, format, out?, width?, height?, density?, background?}` | `vectr export <scene> --format svg\|png [--out <file>] [--width <n>] [--height <n>] [--density <n>] [--background <color\|transparent>]` |

An MCP tool failure is a result with `isError: true` and a body
`{code, message, location, diagnostics}`; read the whole `diagnostics` list, not
just `message`. A CLI failure exits non-zero, prints a diagnostic with its
location, and writes nothing. The `scene` argument to an MCP tool is either the
scene document as JSON text or a path to one.

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
```

Type names are matched case-insensitively; an unknown name is refused with the
closest names listed. The whole contract is large, so for a basic scene read
`Scene`, `Element`, `Geometry`, `Transform`, `Palette`, and `StrokeProfile`
rather than dumping everything into context. Over MCP the same reads are
`schema` with `{"type": "Element"}` or `{"form": "compact"}`.

### Rules the schema does not state

The contract gives properties, types, and allowed values. These rules are
enforced by the engine and are easy to miss:

- **The parser is stricter than the published `required` lists.** Every element
  must carry a `geometry` object and a complete `transform` with all five of
  `translateX`, `translateY`, `rotate`, `scaleX`, `scaleY` — even an identity
  transform. A `group` still needs `geometry`, but `{}` is accepted.
- **Paints are tokens or gradients, never raw colours.** An element's fill or
  stroke paint is `{"kind": "token", "ref": "<tokenName>"}` or
  `{"kind": "gradient", "ref": "<gradientId>"}`. Every colour an element draws
  comes from the palette; the only raw colour fields are the canvas
  `background` and an export `background` override.
- **A stroke pairs a profile and a paint.** `{"profileId": "...", "paint":
  {...}}`; the profile supplies width, cap, and join, the paint supplies colour.
  Both are required, and the profile id must resolve under `strokes/`.
- **`order` is paint order among siblings**, lowest first; later elements draw on
  top. `parentId` builds the tree: a child names its parent's `id`. Group
  decorative children under a `group` element so they move together.
- **Text.** A text element requires `geometry.text` and `geometry.fontSize`.
  `geometry.x` and `geometry.y` anchor the baseline of the first line;
  `align` (`start` | `center` | `end`) positions each line about that anchor;
  `lineHeight` is the baseline-to-baseline distance. Omit `fontId` to use the
  bundled open-licensed sans (Inter, with Noto Sans as glyph fallback);
  `fontId` may appear only on a text element.
- **Recipes** are named `flat`, `line-art`, `geometric`, or `isometric`. A scene
  renders in the recipe it names, else the project's `defaultRecipeId`. A
  stroke-based recipe's `strokeWeight` applies to a stroke whose profile width
  is `0`.
- **Constraints** (`equalSpacing`, `align`, `attach`, `contain`, `snapToGrid`)
  are optional and resolve at compile time; two that cannot both hold fail
  compilation. Leave them out for a simple scene.
- **`formatVersion` must be `"0.2"`.**

## 4. Author the scene

Start from the skeleton below or from the starter scene under `scenes/`, and
edit it. The skeleton of every valid scene:

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

Keep `id`s short and stable: each element's `id` is unique within the scene, and
a child's `parentId` names it. Every element repeats `sceneId` with the scene's
`id`.

Use a named palette rather than hard-coding colour per element, so the whole
graphic restyles by editing one value. `palettes/brand.json`:

```json
{
  "id": "brand",
  "projectId": "project",
  "name": "Habit Brand",
  "tokens": [
    { "name": "accent", "value": "#4f46e5", "description": "Primary accent" },
    { "name": "ink", "value": "#0f172a", "description": "Primary text" },
    { "name": "paper", "value": "#ffffff", "description": "Surfaces" }
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

### Worked example

A logo request — "a rounded badge, a check dot, and the word Habit" — as a
scene. It validates, compiles, and exports as written:

```json
{
  "id": "habit-logo",
  "projectId": "project",
  "name": "Habit logo",
  "formatVersion": "0.2",
  "paletteId": "brand",
  "title": "Habit tracker logo",
  "canvas": { "width": 512, "height": 512, "background": "transparent" },
  "elements": [
    {
      "id": "mark",
      "sceneId": "habit-logo",
      "order": 0,
      "kind": "group",
      "name": "Mark",
      "geometry": {},
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "badge",
      "sceneId": "habit-logo",
      "parentId": "mark",
      "order": 0,
      "kind": "rect",
      "name": "Badge",
      "geometry": { "x": 96, "y": 96, "width": 320, "height": 320, "rx": 72, "ry": 72 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "accent" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "dot",
      "sceneId": "habit-logo",
      "parentId": "mark",
      "order": 1,
      "kind": "ellipse",
      "name": "Check dot",
      "geometry": { "x": 216, "y": 176, "width": 80, "height": 80 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "paper" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "label",
      "sceneId": "habit-logo",
      "parentId": "mark",
      "order": 2,
      "kind": "text",
      "name": "Label",
      "geometry": {
        "text": "Habit",
        "fontSize": 56,
        "x": 256,
        "y": 336,
        "align": "center",
        "lineHeight": 64,
        "letterSpacing": 0
      },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "ink" },
      "opacity": 1,
      "visible": true
    }
  ]
}
```

Geometry notes worth keeping in mind:

- A `rect` and an `ellipse` are placed by the `x`/`y` origin of their bounding
  box and sized by `width`/`height`; the ellipse is inscribed in that box, and
  `rx`/`ry` round a rect's corners.
- A `polygon` and a `line` carry `points` — an array of `[x, y]` pairs.
- A `path` carries `pathData` (SVG path syntax); an `alongPath` element uses
  `pathData` as the guide its children follow.
- Compositions (`boolean`, `offset`, `projection`, `repeat`) act on their child
  elements; give each child `parentId` equal to the composition's `id`.

## 5. Validate, then compile

```sh
vectr validate scenes/logo.json
vectr compile scenes/logo.json --check
```

`validate` checks the scene against the contract and the project's references —
a palette token, stroke profile, gradient, or font that does not resolve is an
error naming the element. `compile --check` confirms the scene resolves to a
render model without writing anything. On success both exit `0` and print
nothing; a broken scene exits non-zero with diagnostics.

Use `vectr validate --json` (or the MCP `validate` result) for a machine-readable
array:

```json
[{"severity":"error","code":"E_SCHEMA","message":"`opacity` must be between 0 and 1","location":{"jsonPath":"/elements/0/opacity"}}]
```

The MCP tools return the same diagnostics in the tool body under `diagnostics`,
with `isError: true` on failure. Exit codes: `0` success, `1` invalid scene, `2`
usage or unreadable input, `3` compilation failure, `4` export dependency
missing, `5` output I/O failure.

## 6. Render and look at the result

Render a preview you can see:

```sh
vectr export scenes/logo.json --format png --out dist/logo.png --width 512 --height 512
```

Then open the PNG and compare it against the request. Reading the image is the
step that catches what a structural check cannot. Export the final deliverables
once the preview matches:

```sh
vectr export scenes/logo.json --format svg --out dist/logo.svg
vectr export scenes/logo.json --format png --out dist/logo.png --width 512 --height 512
```

Exports default to `dist/<scene-stem>.<ext>`; `--out`/`out` chooses another path.
`--density` applies to PNG only; passing it with `--format svg` is a usage error.
`--background <color|transparent>` overrides the canvas background for that
export.

### Inspect and correct

Compare the render to the request on these axes, then fix the scene and
re-render:

| Symptom in the render | Likely cause | Correction |
|---|---|---|
| A shape is cut off at an edge | Geometry extends past the canvas | Move or shrink it inside `canvas.width`/`height`; leave a margin |
| A shape covers one it should sit behind | Wrong `order` | Raise the covering element's `order`, or reorder siblings |
| The mark and its label come apart when moved | Children not grouped | Give them a common `group` parent and set their `parentId` |
| Text sits too high, low, or off the shape | `x`/`y` anchor the first-line baseline, not the box | Adjust `y` to the baseline you want; use `align` for horizontal centring |
| A colour does not match the brand | Element names the wrong token | Point the paint at the right palette token, or fix the token value |
| A stroke is invisible or too heavy | Hairline profile width `0` in a non-stroke recipe, or too large | Name a scaled profile, or a stroke-based recipe whose `strokeWeight` fits |
| Nothing changed after a style edit | The element never referenced the token | Move the element's paint to `{"kind": "token", "ref": ...}` |

Re-render after each correction and look again; stop when the render matches the
request.

## 7. When authoring fails

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
- `E_SCHEMA_TYPE` — a `schema --type` name does not exist; pick one of the
  listed similar names.
- `E_CYCLE` — elements reference each other as parents; break the loop.
- An undefined-token, undefined-stroke, undefined-gradient, or missing-font
  error — the element names something the project does not provide; add it or
  fix the reference.

Correct the scene from the diagnostics and re-validate. **Retry once.** If the
scene still fails after that correction, report the failure together with its
diagnostics and produce no output. Never export from a scene that did not
validate and compile. If the authoring model is unavailable, stop and report
that authoring cannot proceed and produce no output; never emit a placeholder
asset.

## 8. Defaults for an ambiguous request

When the request leaves something open, choose the documented default, state it
in one line, and proceed rather than stopping to ask:

| Open point | Default |
|---|---|
| Canvas size | 512×512 |
| Background | `transparent` |
| Scope of the graphic | One simple mark — a primary shape plus an optional wordmark |
| Shape placement | Centred, with an even margin from each edge |
| Colours | A palette named for the request (`accent`, `ink`, `paper`) so it restyles |
| Text font | The bundled open-licensed sans (omit `fontId`) |
| Recipe | The project's `defaultRecipeId`; set `recipeId` only when the request names a look |
| Icon set | Every icon on the same canvas and style, each exported to its own file |

Keep the first version minimal and valid, render it, then refine toward the
request. A print-ready or highly detailed drawing is a later iteration, not a
reason to stall.

## 9. Licensing and cost

Vectr claims no ownership of the content an authoring model produces. That
content may carry the model provider's own terms; check your provider's terms
before you ship generated graphics. Vectr ships only open-licensed fonts (SIL
OFL); never assume or bundle a commercial font — a user-supplied font is the
user's responsibility.

The model's tokens are billed by the user's provider, not by Vectr. Keep the
loop cheap: read only the schema types you need rather than the whole contract,
and send targeted corrections rather than re-emitting the entire scene when one
element is wrong.
