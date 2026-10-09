# Rules the schema does not state

Read this while authoring the scene. The contract gives properties, types, and
allowed values; these rules are enforced by the engine and are easy to miss. It
also carries the project layout and the starter snippets a new scene begins
from.

## The project and how a scene is addressed

A project is a directory with `vectr.project.json` and one folder per document
kind. A command names a scene by its **identifier** — the scene's `id` — and its
document is `scenes/<id>.json`, so a scene's file is named for its identifier.
The other documents are found by the `id` they declare, whatever their file is
called.

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
that names one operates on that scene alone, whatever the default says,
resolving the project's style assets for it. A project that names no default
reports that no scene was selected rather than choosing among its scenes, and a
default that resolves to no document names the missing scene. Both are errors —
the tools never pick a scene by accident.

An MCP tool addresses a scene by the same rules: `scene` is the identifier, and
omitting both `scene` and `draft` uses the default. It also accepts a scene
document sent inline as `draft` — a JSON object or JSON text — used instead of a
project scene: it never becomes or reads the default, and its assets resolve
against `project` or the server's project context. A call that names both
`scene` and `draft` is malformed (`E_MALFORMED`), since the two are mutually
exclusive.

If there is no project yet, scaffold one with `vectr init [dir]`; it writes the
configuration, a starter scene at `scenes/example.json` that the project names
as its default, a default recipe, the entity folders
`scenes/ palettes/ strokes/ gradients/ recipes/ definitions/ assets/ dist/`, and
a minimal agent guide at the project root. Running it again reports the project
as already initialized and leaves every file untouched.

## Read the schema

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

## Starter snippets

The skeleton of every valid scene:

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

## The language rules

- **The parser is stricter than the published `required` lists.** Every element
  must carry a `geometry` object and a complete `transform` with all five of
  `translateX`, `translateY`, `rotate`, `scaleX`, `scaleY` — even an identity
  transform. A `group` still needs `geometry`, but `{}` is accepted.
- **An element `id` is unique across the whole project, not only within one
  scene.** Element ids and definition ids share one namespace, so two scenes
  cannot both name an element `sky`, and an element id cannot equal a definition
  id. Give each a distinct id — one scene's background is `backdrop`, another's
  `sky` — and prefix a part's ids when a name would repeat.
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
  compilation. Leave them out when nothing needs them.
- **A definition is a project-scoped part, not a scene.** It has no `canvas`; it
  declares `parameters` and an `origin` and holds its own `elements`. Its
  elements carry `definitionId` where a scene's carry `sceneId` — exactly one of
  the two, in the same element shape. A definition renders only where a scene
  places it with an `instance` element.
- **A parameter reference is `{"param": "<name>"}`.** It stands in for a literal
  in a parameter-capable field (a number, `geometry.text` or `pathData`,
  `visible`, or a whole `fill` or `stroke.paint`). It is valid only inside the
  definition that declares the parameter, and its type must match the field. A
  `token` parameter takes the place of a whole paint, so `{"param": "colour"}` is
  a fill, not a token reference.
- **An instance places a definition.**
  `{"kind": "instance", "definitionRef": "<id>", "bindings": [{"name":
  "<param>", "value": ...}]}`. It carries its own `transform` and `opacity` and
  no geometry of its own; the definition's elements render under it. A parameter
  left unbound takes its declared `default`.
- **`formatVersion` must be the version the installed tool targets.** The
  entry point names it, and `vectr schema` reports it as `x-vectr-formatVersion`;
  a value the tool does not support is `E_FORMAT_VERSION`.

## Geometry notes

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
