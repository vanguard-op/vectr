# Reusable parts: definitions and instances

Read this when a section is a reusable part — placed more than once, or placed
across scenes. A reusable part is a **Definition**: a project-scoped group of
elements with its own identity and an optional set of named parameters. It lives
in `definitions/<id>.json` and is placed by an `instance` element, in one scene
or several. Author a part once and place it wherever it is needed instead of
repeating its elements by hand. It is one kind of section, not the required
unit: a section that is used once is authored in place.

A definition declares its `parameters`, an `origin`, and its own `elements`.
Its `id` and its elements' `id`s share one namespace with the project's scene
elements, so none of them may collide with each other or with a scene element.
Each element carries `definitionId` where a scene element carries `sceneId` —
exactly one of the two, in the same element shape.

A parameter-capable field holds `{"param": "<name>"}` in place of a literal; a
`token` parameter replaces a whole paint, so `{"param": "colour"}` is a fill. A
reference is valid only inside the definition that declares the parameter, and
its type must match the field. `examples/cloud.md` is a worked definition with
a `token` parameter (`colour`, default `paper`) and a `number` parameter
(`size`, default `120`) used as an ellipse width:

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

`examples/skyline.md` is a worked scene that places the `cloud` definition
twice with different bindings — the second varies `colour` and takes the default
`size`. `examples/alpine-lake.md` places the `pine` definition from
`examples/pine.md` across two depth layers, scaling the far row and binding
its `foliage` parameter to a hazier token; the definition's `origin` sits at the
trunk base, the shared anchor every placement joins the shore at.

Verify a definition on its own before it is placed, then place it and verify the
whole again:

```sh
vectr render cloud --format png --out dist/cloud.png
```

The render parses and validates the definition structurally and draws the part
alone, framed to its own bounds; it prints the frame it used. A definition that
does not render on its own is not verified, and an unverified part is never
composed.
