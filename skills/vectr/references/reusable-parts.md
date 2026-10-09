# Reusable parts: definitions and instances

Read this when a request needs a reusable part. A reusable part is a
**Definition**: a project-scoped group of elements with its own identity and an
optional set of named parameters. It lives in `definitions/<id>.json` and is
placed by an `instance` element, in one scene or several. Author a part once and
place it wherever it is needed instead of repeating its elements by hand.

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
