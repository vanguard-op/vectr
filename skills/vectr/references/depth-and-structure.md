# Author for depth and structure

Read this when a request implies depth or several parts. The hard end of the
range is a coherent object, not a set of disjoint shapes. Four directives apply;
each uses only the primitives the language already carries. Read them before
refining a section that carries depth, and check the render against them in the
inspect-and-correct reference (`inspect-and-correct.md`).

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
