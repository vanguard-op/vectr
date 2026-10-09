# Authoring Vectr scenes

This is the procedure for turning a described graphic into a Vectr scene: plain
JSON that compiles to a render model and exports as SVG or PNG. Work through it
in order. It is written to be read alongside the published schema, which is the
source of truth for every type and every allowed value; this guide adds the
workflow and the rules the schema does not spell out.

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

The scene language is at `formatVersion` `"0.2"`; the published schema carries
`x-vectr-formatVersion: "0.2"`.

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
names both `scene` and `draft` is malformed, since the two are mutually
exclusive.

Run the tools from inside the project so the root is found. If there is no
project yet, scaffold one with `vectr init [dir]`; it writes the configuration,
a starter scene at `scenes/example.json` that the project names as its default,
a default recipe, the entity folders
`scenes/ palettes/ strokes/ gradients/ recipes/ definitions/ assets/ dist/`, and
this guide at the project root. Running it again reports the project as already
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

A simple mark follows those six steps once. A request that names several distinct
objects, repeats an object, or spans background and foreground layers is a
complex graphic: build it up in verified parts instead of authoring it in one
pass — decompose it into named parts, author each part as a reusable definition,
verify it on its own, then compose the verified parts one at a time, verifying
the scene after each addition. **Build a complex graphic up in verified parts**
(section 6) is that method.

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
| Render a part | `render-part` `{part, project?, format, out?, width?, height?, density?, background?}` | `vectr render <part> --format svg\|png [--out <file>] [--width <n>] [--height <n>] [--density <n>] [--background <color\|transparent>]` |
| Scaffold | — | `vectr init [dir]` |

`render-part` and `vectr render` preview one part on its own — a reusable
definition or a named element subtree, addressed by its identifier — framed to
the part's own bounds or to a requested size; the CLI prints the frame it used.

An MCP tool addresses a scene by the same rules as the CLI. `scene` is the
identifier; omitting both `scene` and `draft` uses the project's default, and a
project that names no default is an error rather than a choice among its scenes.
An MCP tool also accepts a scene document sent inline as `draft` — a JSON object
or JSON text — used instead of a project scene: it never becomes or reads the
default, and its assets resolve against `project` or the server's project
context. Naming both `scene` and `draft` in one call is malformed. A project
scene renders to `dist/<id>.<ext>` by default; a draft has no identifier, so it
renders to `dist/scene.<ext>` unless `out` names a path.

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

### Rules the schema does not state

The contract gives properties, types, and allowed values. These rules are
enforced by the engine and are easy to miss:

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
  compilation. Leave them out for a simple scene.
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
- **`formatVersion` must be `"0.2"`.**

## 4. Author the scene

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
address it by that identifier on the command line (the skeleton below becomes
`scenes/habit-logo.json`, addressed as `habit-logo`).

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

### Author for depth and structure

The hard end of the range is a coherent object, not a set of disjoint shapes.
Four directives apply whenever a request implies depth or several parts; each
uses only the primitives the language already carries. Read them before a
complex illustration, and check the render against them in **Inspect and
correct**.

**Convey depth through structure, not photorealism.** Depth here is the
structural depth the language carries — projection, paint order, grouping,
relative scale, and the palette's atmospheric tokens — never vanishing-point or
photorealistic rendering, which is outside the product's scope.

- Paint order is depth. Order siblings back to front: the farthest mass takes
  the lowest `order` and a nearer part a higher one, so a nearer part occludes a
  farther one. Put each depth layer in its own `group` and give the groups
  increasing `order` — sky, far ridge, midground, foreground — so a layer moves
  as one.
- Diminish scale with distance. A nearer part is larger than a farther one;
  carry the difference in each part's `transform` (`scaleX`/`scaleY` and
  position) rather than drawing every part the same size.
- Recede distant masses with the palette's atmospheric tokens. A far ridge
  takes a hazier token than a near one (`mist` over `ridge` over `rock`), and a
  lower `opacity` fades a mass further, so distance reads through colour rather
  than detail.
- Project a constructed object. An object shown in three dimensions goes onto
  the `projection` element's `axis` (`x`, `y`, or `isometric`) or into the
  `isometric` recipe, rather than stacking unprojected flat shapes; project only
  through an explicit `projection` element.

If the chosen recipe cannot carry the requested depth, carry the structural
depth it does support and do not fake photorealism.

**Place parts relatively, not by absolute coordinates.** Author each part in its
own local frame and place it by a transform; reserve absolute coordinates for
the canvas and the root placement.

- Author a reusable part with its `origin` at the anchor it joins at — a base, a
  pivot, a shared edge — and draw its elements around that origin (negative
  coordinates are fine).
- Place the part with an `instance` `transform` (`translateX`/`translateY`,
  `rotate`, `scaleX`/`scaleY`) rather than editing its geometry for each use.
- Position a child relative to its parent or composition: a child's coordinates
  are in its parent group's local frame, and inside `repeat`, `alongPath`, or
  `projection` they are composition-relative — the first copy sits at the
  element's origin and each further copy is placed by the composition.
- State sibling relationships as constraints (`equalSpacing`, `align`,
  `attach`) instead of hand-computing positions, so the relationship survives an
  edit.

**Decompose along real parts and join them at shared anchors.** Decompose an
object along what it is actually made of, not into arbitrary slices, and author
each part so it joins its neighbour at a shared anchor.

- Name the anchor each part joins at and record it with the decomposition: a
  pine's trunk base meets the shore and its canopy meets the trunk top; a boat's
  hull meets the waterline; a snow cap meets the peak's apex.
- Give the shared anchor a value both parts agree on — the same point in the
  parent frame, or the definition's `origin` — so the parts meet rather than
  leaving a seam.
- When a part has no natural anchor to share, add an explicit anchor point or an
  `attach` constraint rather than leaving a visible gap.
- Inspect the composed result at each anchor for a gap or an overlap, and
  correct the anchor rather than nudging one part by eye.

**Research a named subject before authoring.** When the request names a concrete
subject and the host exposes research tools, research its real proportions,
anatomy, and distinguishing features first, and use what you find to choose the
decomposition.

- Research before decomposing: the real parts and their proportions decide the
  decomposition and the anchors, not a guess.
- If the host exposes no research capability, or the lookup returns nothing
  usable, author from your own knowledge, state in one line that the subject
  could not be researched, and proceed rather than stalling or refusing. A
  research failure never fails the scene.
- A subject that is unknown or fictional is authored from the description rather
  than stalling for research.

### Worked example: a complex illustration

A detailed request is not a reason to simplify, and the language carries
complexity through composition rather than a wider set of shape kinds: build
the illustration from its parts by grouping them and layering the composition
elements over them. Never reduce a detailed request to a single mark or drop
the parts it names. A request this size is built with the build-up method below:
the repeated and reused pieces — the pine, the cloud, the ripple, the step — are
the parts to author as definitions and verify on their own before they are
composed. The scene below is that composition, with the pine authored once as a
reusable definition.

"An alpine lake at dawn — mountains with snow, a pine forest, a lake with reeds
and a trail of stepping stones, and a low sun" becomes the scene below. It
exercises nested groups; a cloud row and two pine rows (`repeat`); a snow cap
(`boolean` intersect) and a glacier (`boolean` subtract); a lake (`boolean`
union) with ripples and reeds placed along guides (`alongPath`); a sun halo
(`offset`); an isometric boardwalk (`projection`); and a planting row laid out
by an `equalSpacing` constraint. It validates, compiles, and exports as written
against the palette above; read the schema for a composition kind only when you
are about to use it.

It also shows the directives above in practice. The pine is authored in its own
local frame with its trunk base at the origin, then placed by an `instance`
transform on the shore line — the shared anchor every pine joins at — rather
than by absolute coordinates per tree. The far row scales the same part to 0.55
and binds its `foliage` parameter to the hazier `pine-far` token, so distance
reads through relative scale and an atmospheric colour; the near row takes the
default. Each depth layer is its own group with increasing `order`, so a nearer
part occludes a farther one. `definitions/pine.json`:

```json
{
  "id": "pine",
  "projectId": "project",
  "name": "Pine",
  "parameters": [
    { "name": "foliage", "type": "token", "default": "pine" }
  ],
  "origin": { "x": 0, "y": 0 },
  "elements": [
    {
      "id": "pine-trunk",
      "definitionId": "pine",
      "order": 0,
      "kind": "rect",
      "name": "Trunk",
      "geometry": { "x": -4, "y": -46, "width": 8, "height": 46 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "trunk" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "pine-canopy-low",
      "definitionId": "pine",
      "order": 1,
      "kind": "polygon",
      "name": "Lower canopy",
      "geometry": { "points": [[0, -140], [-36, -20], [36, -20]] },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "param": "foliage" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "pine-canopy-high",
      "definitionId": "pine",
      "order": 2,
      "kind": "polygon",
      "name": "Upper canopy",
      "geometry": { "points": [[0, -96], [-42, 0], [42, 0]] },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "pine-dark" },
      "opacity": 1,
      "visible": true
    }
  ]
}
```

The scene, `scenes/alpine-lake.json`:

```json
{
  "id": "alpine-lake",
  "projectId": "project",
  "name": "Alpine lake at dawn",
  "formatVersion": "0.2",
  "paletteId": "brand",
  "title": "Alpine lake at dawn",
  "canvas": {"width": 800, "height": 600, "background": "transparent"},
  "elements": [
    {"id": "sky", "sceneId": "alpine-lake", "order": 0, "kind": "rect", "geometry": {"x": 0, "y": 0, "width": 800, "height": 600}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Sky", "fill": {"kind": "token", "ref": "sky"}},
    {"id": "sun", "sceneId": "alpine-lake", "order": 1, "kind": "group", "geometry": {}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Sun"},
    {"id": "halo", "sceneId": "alpine-lake", "order": 0, "kind": "offset", "geometry": {"distance": 30}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 0.55, "visible": true, "parentId": "sun", "name": "Halo", "fill": {"kind": "token", "ref": "dawn"}},
    {"id": "halo-src", "sceneId": "alpine-lake", "order": 0, "kind": "ellipse", "geometry": {"x": 600, "y": 60, "width": 110, "height": 110}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "halo", "name": "Halo source"},
    {"id": "sun-disc", "sceneId": "alpine-lake", "order": 1, "kind": "ellipse", "geometry": {"x": 600, "y": 60, "width": 110, "height": 110}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "sun", "name": "Disc", "fill": {"kind": "token", "ref": "sun"}},
    {"id": "clouds", "sceneId": "alpine-lake", "order": 2, "kind": "repeat", "geometry": {"count": 2, "spacing": 250}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Clouds"},
    {"id": "cloud-puff", "sceneId": "alpine-lake", "order": 0, "kind": "group", "geometry": {}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "clouds", "name": "Cloud"},
    {"id": "cloud-a", "sceneId": "alpine-lake", "order": 0, "kind": "ellipse", "geometry": {"x": 90, "y": 120, "width": 150, "height": 50}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "cloud-puff", "fill": {"kind": "token", "ref": "paper"}},
    {"id": "cloud-b", "sceneId": "alpine-lake", "order": 1, "kind": "ellipse", "geometry": {"x": 150, "y": 100, "width": 130, "height": 60}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "cloud-puff", "fill": {"kind": "token", "ref": "paper"}},
    {"id": "cloud-c", "sceneId": "alpine-lake", "order": 2, "kind": "ellipse", "geometry": {"x": 210, "y": 130, "width": 140, "height": 45}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "cloud-puff", "fill": {"kind": "token", "ref": "paper"}},
    {"id": "haze", "sceneId": "alpine-lake", "order": 3, "kind": "group", "geometry": {}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Haze"},
    {"id": "haze-band", "sceneId": "alpine-lake", "order": 0, "kind": "rect", "geometry": {"x": 0, "y": 300, "width": 800, "height": 72}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 0.4, "visible": true, "parentId": "haze", "fill": {"kind": "token", "ref": "mist"}},
    {"id": "peaks", "sceneId": "alpine-lake", "order": 4, "kind": "group", "geometry": {}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Peaks"},
    {"id": "peak-far", "sceneId": "alpine-lake", "order": 0, "kind": "polygon", "geometry": {"points": [[40, 350], [260, 150], [480, 350]]}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "peaks", "fill": {"kind": "token", "ref": "ridge"}},
    {"id": "peak-near", "sceneId": "alpine-lake", "order": 1, "kind": "polygon", "geometry": {"points": [[300, 350], [520, 120], [740, 350]]}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "peaks", "fill": {"kind": "token", "ref": "rock"}},
    {"id": "snow", "sceneId": "alpine-lake", "order": 2, "kind": "boolean", "geometry": {"operation": "intersect"}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "peaks", "name": "Snow cap", "fill": {"kind": "token", "ref": "snow"}},
    {"id": "snow-mask", "sceneId": "alpine-lake", "order": 0, "kind": "polygon", "geometry": {"points": [[455, 205], [520, 120], [585, 205], [520, 235]]}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "snow"},
    {"id": "snow-clip", "sceneId": "alpine-lake", "order": 1, "kind": "polygon", "geometry": {"points": [[430, 350], [520, 120], [610, 350]]}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "snow"},
    {"id": "glacier", "sceneId": "alpine-lake", "order": 3, "kind": "boolean", "geometry": {"operation": "subtract"}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "peaks", "name": "Glacier", "fill": {"kind": "token", "ref": "paper"}},
    {"id": "ice-mass", "sceneId": "alpine-lake", "order": 0, "kind": "polygon", "geometry": {"points": [[70, 370], [130, 300], [200, 370]]}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "glacier"},
    {"id": "ice-cut", "sceneId": "alpine-lake", "order": 1, "kind": "polygon", "geometry": {"points": [[110, 370], [150, 300], [150, 370]]}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "glacier"},
    {"id": "ridge", "sceneId": "alpine-lake", "order": 5, "kind": "group", "geometry": {}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Near shore"},
    {"id": "hillside", "sceneId": "alpine-lake", "order": 0, "kind": "polygon", "geometry": {"points": [[0, 430], [220, 320], [430, 430], [640, 360], [800, 320], [800, 600], [0, 600]]}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "ridge", "fill": {"kind": "token", "ref": "pine-dark"}},
    {"id": "pond", "sceneId": "alpine-lake", "order": 6, "kind": "group", "geometry": {}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Lake"},
    {"id": "pond-shape", "sceneId": "alpine-lake", "order": 0, "kind": "boolean", "geometry": {"operation": "union"}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "pond", "name": "Water", "fill": {"kind": "token", "ref": "water"}},
    {"id": "pond-a", "sceneId": "alpine-lake", "order": 0, "kind": "ellipse", "geometry": {"x": 110, "y": 420, "width": 320, "height": 100}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "pond-shape"},
    {"id": "pond-b", "sceneId": "alpine-lake", "order": 1, "kind": "ellipse", "geometry": {"x": 330, "y": 420, "width": 320, "height": 100}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "pond-shape"},
    {"id": "ripples", "sceneId": "alpine-lake", "order": 1, "kind": "alongPath", "geometry": {"pathData": "M160 468 L620 468", "count": 5}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "pond", "name": "Ripples"},
    {"id": "ripple", "sceneId": "alpine-lake", "order": 0, "kind": "ellipse", "geometry": {"x": -22, "y": -5, "width": 44, "height": 10}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 0.8, "visible": true, "parentId": "ripples", "fill": {"kind": "token", "ref": "water-light"}},
    {"id": "reeds", "sceneId": "alpine-lake", "order": 2, "kind": "alongPath", "geometry": {"pathData": "M138 512 L246 446", "count": 6}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "pond", "name": "Reeds"},
    {"id": "reed-clump", "sceneId": "alpine-lake", "order": 0, "kind": "group", "geometry": {}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "reeds", "name": "Clump"},
    {"id": "reed", "sceneId": "alpine-lake", "order": 0, "kind": "rect", "geometry": {"x": -3, "y": -46, "width": 6, "height": 46}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "reed-clump", "fill": {"kind": "token", "ref": "reed"}},
    {"id": "forest-far", "sceneId": "alpine-lake", "order": 7, "kind": "group", "geometry": {}, "transform": {"translateX": 70.0, "translateY": 416.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Far pines"},
    {"id": "pines-far", "sceneId": "alpine-lake", "order": 0, "kind": "repeat", "geometry": {"count": 5, "spacing": 130}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "forest-far", "name": "Far row"},
    {"id": "pine-far", "sceneId": "alpine-lake", "order": 0, "kind": "instance", "geometry": {}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 0.55, "scaleY": 0.55}, "opacity": 0.7, "visible": true, "parentId": "pines-far", "name": "Pine", "definitionRef": "pine", "bindings": [{"name": "foliage", "value": "pine-far"}]},
    {"id": "forest", "sceneId": "alpine-lake", "order": 8, "kind": "group", "geometry": {}, "transform": {"translateX": 40.0, "translateY": 434.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Forest"},
    {"id": "pines", "sceneId": "alpine-lake", "order": 0, "kind": "repeat", "geometry": {"count": 6, "spacing": 108}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "forest", "name": "Pines"},
    {"id": "pine-near", "sceneId": "alpine-lake", "order": 0, "kind": "instance", "geometry": {}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "pines", "name": "Pine", "definitionRef": "pine"},
    {"id": "shrub-a", "sceneId": "alpine-lake", "order": 9, "kind": "group", "geometry": {}, "transform": {"translateX": 540.0, "translateY": 438.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Shrub A"},
    {"id": "shrub-a-leaf", "sceneId": "alpine-lake", "order": 0, "kind": "ellipse", "geometry": {"x": -26, "y": -18, "width": 52, "height": 36}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "shrub-a", "fill": {"kind": "token", "ref": "pine"}},
    {"id": "shrub-b", "sceneId": "alpine-lake", "order": 10, "kind": "group", "geometry": {}, "transform": {"translateX": 600.0, "translateY": 438.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Shrub B"},
    {"id": "shrub-b-leaf", "sceneId": "alpine-lake", "order": 0, "kind": "ellipse", "geometry": {"x": -26, "y": -18, "width": 52, "height": 36}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "shrub-b", "fill": {"kind": "token", "ref": "pine"}},
    {"id": "shrub-c", "sceneId": "alpine-lake", "order": 11, "kind": "group", "geometry": {}, "transform": {"translateX": 660.0, "translateY": 438.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Shrub C"},
    {"id": "shrub-c-leaf", "sceneId": "alpine-lake", "order": 0, "kind": "ellipse", "geometry": {"x": -26, "y": -18, "width": 52, "height": 36}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "shrub-c", "fill": {"kind": "token", "ref": "pine"}},
    {"id": "flowers", "sceneId": "alpine-lake", "order": 12, "kind": "group", "geometry": {}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Wildflowers"},
    {"id": "flower-0", "sceneId": "alpine-lake", "order": 0, "kind": "ellipse", "geometry": {"x": 646, "y": 470, "width": 16, "height": 16}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "flowers", "fill": {"kind": "token", "ref": "accent"}},
    {"id": "flower-1", "sceneId": "alpine-lake", "order": 1, "kind": "ellipse", "geometry": {"x": 676, "y": 488, "width": 16, "height": 16}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "flowers", "fill": {"kind": "token", "ref": "accent"}},
    {"id": "flower-2", "sceneId": "alpine-lake", "order": 2, "kind": "ellipse", "geometry": {"x": 704, "y": 468, "width": 16, "height": 16}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "flowers", "fill": {"kind": "token", "ref": "accent"}},
    {"id": "trail", "sceneId": "alpine-lake", "order": 13, "kind": "alongPath", "geometry": {"pathData": "M50 566 C 250 520 430 596 760 506", "count": 9}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Trail"},
    {"id": "step", "sceneId": "alpine-lake", "order": 0, "kind": "ellipse", "geometry": {"x": -16, "y": -8, "width": 32, "height": 16}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "trail", "fill": {"kind": "token", "ref": "stone"}},
    {"id": "pier", "sceneId": "alpine-lake", "order": 14, "kind": "projection", "geometry": {"axis": "isometric"}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Pier"},
    {"id": "pier-deck", "sceneId": "alpine-lake", "order": 0, "kind": "rect", "geometry": {"x": 560, "y": 430, "width": 140, "height": 26}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "parentId": "pier", "fill": {"kind": "token", "ref": "trunk"}},
    {"id": "caption", "sceneId": "alpine-lake", "order": 15, "kind": "text", "geometry": {"text": "Alpine Lake", "fontSize": 42, "x": 48, "y": 76, "align": "start", "lineHeight": 48, "letterSpacing": 0}, "transform": {"translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0}, "opacity": 1.0, "visible": true, "name": "Title", "fill": {"kind": "token", "ref": "ink"}}
  ],
  "constraints": [
    {"id": "shrub-row", "sceneId": "alpine-lake", "kind": "equalSpacing", "elementIds": ["shrub-a", "shrub-b", "shrub-c"], "axis": "x", "value": 44}
  ]
}
```

Whether a part is a definition or an element in the scene, compose in this
order:

1. Block in the large shapes first — sky, peaks, shore, lake — as siblings with
   increasing `order`, and validate.
2. Group what belongs together (the sun, the lake, the forest) and add the
   detail inside each group, where the group's transform carries it.
3. Reach for a composition element for repetition, booleans, paths, and
   outlines, and give every operand `parentId` equal to the composition's `id`.
4. Render and inspect. A small shape lost behind a larger one is an `order`
   problem; a repeated row that drifts is a `spacing` problem; a copy facing the
   wrong way along a guide is an `alongPath` direction problem; a copy far from
   where you expected is a composition-relative coordinate problem.

## 5. Reusable parts: definitions and instances

A reusable part is a **Definition**: a project-scoped group of elements with its
own identity and an optional set of named parameters. It lives in
`definitions/<id>.json` and is placed by an `instance` element, in one scene or
several. Author a part once and place it wherever it is needed instead of
repeating its elements by hand.

A definition declares its `parameters`, an `origin`, and its own `elements`.
Its `id` and its elements' `id`s share one namespace with the project's scene
elements, so none of them may collide with each other or with a scene element.
Each element carries `definitionId` where a scene element carries `sceneId` —
exactly one of the two, in the same element shape. A parameter-capable field
holds `{"param": "<name>"}` in place of a literal; a `token` parameter replaces a
whole paint, so `{"param": "colour"}` is a fill. A reference is valid only inside
the definition that declares the parameter, and its type must match the field.
`definitions/cloud.json`:

```json
{
  "id": "cloud",
  "projectId": "project",
  "name": "Cloud",
  "parameters": [
    { "name": "colour", "type": "token", "default": "paper" },
    { "name": "size", "type": "number", "default": 120 }
  ],
  "origin": { "x": 0, "y": 0 },
  "elements": [
    {
      "id": "cloud-body",
      "definitionId": "cloud",
      "order": 0,
      "kind": "ellipse",
      "name": "Cloud body",
      "geometry": { "x": 0, "y": 0, "width": { "param": "size" }, "height": 40 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "param": "colour" },
      "opacity": 1,
      "visible": true
    }
  ]
}
```

An `instance` element places it. The instance carries its own `transform` and
`opacity` and no geometry of its own; it renders the definition's elements under
that placement, binding each parameter this use varies and taking the declared
`default` for one it leaves unbound:

```json
{
  "id": "cloud-left",
  "sceneId": "skyline",
  "order": 1,
  "kind": "instance",
  "name": "Cloud left",
  "geometry": {},
  "transform": { "translateX": 70, "translateY": 60, "rotate": 0, "scaleX": 1, "scaleY": 1 },
  "opacity": 1,
  "visible": true,
  "definitionRef": "cloud",
  "bindings": [ { "name": "size", "value": 90 } ]
}
```

One definition can be placed many times with different bindings, and a scene in
the same project can place it too; editing the definition changes every
placement on the next compile. A definition may itself place another definition,
so parts compose into deeper wholes. A definition that no scene places renders
nothing and warns (`W_UNUSED_DEFINITION`); it is not an error.

The scene that places the part, `scenes/skyline.json`, uses one definition twice
with different bindings — the second instance varies `colour` and takes the
default `size`:

```json
{
  "id": "skyline",
  "projectId": "project",
  "name": "Skyline",
  "formatVersion": "0.2",
  "paletteId": "brand",
  "canvas": { "width": 320, "height": 200, "background": "transparent" },
  "elements": [
    {
      "id": "backdrop",
      "sceneId": "skyline",
      "order": 0,
      "kind": "rect",
      "geometry": { "x": 0, "y": 0, "width": 320, "height": 200 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "sky" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "cloud-left",
      "sceneId": "skyline",
      "order": 1,
      "kind": "instance",
      "name": "Cloud left",
      "geometry": {},
      "transform": { "translateX": 70, "translateY": 60, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true,
      "definitionRef": "cloud",
      "bindings": [ { "name": "size", "value": 90 } ]
    },
    {
      "id": "cloud-right",
      "sceneId": "skyline",
      "order": 2,
      "kind": "instance",
      "name": "Cloud right",
      "geometry": {},
      "transform": { "translateX": 210, "translateY": 90, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 0.6,
      "visible": true,
      "definitionRef": "cloud",
      "bindings": [ { "name": "colour", "value": "paper" } ]
    }
  ]
}
```

## 6. Build a complex graphic up in verified parts

A detailed illustration is reliable when each part is correct before it is
composed. This is the method for a complex request; a simple mark does not need
it. Work one part at a time, and never compose a part that has not been
verified. Read **Author for depth and structure** alongside it: the depth,
relative-placement, shared-anchor, and research directives are what each part is
authored against.

1. **Decompose.** Name the parts the drawing is made of and record the order.
   Name each part's shared anchor as well as its order — the point where it meets
   the part it sits against — so the composition meets there rather than leaving
   a seam. When the request names no parts, use the documented default
   decomposition: background and sky; the midground masses (land, water, large
   structures); the repeating or reused objects (trees, clouds, ripples, steps);
   then the foreground detail (reeds, stones, flowers, text). A request that does
   not decompose cleanly still gets this decomposition rather than a stall.
2. **Author one part as a definition.** Write `definitions/<part>.json`. Give it
   a parameter for each value a use may vary, with a default, so one definition
   serves every placement.
3. **Verify the part in isolation.** Render it on its own and look at the
   result:

   ```sh
   vectr render <part> --format png --out dist/<part>.png
   ```

   The render parses and validates the definition structurally, then draws the
   part alone, framed to its own bounds; it prints the frame it used. Read the
   diagnostics on failure — `E_SCHEMA`, `E_DEFINITION_CYCLE`, `E_BINDING`, and
   the definition's own reference errors all name the element and its location.
   Correct the definition and render it again until it is structurally sound and
   matches the request. Do not compose a part that is not verified.
4. **Compose the verified part.** Add an `instance` element to the scene,
   binding the parameters this use varies and setting the placement transform.
   Then validate and render the scene so far:

   ```sh
   vectr validate <scene>
   vectr export <scene> --format png --out dist/<scene>.png
   ```

   Inspect the composition before adding the next part. If it does not match,
   correct the instance's placement, binding, or anchor — the parts are already
   verified, so a composition failure names the composition step, not the parts.
5. **Repeat for each part.** Place a part already verified without re-authoring
   it: another instance, or another scene in the same project, reuses the same
   definition.
6. **Verify the whole and export.** When every part is composed, validate and
   compile the whole scene, then export the final SVG and PNG.

Two ordering rules. Author and verify a part that depends on another — a boat on
the lake, a tree on the hill — after the part it sits on, and record that order.
If an increment expands past the tool's element limit, the error names the
definition and refuses the whole rather than truncating it; reduce the part's
repetition or nesting, or split it, and re-verify. The whole is complete when
every part is placed and the scene validates, compiles, and renders
deterministically with no part dropped.

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

## 8. Render and look at the result

Render a preview you can see:

```sh
vectr export habit-logo --format png --out dist/habit-logo.png --width 512 --height 512
```

Then open the PNG and compare it against the request. Reading the image is the
step that catches what a structural check cannot. Export the final deliverables
once the preview matches:

```sh
vectr export habit-logo --format svg --out dist/habit-logo.svg
vectr export habit-logo --format png --out dist/habit-logo.png --width 512 --height 512
```

Exports default to `dist/<scene-id>.<ext>`; `--out`/`out` chooses another path.
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
diagnostics and produce no output. Never export from a scene that did not
validate and compile. If the authoring model is unavailable, stop and report
that authoring cannot proceed and produce no output; never emit a placeholder
asset.

## 10. Defaults for an ambiguous request

When the request leaves something open, choose the documented default, state it
in one line, and proceed rather than stopping to ask:

| Open point | Default |
|---|---|
| Canvas size | 512×512 |
| Scene identifier | A short kebab-case name for the request (e.g. `habit-logo`); its file is `scenes/<id>.json` and commands address it by that id |
| Background | `transparent` |
| Scope of the graphic | When the request leaves it open, one simple mark — a primary shape plus an optional wordmark. A request that asks for a detailed or complex illustration is authored in full |
| Parts of a complex request | The default decomposition in the build-up method: background and sky; midground masses; repeating or reused objects; then foreground detail |
| Shape placement | Centred, with an even margin from each edge |
| Colours | A palette named for the request (`accent`, `ink`, `paper`) so it restyles |
| Text font | The bundled open-licensed sans (omit `fontId`) |
| Recipe | The project's `defaultRecipeId`; set `recipeId` only when the request names a look |
| Icon set | Every icon on the same canvas and style, each exported to its own file |

Keep a simple request's first version valid and renderable, then refine it toward
the request. A detailed or complex request is built up in verified parts
(section 6), not authored in one pass and not simplified to a simple mark.

## 11. Licensing and cost

Vectr claims no ownership of the content an authoring model produces. That
content may carry the model provider's own terms; check your provider's terms
before you ship generated graphics. Vectr ships only open-licensed fonts (SIL
OFL); never assume or bundle a commercial font — a user-supplied font is the
user's responsibility.

The model's tokens are billed by the user's provider, not by Vectr. Keep the
loop cheap: read only the schema types you need rather than the whole contract,
and send targeted corrections rather than re-emitting the entire scene when one
element is wrong.
