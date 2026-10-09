# Pine — a reusable definition

## Request

From the alpine-lake request: *"a pine forest"*. The same pine stands in the far
layer and the near layer, so it is authored once and placed wherever it is
needed.

## Deduced plan

- **Kind of the section.** An `instance` — a reusable definition placed more
  than once. The far and near forests are the same part at different scales.
- **Anchor.** The definition's `origin` sits at the trunk base, the shared
  anchor every placement joins the shore at, so the parts meet rather than
  leaving a seam.
- **Parameter.** `foliage` (a `token`) lets the far row bind a hazier token
  without a second definition.
- **Depth.** The far placement scales the part down and fades it; the near one
  is full size. The scale lives in the instance's `transform`, not the
  definition's geometry.

## Definition

`definitions/pine.json`:

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

## Definitions it uses

None.

## Notes

- The canopies are polygons whose base edge sits on the trunk, so the parts join
  at a shared anchor rather than leaving a seam.
- `foliage` defaults to the `pine` token; the far placement binds `pine-far`,
  and the near placement takes the default.
- Verify the part on its own before placing it:
  `vectr render pine --format png --out dist/pine.png`.
