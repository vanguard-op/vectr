# Alpine lake at dawn — a compositionally complex illustration

## Request

> Build a rich, detailed illustration: a mountain lake at dawn with snow-capped
> peaks, a pine forest, reeds at the water's edge, a trail of stepping stones,
> and a low sun.

## Deduced plan

The prompt describes a picture and prescribes no structure, so the sections, the
kind of each, the recipe, the palette, and the depth handling are deduced from
it.

- **Sections, back to front:** sky; sun (halo and disc); clouds; haze band;
  peaks (far ridge, near peak, snow cap, glacier); near shore; lake (water,
  ripples, reeds); far pines; forest; shrubs; wildflowers; trail; pier; caption.
- **Kind of each section.** The whole is one scene; each depth layer is a
  `group` that moves as one; the pines are a reusable definition (`pine`,
  `examples/pine.md`) placed twice, so the far and near forests reuse one
  authored part instead of re-authoring a tree per row.
- **Recipe and palette.** The project's default recipe; the `brand` palette
  carries the atmospheric tokens (`mist`, `ridge`, `rock`, `pine-far`) that
  recede distance through colour.
- **Depth.** Paint order is depth: the sky takes the lowest `order` and the
  caption the highest, and each layer group carries an increasing `order`.
  Distant pines scale to `0.55` and fade to `0.7` opacity; the far ridge takes
  `ridge` and the near peak `rock`; a `mist` haze band sits between them.
- **Anchors.** `pine`'s `origin` sits at its trunk base, the shared anchor every
  placement joins the shore at.
- **Method.** The whole is sketched at low fidelity first, then each section is
  refined, rendered on its own, and integrated before the next; the scene is
  exported only once the whole passes.

## Scene

`scenes/alpine-lake.json`:

```json
{
  "id": "alpine-lake",
  "projectId": "project",
  "name": "Alpine lake at dawn",
  "formatVersion": "0.2",
  "paletteId": "brand",
  "title": "Alpine lake at dawn",
  "canvas": { "width": 800, "height": 600, "background": "transparent" },
  "elements": [
    {
      "id": "sky",
      "sceneId": "alpine-lake",
      "order": 0,
      "kind": "rect",
      "name": "Sky",
      "geometry": { "x": 0, "y": 0, "width": 800, "height": 600 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "sky" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "sun",
      "sceneId": "alpine-lake",
      "order": 1,
      "kind": "group",
      "name": "Sun",
      "geometry": {},
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "halo",
      "sceneId": "alpine-lake",
      "parentId": "sun",
      "order": 0,
      "kind": "offset",
      "name": "Halo",
      "geometry": { "distance": 30 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "dawn" },
      "opacity": 0.55,
      "visible": true
    },
    {
      "id": "halo-src",
      "sceneId": "alpine-lake",
      "parentId": "halo",
      "order": 0,
      "kind": "ellipse",
      "name": "Halo source",
      "geometry": { "x": 600, "y": 60, "width": 110, "height": 110 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "sun-disc",
      "sceneId": "alpine-lake",
      "parentId": "sun",
      "order": 1,
      "kind": "ellipse",
      "name": "Disc",
      "geometry": { "x": 600, "y": 60, "width": 110, "height": 110 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "sun" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "clouds",
      "sceneId": "alpine-lake",
      "order": 2,
      "kind": "repeat",
      "name": "Clouds",
      "geometry": { "count": 2, "spacing": 250 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "cloud-puff",
      "sceneId": "alpine-lake",
      "parentId": "clouds",
      "order": 0,
      "kind": "group",
      "name": "Cloud",
      "geometry": {},
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "cloud-a",
      "sceneId": "alpine-lake",
      "parentId": "cloud-puff",
      "order": 0,
      "kind": "ellipse",
      "geometry": { "x": 90, "y": 120, "width": 150, "height": 50 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "paper" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "cloud-b",
      "sceneId": "alpine-lake",
      "parentId": "cloud-puff",
      "order": 1,
      "kind": "ellipse",
      "geometry": { "x": 150, "y": 100, "width": 130, "height": 60 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "paper" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "cloud-c",
      "sceneId": "alpine-lake",
      "parentId": "cloud-puff",
      "order": 2,
      "kind": "ellipse",
      "geometry": { "x": 210, "y": 130, "width": 140, "height": 45 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "paper" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "haze",
      "sceneId": "alpine-lake",
      "order": 3,
      "kind": "group",
      "name": "Haze",
      "geometry": {},
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "haze-band",
      "sceneId": "alpine-lake",
      "parentId": "haze",
      "order": 0,
      "kind": "rect",
      "geometry": { "x": 0, "y": 300, "width": 800, "height": 72 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "mist" },
      "opacity": 0.4,
      "visible": true
    },
    {
      "id": "peaks",
      "sceneId": "alpine-lake",
      "order": 4,
      "kind": "group",
      "name": "Peaks",
      "geometry": {},
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "peak-far",
      "sceneId": "alpine-lake",
      "parentId": "peaks",
      "order": 0,
      "kind": "polygon",
      "geometry": { "points": [[40, 350], [260, 150], [480, 350]] },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "ridge" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "peak-near",
      "sceneId": "alpine-lake",
      "parentId": "peaks",
      "order": 1,
      "kind": "polygon",
      "geometry": { "points": [[300, 350], [520, 120], [740, 350]] },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "rock" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "snow",
      "sceneId": "alpine-lake",
      "parentId": "peaks",
      "order": 2,
      "kind": "boolean",
      "name": "Snow cap",
      "geometry": { "operation": "intersect" },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "snow" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "snow-mask",
      "sceneId": "alpine-lake",
      "parentId": "snow",
      "order": 0,
      "kind": "polygon",
      "geometry": { "points": [[455, 205], [520, 120], [585, 205], [520, 235]] },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "snow-clip",
      "sceneId": "alpine-lake",
      "parentId": "snow",
      "order": 1,
      "kind": "polygon",
      "geometry": { "points": [[430, 350], [520, 120], [610, 350]] },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "glacier",
      "sceneId": "alpine-lake",
      "parentId": "peaks",
      "order": 3,
      "kind": "boolean",
      "name": "Glacier",
      "geometry": { "operation": "subtract" },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "paper" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "ice-mass",
      "sceneId": "alpine-lake",
      "parentId": "glacier",
      "order": 0,
      "kind": "polygon",
      "geometry": { "points": [[70, 370], [130, 300], [200, 370]] },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "ice-cut",
      "sceneId": "alpine-lake",
      "parentId": "glacier",
      "order": 1,
      "kind": "polygon",
      "geometry": { "points": [[110, 370], [150, 300], [150, 370]] },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "ridge",
      "sceneId": "alpine-lake",
      "order": 5,
      "kind": "group",
      "name": "Near shore",
      "geometry": {},
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "hillside",
      "sceneId": "alpine-lake",
      "parentId": "ridge",
      "order": 0,
      "kind": "polygon",
      "geometry": { "points": [[0, 430], [220, 320], [430, 430], [640, 360], [800, 320], [800, 600], [0, 600]] },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "pine-dark" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "pond",
      "sceneId": "alpine-lake",
      "order": 6,
      "kind": "group",
      "name": "Lake",
      "geometry": {},
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "pond-shape",
      "sceneId": "alpine-lake",
      "parentId": "pond",
      "order": 0,
      "kind": "boolean",
      "name": "Water",
      "geometry": { "operation": "union" },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "water" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "pond-a",
      "sceneId": "alpine-lake",
      "parentId": "pond-shape",
      "order": 0,
      "kind": "ellipse",
      "geometry": { "x": 110, "y": 420, "width": 320, "height": 100 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "pond-b",
      "sceneId": "alpine-lake",
      "parentId": "pond-shape",
      "order": 1,
      "kind": "ellipse",
      "geometry": { "x": 330, "y": 420, "width": 320, "height": 100 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "ripples",
      "sceneId": "alpine-lake",
      "parentId": "pond",
      "order": 1,
      "kind": "alongPath",
      "name": "Ripples",
      "geometry": { "pathData": "M160 468 L620 468", "count": 5 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "ripple",
      "sceneId": "alpine-lake",
      "parentId": "ripples",
      "order": 0,
      "kind": "ellipse",
      "geometry": { "x": -22, "y": -5, "width": 44, "height": 10 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "water-light" },
      "opacity": 0.8,
      "visible": true
    },
    {
      "id": "reeds",
      "sceneId": "alpine-lake",
      "parentId": "pond",
      "order": 2,
      "kind": "alongPath",
      "name": "Reeds",
      "geometry": { "pathData": "M138 512 L246 446", "count": 6 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "reed-clump",
      "sceneId": "alpine-lake",
      "parentId": "reeds",
      "order": 0,
      "kind": "group",
      "name": "Clump",
      "geometry": {},
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "reed",
      "sceneId": "alpine-lake",
      "parentId": "reed-clump",
      "order": 0,
      "kind": "rect",
      "geometry": { "x": -3, "y": -46, "width": 6, "height": 46 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "reed" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "forest-far",
      "sceneId": "alpine-lake",
      "order": 7,
      "kind": "group",
      "name": "Far pines",
      "geometry": {},
      "transform": { "translateX": 70, "translateY": 416, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "pines-far",
      "sceneId": "alpine-lake",
      "parentId": "forest-far",
      "order": 0,
      "kind": "repeat",
      "name": "Far row",
      "geometry": { "count": 5, "spacing": 130 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "pine-far",
      "sceneId": "alpine-lake",
      "parentId": "pines-far",
      "order": 0,
      "kind": "instance",
      "name": "Pine",
      "geometry": {},
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 0.55, "scaleY": 0.55 },
      "opacity": 0.7,
      "visible": true,
      "definitionRef": "pine",
      "bindings": [{ "name": "foliage", "value": "pine-far" }]
    },
    {
      "id": "forest",
      "sceneId": "alpine-lake",
      "order": 8,
      "kind": "group",
      "name": "Forest",
      "geometry": {},
      "transform": { "translateX": 40, "translateY": 434, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "pines",
      "sceneId": "alpine-lake",
      "parentId": "forest",
      "order": 0,
      "kind": "repeat",
      "name": "Pines",
      "geometry": { "count": 6, "spacing": 108 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "pine-near",
      "sceneId": "alpine-lake",
      "parentId": "pines",
      "order": 0,
      "kind": "instance",
      "name": "Pine",
      "geometry": {},
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true,
      "definitionRef": "pine"
    },
    {
      "id": "shrub-a",
      "sceneId": "alpine-lake",
      "order": 9,
      "kind": "group",
      "name": "Shrub A",
      "geometry": {},
      "transform": { "translateX": 540, "translateY": 438, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "shrub-a-leaf",
      "sceneId": "alpine-lake",
      "parentId": "shrub-a",
      "order": 0,
      "kind": "ellipse",
      "geometry": { "x": -26, "y": -18, "width": 52, "height": 36 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "pine" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "shrub-b",
      "sceneId": "alpine-lake",
      "order": 10,
      "kind": "group",
      "name": "Shrub B",
      "geometry": {},
      "transform": { "translateX": 600, "translateY": 438, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "shrub-b-leaf",
      "sceneId": "alpine-lake",
      "parentId": "shrub-b",
      "order": 0,
      "kind": "ellipse",
      "geometry": { "x": -26, "y": -18, "width": 52, "height": 36 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "pine" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "shrub-c",
      "sceneId": "alpine-lake",
      "order": 11,
      "kind": "group",
      "name": "Shrub C",
      "geometry": {},
      "transform": { "translateX": 660, "translateY": 438, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "shrub-c-leaf",
      "sceneId": "alpine-lake",
      "parentId": "shrub-c",
      "order": 0,
      "kind": "ellipse",
      "geometry": { "x": -26, "y": -18, "width": 52, "height": 36 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "pine" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "flowers",
      "sceneId": "alpine-lake",
      "order": 12,
      "kind": "group",
      "name": "Wildflowers",
      "geometry": {},
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "flower-0",
      "sceneId": "alpine-lake",
      "parentId": "flowers",
      "order": 0,
      "kind": "ellipse",
      "geometry": { "x": 646, "y": 470, "width": 16, "height": 16 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "accent" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "flower-1",
      "sceneId": "alpine-lake",
      "parentId": "flowers",
      "order": 1,
      "kind": "ellipse",
      "geometry": { "x": 676, "y": 488, "width": 16, "height": 16 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "accent" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "flower-2",
      "sceneId": "alpine-lake",
      "parentId": "flowers",
      "order": 2,
      "kind": "ellipse",
      "geometry": { "x": 704, "y": 468, "width": 16, "height": 16 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "accent" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "trail",
      "sceneId": "alpine-lake",
      "order": 13,
      "kind": "alongPath",
      "name": "Trail",
      "geometry": { "pathData": "M50 566 C 250 520 430 596 760 506", "count": 9 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "step",
      "sceneId": "alpine-lake",
      "parentId": "trail",
      "order": 0,
      "kind": "ellipse",
      "geometry": { "x": -16, "y": -8, "width": 32, "height": 16 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "stone" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "pier",
      "sceneId": "alpine-lake",
      "order": 14,
      "kind": "projection",
      "name": "Pier",
      "geometry": { "axis": "isometric" },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "pier-deck",
      "sceneId": "alpine-lake",
      "parentId": "pier",
      "order": 0,
      "kind": "rect",
      "geometry": { "x": 560, "y": 430, "width": 140, "height": 26 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "trunk" },
      "opacity": 1,
      "visible": true
    },
    {
      "id": "caption",
      "sceneId": "alpine-lake",
      "order": 15,
      "kind": "text",
      "name": "Title",
      "geometry": { "text": "Alpine Lake", "fontSize": 42, "x": 48, "y": 76, "align": "start", "lineHeight": 48, "letterSpacing": 0 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "ink" },
      "opacity": 1,
      "visible": true
    }
  ],
  "constraints": [
    { "id": "shrub-row", "sceneId": "alpine-lake", "kind": "equalSpacing", "elementIds": ["shrub-a", "shrub-b", "shrub-c"], "axis": "x", "value": 44 }
  ]
}
```

## Definitions it uses

- `pine` — `examples/pine.md`, placed in the far and near forests.

## Notes

- Composition primitives do the repetition and shaping rather than enumerating
  primitives by hand: `repeat` for the clouds and the pine rows, `boolean` for
  the lake union, the snow-cap intersect, and the glacier subtract, `alongPath`
  for the ripples, reeds, and trail, `offset` for the sun halo, and `projection`
  for the pier.
- One `constraint` (`equalSpacing`) states the shrub row rather than
  hand-computing three positions.
- The snow cap and glacier are `boolean` compositions that carry their own
  paint; their children supply the shapes.
- The pier is the one constructed object shown in three dimensions, so it is
  projected on the `isometric` axis rather than stacked flat.
