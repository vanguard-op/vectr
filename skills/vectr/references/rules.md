# Rules the schema does not state

Read this while authoring the scene. The contract gives properties, types, and
allowed values; these rules are enforced by the engine and are easy to miss.

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
  procedure names it, and `vectr schema` reports it as `x-vectr-formatVersion`;
  a value the tool does not support is `E_FORMAT_VERSION`.
