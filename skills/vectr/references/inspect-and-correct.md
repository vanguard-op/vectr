# Inspect and correct

Read this after a render. Compare the render to the request on these axes, then
fix the scene and re-render:

| Symptom in the render | Likely cause | Correction |
|---|---|---|
| A shape is cut off at an edge | Geometry extends past the canvas | Move or shrink it inside `canvas.width`/`height`; leave a margin |
| A shape covers one it should sit behind | Wrong `order` | Raise the covering element's `order`, or reorder siblings |
| Two parts leave a gap or overlap where they should meet | The shared anchor differs | Give both parts the same value at the anchor (or the definition's `origin`), and correct the anchor rather than nudging one part by eye |
| The mark and its label come apart when moved | Children not grouped | Give them a common `group` parent and set their `parentId` |
| Text sits too high, low, or off the shape | `x`/`y` anchor the first-line baseline, not the box | Adjust `y` to the baseline you want; use `align` for horizontal centring |
| A colour does not match the brand | Element names the wrong token | Point the paint at the right palette token, or fix the token value |
| A stroke is invisible or too heavy | Hairline profile width `0` in a non-stroke recipe, or too large | Name a scaled profile, or a stroke-based recipe whose `strokeWeight` fits |
| Nothing changed after a style edit | The element never referenced the token | Move the element's paint to `{"kind": "token", "ref": ...}` |

Re-render after each correction and look again; stop when the render matches the
request.
