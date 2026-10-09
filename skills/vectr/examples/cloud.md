# Cloud — a reusable definition

## Request

From the skyline request: *"two clouds, one pale and one at a lower opacity"*.
The same cloud is placed twice, so it is authored once.

## Deduced plan

- **Kind of the section.** An `instance` — one reusable definition placed twice
  with different bindings.
- **Parameters.** `colour` (a `token`, default `paper`) and `size` (a `number`,
  default `120`) used as the body width, so each placement can vary the part
  without a second definition.
- **Anchor.** The definition's `origin` is the body's left edge, so a placement
  positions the cloud by where it sits in the sky.

## Definition

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

## Definitions it uses

None.

## Notes

- A `token` parameter replaces a whole paint, so `fill` is
  `{"param": "colour"}` — a fill, not a token reference — and the default
  resolves to the `paper` token.
- A definition renders only where a scene places it with an `instance` element;
  unplaced it renders nothing and warns (`W_UNUSED_DEFINITION`).
