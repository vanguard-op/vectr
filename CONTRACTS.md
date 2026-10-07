# Contracts

### C-001: SCENE scene document (strict JSON)
Status: Frozen (revision 1)
Auth: None. Local file input, treated as untrusted: bounded size, no external entity expansion, no file or network access from a scene (NFR-021).
Request: Scene document per docs/Vectr/schema.md — id: string; projectId: string; name: string; formatVersion: string (^[0-9]+\.[0-9]+$); canvas: {width: number, height: number, background: string}; paletteId?: string; recipeId?: string; title?: string; description?: string; elements: Element[]; Element requires {id: string, sceneId: string, order: integer, kind: enum[rect, ellipse, polygon, line, path, group, repeat, boolean, alongPath, offset, projection, raster], geometry: object, transform: Transform, opacity: number 0..1, visible: boolean} and may omit or carry {parentId: string|null, name: string, fillToken: string|null, strokeProfileId: string|null, strokeToken: string|null}; Transform requires {translateX: number, translateY: number, rotate: number, scaleX: number, scaleY: number} and may carry {skewX: number, skewY: number}; constraints?: Constraint[] (id: string, sceneId: string, kind: enum[equalSpacing, align, attach, contain, snapToGrid], elementIds: string[>=2], axis?: enum[x, y, both]|null, value?: number|null). A stroke exists only when both `strokeProfileId` and `strokeToken` are set; either without the other is a validation error.
Response Ok: Parsed scene model; every element addressable by a stable identifier; the document round-trips without loss.
Errors: E_SCHEMA (unknown property, missing required field, or invalid value, with location), E_PARSE (not valid JSON or not a scene document), E_FORMAT_VERSION (unsupported formatVersion; names the version and the supported range), E_DUPLICATE_ID (duplicate element identifier)

### C-002: CALL vectr-core library API
Status: Frozen (revision 1)
Auth: In-process caller; no authentication. Deterministic and safe for concurrent use with no shared mutable state (FEAT-021).
Request: parse(source: &str) -> Result<Scene, Diagnostics>; validate(&Scene) -> Diagnostics; compile(&Scene) -> Result<RenderModel, Diagnostics>; compile_with_style(&Scene, &StyleContext) -> Result<RenderModel, Diagnostics>; export_svg(&RenderModel, &SvgOptions) -> Result<String, Diagnostics>; export_png(&RenderModel, &RasterOptions) -> Result<Vec<u8>, Diagnostics>. `compile` runs without style assets and carries the scene's declared style references; `compile_with_style` resolves them against the caller's palette and stroke profiles (D-013).
Response Ok: RenderModel, SVG text, or PNG bytes; identical input and seed yields byte-identical output (NFR-010).
Errors: Diagnostics — structured findings carrying severity (error|warning) and location (element id + JSON path); classes: invalid scene, unsupported feature, reference cycle, constraint conflict, defined size limit, missing rasterizer, missing font

### C-003: MODEL render model (compiler to exporter seam)
Status: Frozen (revision 1)
Auth: Internal to vectr-core; read by every exporter (docs/Vectr/architecture.md, "Component: Compiler" and "Component: Export Layer").
Request: RenderModel { canvas: {width: number, height: number, background: string}, nodes: ResolvedNode[], meta: {title?: string, description?: string}, diagnostics: Diagnostic[] }; ResolvedNode = { id: string, name?: string, order: integer, kind: string, geometry: concrete Shape (rect {x, y, width, height, rx, ry} | ellipse {cx, cy, rx, ry} | polygon {points} | line {points} | path {subpaths: [{segments, closed}]}), transform: resolved Affine, paint: {fill?: string, stroke?: {value: string, width: number, cap: string, join: string}}, opacity: number, visible: boolean }. `nodes` is flat, in paint order; a group or composition is lowered to concrete nodes. `paint.fill` and `paint.stroke.value` are concrete palette colours resolved from the element's fill and stroke tokens; no unresolved colour reference survives compilation. The model serializes to camelCase JSON (D-014).
Response Ok: SVG text, PNG bytes, or PDF bytes whose appearance matches the render model.
Errors: none at this seam (compilation already succeeded); exporter findings are warnings: unsupported-feature (omitted with a warning), rasterizer-missing, font-missing

### C-004: CLI vectr
Status: Frozen
Auth: None. Runs as the local user; no authentication and no network (docs/Vectr/architecture.md, "Cross-Cutting Concerns").
Request: vectr init [dir]; vectr validate <scene> [--json]; vectr compile <scene> [--out <file>] [--check]; vectr export <scene> --format svg|png [--out <file>] [--width <n>] [--height <n>] [--density <n>] [--background <color|transparent>]. (render and inspect reserved for FEAT-022; schema reserved for FEAT-017.)
Response Exit 0: success; output written, or nothing written under --check.
Errors: Exit 1 (scene invalid; diagnostics printed), Exit 2 (usage error or missing/unreadable input), Exit 3 (compilation failure: constraint conflict, cycle, defined size limit), Exit 4 (export dependency missing: rasterizer or font), Exit 5 (output I/O failure: unwritable path). Every non-zero exit prints a structured diagnostic with its location; no partial output is written and existing files are left untouched (NFR-011).
