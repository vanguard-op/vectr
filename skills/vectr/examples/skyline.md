# Skyline — one definition placed twice

## Request

> a small skyline with two clouds, one pale and one at a lower opacity

## Deduced plan

- **Sections.** The sky and the two clouds. Both clouds are the same part, so
  the part is one reusable definition (`cloud`, `examples/cloud.md`) placed
  twice; the sky is a plain `rect` authored in place.
- **Kind of each section.** The sky is an element in place; each cloud is an
  `instance` of the `cloud` definition.
- **Recipe and palette.** The project's default recipe; the `brand` palette,
  with the sky on the `sky` token and the clouds on `paper`.
- **Depth.** The right cloud takes a lower `opacity` so it reads as the farther
  one; `order` places both over the sky.
- **Bindings.** The left placement binds `size`; the right binds `colour` and
  takes the default `size`, so one definition serves both.

## Scene

`scenes/skyline.json`:

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
      "bindings": [{ "name": "size", "value": 90 }]
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
      "bindings": [{ "name": "colour", "value": "paper" }]
    }
  ]
}
```

## Definitions it uses

- `cloud` — `examples/cloud.md`.

## Notes

- The right cloud varies `colour` and leaves `size` to the definition's default,
  showing one definition placed with different bindings.
- Editing `cloud` changes both placements on the next compile; the scene is not
  re-authored to restyle a cloud.
