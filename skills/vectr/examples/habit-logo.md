# Habit logo — a simple mark

## Request

> make me a logo for a habit tracking app called Habit: a rounded badge with a
> check mark and the word Habit, in a purple

## Deduced plan

- **Sections.** One section: the mark. The badge, the check dot, and the
  wordmark move together, so they share one `group` parent.
- **Kind of the section.** A `group` — the whole mark is one unit, authored in
  place rather than as a reusable part.
- **Recipe and palette.** The project's default recipe; a `brand` palette so the
  purple is a token (`accent`) and the mark restyles by editing one value.
- **Depth.** None requested, so none is added.
- **Method.** A simple mark is sketched and refined in one pass; one section is
  the whole, so the loop takes a single turn.

## Scene

`scenes/habit-logo.json`:

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

## Definitions it uses

None: a mark authored once and used once is authored in place, not as a
reusable part.

## Notes

- The label is a `text` element anchored by `x`/`y` at the baseline of its first
  line, not the box: `y` sits below the badge so the wordmark reads centred
  under it.
- Every colour is a palette token, so restyling the brand is one edit and no
  element carries a raw colour.
