//! The authoring guide `vectr init` writes into a new project (FEAT-020).
//!
//! Coding agents load a project's `AGENTS.md` without bespoke setup, so the
//! scaffold writes the workflow there: read the schema, author the scene,
//! validate, compile, render, inspect, and refine. The guide is self-contained —
//! a model that has only this file and the published schema can produce a valid
//! scene — and it names the tool and format versions it was written for, so a
//! version mismatch is reported rather than worked around.
//!
//! The guide is embedded in the binary rather than read from disk: the scaffold
//! must produce the same file whether it runs from a checkout or an installed
//! crate, and it must not depend on the skill package being present.

use vectr_core::scene::CURRENT_FORMAT_VERSION;

/// The file the scaffold writes the guide to, at the project root.
pub const AUTHORING_GUIDE_FILE: &str = "AGENTS.md";

/// The version stamp the guide declares, filled with the running tool's own
/// package version.
const VERSION_PLACEHOLDER: &str = "__VECTR_VERSION__";

/// The format-version stamp the guide declares.
const FORMAT_PLACEHOLDER: &str = "__FORMAT_VERSION__";

/// The authoring guide, stamped with the versions of the tool writing it.
///
/// Interpolating the versions keeps a scaffolded project's guide honest: it
/// always names the tool that wrote it, and an agent that finds a different
/// `vectr --version` reports the mismatch instead of authoring against a guide
/// built for another release (FEAT-020).
pub fn authoring_guide() -> String {
    GUIDE
        .replace(VERSION_PLACEHOLDER, env!("CARGO_PKG_VERSION"))
        .replace(FORMAT_PLACEHOLDER, CURRENT_FORMAT_VERSION)
}

/// The guide text, with the version stamps left for [`authoring_guide`].
const GUIDE: &str = r##"# Authoring Vectr scenes

This directory is a Vectr project. This guide is the procedure for turning a
described graphic into a Vectr scene and exporting it as SVG or PNG. Work
through it in order; it is written to be read alongside the published schema.

This project was scaffolded by Vectr __VECTR_VERSION__ and targets scene
`formatVersion` "__FORMAT_VERSION__". Before authoring, confirm the installed
tool matches:

```sh
vectr --version        # prints: vectr __VECTR_VERSION__
```

If it prints a different version, stop and report the mismatch: this guide was
written for __VECTR_VERSION__. The same applies if `vectr schema` fails with
`E_SCHEMA_VERSION` — the installed tool and this guide disagree, and a scene
written against the wrong contract will not compile.

## The workflow

1. Read the schema for the types you will write.
2. Author the scene as JSON.
3. Validate it. Correct and re-validate until it passes.
4. Compile with `--check` to confirm the project's references resolve.
5. Render a PNG preview and look at it against the request.
6. If it does not match, correct the scene and re-render, then export the final
   SVG and PNG.

Never render before validation passes, and never export from a scene that failed
to compile. A failed step stops the pipeline; there is no partial output.

## What a project holds

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

Run the tools from inside the project so the root is found.

## Read the schema for the types you will write

Do not author from memory. Read the contract, in full or one type at a time:

```sh
vectr schema                    # the whole contract, indented
vectr schema --compact          # the same, minified for machine reading
vectr schema --type Scene       # one type's properties and allowed values
vectr schema --type Element
vectr schema --type Geometry
vectr schema --type Transform
vectr schema --type Paint
vectr schema --type Stroke
vectr schema --type Palette
vectr schema --type StrokeProfile
```

Type names are matched case-insensitively, and an unknown name is refused with
the closest ones listed. The whole contract is large, so read `Scene`,
`Element`, `Geometry`, `Transform`, `Palette`, and `StrokeProfile` for a basic
scene rather than dumping everything into context.

### Rules the schema does not state

The contract gives properties, types, and allowed values; these rules are
enforced by the engine and are easy to miss:

- **The parser is stricter than the published `required` lists.** Every element
  carries a `geometry` object and a complete `transform` with all five of
  `translateX`, `translateY`, `rotate`, `scaleX`, `scaleY`, even an identity
  transform. A `group` still needs `geometry`, but `{}` is accepted.
- **Paints are tokens or gradients, never raw colours.** A fill or stroke paint
  is `{"kind": "token", "ref": "<tokenName>"}` or `{"kind": "gradient", "ref":
  "<gradientId>"}`. Every colour an element draws comes from the palette; the
  only raw colour fields are the canvas `background` and an export background
  override.
- **A stroke pairs a profile and a paint.** `{"profileId": "...", "paint":
  {...}}`; the profile carries width, cap, and join, the paint carries colour.
  Both are required, and the profile id must resolve under `strokes/`.
- **`order` is paint order among siblings**, lowest first; later elements draw
  on top. `parentId` names an element's parent. Group decorative children under
  a `group` element so they move together.
- **Text** requires `geometry.text` and `geometry.fontSize`. `geometry.x` and
  `geometry.y` anchor the baseline of the first line; `align` (`start` |
  `center` | `end`) positions lines about that anchor; `lineHeight` is the
  baseline-to-baseline distance. Omit `fontId` to use the bundled open-licensed
  sans; `fontId` may appear only on a text element.
- **Recipes** are named `flat`, `line-art`, `geometric`, or `isometric`. A
  stroke-based recipe's `strokeWeight` applies to a stroke whose profile width
  is `0`.
- **`formatVersion` must be "__FORMAT_VERSION__".**

## Author the scene

The skeleton of every valid scene, with its palette:

```json
{
  "id": "habit-logo",
  "projectId": "project",
  "name": "Habit logo",
  "formatVersion": "__FORMAT_VERSION__",
  "paletteId": "brand",
  "title": "Habit tracker logo",
  "canvas": { "width": 512, "height": 512, "background": "transparent" },
  "elements": []
}
```

Use a named palette rather than hard-coding colour per element, so the whole
graphic restyles by editing one value. Save it as `palettes/brand.json`:

```json
{
  "id": "brand",
  "projectId": "project",
  "name": "Habit Brand",
  "tokens": [
    { "name": "accent", "value": "#4f46e5" },
    { "name": "ink", "value": "#0f172a" },
    { "name": "paper", "value": "#ffffff" }
  ]
}
```

Keep element `id`s short and stable; each is unique within the scene and a
child's `parentId` names it. Every element repeats `sceneId` with the scene's
`id`.

### A worked example

A logo request — "a rounded badge, a check dot, and the word Habit" — as
`scenes/logo.json`. It validates and compiles as written:

```json
{
  "id": "habit-logo",
  "projectId": "project",
  "name": "Habit logo",
  "formatVersion": "__FORMAT_VERSION__",
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
- A `polygon` and a `line` carry `points`, an array of `[x, y]` pairs.
- A `path` carries `pathData` (SVG path syntax).
- Compositions (`boolean`, `offset`, `projection`, `repeat`) act on their child
  elements; give each child `parentId` equal to the composition's `id`.

## Validate, then compile

```sh
vectr validate scenes/logo.json
vectr compile scenes/logo.json --check
```

`validate` checks the scene against the contract and the project's references: a
palette token, stroke profile, gradient, or font that does not resolve is an
error naming the element. `compile --check` confirms the scene resolves to a
render model without writing anything. On success both exit `0` and print
nothing; a broken scene exits non-zero with diagnostics.

Use `vectr validate --json` for a machine-readable array:

```json
[{"severity":"error","code":"E_SCHEMA","message":"`opacity` must be between 0 and 1","location":{"jsonPath":"/elements/0/opacity"}}]
```

Exit codes: `0` success, `1` invalid scene, `2` usage or unreadable input, `3`
compilation failure, `4` export dependency missing, `5` output I/O failure.

## Render and look at the result

Render a preview you can see:

```sh
vectr export scenes/logo.json --format png --out dist/logo.png --width 512 --height 512
```

Open the PNG and compare it against the request. Reading the image is the step
that catches what a structural check cannot. Export the deliverables once the
preview matches:

```sh
vectr export scenes/logo.json --format svg --out dist/logo.svg
vectr export scenes/logo.json --format png --out dist/logo.png --width 512 --height 512
```

`--density` applies to PNG only; passing it with `--format svg` is a usage
error. `--background <color|transparent>` overrides the canvas background for
that export.

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
| A stroke is invisible or too heavy | Profile width `0` in a non-stroke recipe, or too large | Name a scaled profile, or a stroke-based recipe whose `strokeWeight` fits |
| Nothing changed after a style edit | The element never referenced the token | Move the element's paint to `{"kind": "token", "ref": ...}` |

Re-render after each correction and look again; stop when the render matches the
request.

## When authoring fails

Validation diagnostics name the problem and its location. The usual findings:

- `E_SCHEMA` — a missing required field, a wrong type, or an out-of-range value;
  the `jsonPath` points at it. The most common cause is a partial `transform` or
  a missing `geometry`.
- `E_PARSE` — the document is not valid JSON.
- `E_FORMAT_VERSION` — `formatVersion` is not "__FORMAT_VERSION__".
- `E_INVALID_COLOR` — a canvas background is not a colour SVG supports and not
  `transparent`.
- `E_PROJECT_ASSET` — a referenced palette, recipe, gradient, or stroke document
  is missing, unreadable, or declares a different `id`.
- `E_CYCLE` — elements reference each other as parents; break the loop.
- An undefined-token, undefined-stroke, undefined-gradient, or missing-font
  error — the element names something the project does not provide; add it or
  fix the reference.

Correct the scene from the diagnostics and re-validate. **Retry once.** If the
scene still fails after that correction, report the failure with its diagnostics
and produce no output. Never export from a scene that did not validate and
compile. If the authoring model is unavailable, report that authoring cannot
proceed and produce no output; never emit a placeholder asset.

## Defaults for an ambiguous request

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

## Licensing and cost

Vectr claims no ownership of the content an authoring model produces. That
content may carry the model provider's own terms; check your provider's terms
before you ship generated graphics. Vectr ships only open-licensed fonts (SIL
OFL); never bundle a commercial font. The model's tokens are billed by your
provider, not by Vectr; read only the schema types you need rather than the
whole contract, and send targeted corrections rather than re-emitting the whole
scene when one element is wrong.
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guide_is_stamped_with_the_running_versions() {
        let guide = authoring_guide();
        assert!(!guide.contains("__"), "a placeholder survived: {guide}");
        assert!(
            guide.contains(env!("CARGO_PKG_VERSION")),
            "the guide names the tool version"
        );
        assert!(
            guide.contains(CURRENT_FORMAT_VERSION),
            "the guide names the target format version"
        );
    }

    #[test]
    fn the_guide_carries_the_full_authoring_workflow() {
        let guide = authoring_guide();
        for step in [
            "vectr schema",
            "vectr validate",
            "vectr compile",
            "vectr export",
        ] {
            assert!(guide.contains(step), "the guide teaches `{step}`");
        }
        // The inspect step and the retry-once fallback are the behaviour
        // FEAT-020's acceptance criteria and edge cases turn on.
        assert!(
            guide.contains("Inspect and correct"),
            "the guide teaches inspection"
        );
        assert!(guide.contains("Retry once"), "the guide bounds the retry");
        assert!(guide.contains("Defaults for an ambiguous request"));
        assert!(guide.contains("Licensing"));
    }
}
