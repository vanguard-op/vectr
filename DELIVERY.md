# Delivery

## Source of truth
docs/Vectr/ — git submodule, remote https://github.com/vanguard-op/vectr-docs.git; working tree at cc1cd19 (reusable parts, incremental authoring, and part-scoped rendering specified for Phase 4), the recorded pointer advancing with the build. The docs are authoritative for what to build; this file tracks state only and never restates the spec.

## Team & file ownership
| Member | Owns |
|---|---|
| lead | README.md, DELIVERY.md, CONTRACTS.md, ASSETS.md, .delivery/**, Cargo.lock |
| infra-engineer | Cargo.toml, rust-toolchain.toml, deny.toml, .gitignore, .github/**, scripts/**, assets/fonts/** |
| backend-engineer | crates/vectr-core/**, crates/vectr-cli/**, schema/** |
| ai-engineer | crates/vectr-mcp/**, crates/vectr-eval/**, skills/** |
| qa-engineer | tests/**, fixtures/**, corpus/** |
| frontend-engineer | (none — Vectr has no UI) |
| visual-artist | (none — the docs declare no imagery) |

Scratch: scratch/<task>/ — gitignored, private to the task's owner.

## Stack
Rust (stable, pinned by rust-toolchain.toml), cargo workspace. serde + schemars for the scene model and the generated JSON Schema; lyon + i_overlay for path geometry, boolean and offset; harfrust + skrifa for text and outlining; resvg/tiny-skia as the rasterizer behind the export layer; an in-crate vector PDF emitter. Evaluation harness: multiple pinned providers (OpenAI, Anthropic, Google) plus an OpenCode adapter. Distributed as crates.io packages and signed GitHub Release binaries with checksums for macOS, Linux and Windows.

## Structure
- Cargo.toml, Cargo.lock, rust-toolchain.toml
- crates/vectr-core/ — engine: parse, validate, resolve, style, compile, export, fonts
- crates/vectr-cli/ — binary vectr
- crates/vectr-mcp/ — binary vectr-mcp (Phase 3)
- crates/vectr-eval/ — binary vectr-eval (Phase 4)
- schema/ — published JSON Schema artifacts
- assets/fonts/ — Inter + Noto Sans + SIL OFL licence texts
- skills/vectr/ — agent skill and authoring guide (Phase 3)
- corpus/ — evaluation corpus (Phase 4)
- tests/, fixtures/ — integration and acceptance tests, test scenes
- docs/ — docs submodule (read-only)
- .delivery/ — task routes (gitignored)
- scratch/ — per-task scratch (gitignored)

## Contracts
CONTRACTS.md — C-001 scene document, C-002 library API, C-003 render model, C-004 CLI, C-005 MCP server.

## Token map
Vectr has no product UI, so there are no product-level design tokens. Scene palettes (schema.md, "Palette") are project data authored by the user.

## Assets
ASSETS.md — A-001 Inter, A-002 Noto Sans, A-003 SIL OFL licence texts.

## Conventions
- Rust stable, pinned; cargo workspace; cargo fmt and cargo clippy -D warnings clean.
- Determinism: ordered collections, no unseeded randomness; procedural generation requires a seed (FEAT-006).
- Commit messages cite durable IDs only (FEAT-###, C-###); never a task id, route filename, or .delivery path.
- Never write under docs/. Engineers commit to the shared dev branch; the lead gates main.
- Diagnostics are structured, located, and distinguish errors from warnings.

## Behavior rules
- Local only: no telemetry and no server; the only network access is the user-configured model call in the evaluation harness (FEAT-023).
- Deterministic output: identical input and seed produce byte-identical results (NFR-010).
- No silent failures and no partial output; a failed step stops the pipeline (NFR-011).
- Untrusted scene parsing: bounded size, no external entities, no file or network access from a scene (NFR-021).
- Emitted SVG is inert: no script, event handler, or foreign content (NFR-023).
- The MCP server binds locally; filesystem scope widens only on explicit opt-in (NFR-024).
- Fonts are SIL OFL only; never bundle or redistribute commercial fonts (NFR-040).

## Task board

### Completed phases
| Phase | Closed | Notes |
|---|---|---|
| Phase 1 — First Graphic | 2026-10-08 | FEAT-001, FEAT-002, FEAT-003, FEAT-004, FEAT-005, FEAT-011, FEAT-012, FEAT-013, FEAT-016, FEAT-024 shipped; C-001–C-004 implemented; A-001–A-004 sourced. |
| Phase 2 — Style Core | 2026-10-08 | FEAT-027, FEAT-007, FEAT-008, FEAT-009, FEAT-010 shipped; the element paint model unified with linear/radial gradients; recipe selection wired through the CLI; C-001–C-003 re-implemented at revision 5. |
| Phase 3 — Any Model Can Author | 2026-10-08 | FEAT-017, FEAT-018, FEAT-019, FEAT-020 shipped; validated alpha colour model (FEAT-005) and the shading-request warning (FEAT-007); scope extended to very complex illustrations (FEAT-003, FEAT-011) with complex-scene coverage; multi-scene projects with identifier addressing and a default scene, matched by the MCP surface (FEAT-016, FEAT-019); CLI and MCP share one project loader and one authoring guide; crates self-contained for packaging, both binaries and the skill distributed; released as 0.1.0-pre.1 on crates.io and GitHub Releases (checksummed and signed); the gate covers the acceptance crate's fmt/lints and a determinism check; C-001–C-005 implemented. |

### Active phase: Phase 4 — Reuse & Incremental Authoring
| Task | Feature | Owner | Status | Contract |
|---|---|---|---|---|
| T-072 | FEAT-030 | backend-engineer | Done | C-001, C-002, C-003 |
| T-073 | FEAT-031 | backend-engineer | Backlog | C-002, C-004 |
| T-074 | FEAT-031 | ai-engineer | Backlog | C-005 |
| T-075 | FEAT-029 | ai-engineer | Backlog | C-002, C-004, C-005 |

## Decisions log
| # | Decision | Rationale | By |
|---|---|---|---|
| D-001 | Build Vectr as a Rust cargo workspace: vectr-core library, vectr CLI, vectr-mcp server. | Deterministic single-binary core; performance for 50k-element scenes; memory-safe untrusted parsing (NFR-021); in-process embedding (FEAT-021); cross-platform CI. | user |
| D-002 | Scene documents are strict JSON. | Aligns the authoring format with schema.md's JSON-Schema model; enables constrained decoding, the cheapest route to NFR-030's >=95% compile bar; keeps risk R-005 off the critical path. A custom markup can be added later as an alternate syntax over the same scene model. | user |
| D-003 | docs/ is a git submodule pinned to a local bare remote. | Isolates the docs repository from the product repository; a real remote can replace the local URL later without changing the submodule layout. | user |
| D-004 | Bundle Inter as the default sans with Noto Sans as glyph fallback. | Satisfies NFR-040 (SIL OFL only, no commercial fonts) while covering wordmarks, labels and non-Latin glyphs. | user |
| D-005 | The evaluation harness targets multiple pinned providers (OpenAI, Anthropic, Google) plus an OpenCode adapter. | Meets NFR-030's >=3-model bar and reaches many more models through OpenCode. | user |
| D-006 | Distribute via crates.io and signed GitHub Release binaries with checksums per OS. | Matches dependency D4 and NFR-025 release integrity. | user |
| D-007 | Rasterizer is resvg/tiny-skia behind the export layer; geometry via lyon + i_overlay; text via rustybuzz + ttf-parser. | Pure-Rust, deterministic and free of native system dependencies; swappable behind the export layer per risk R-008. | lead |
| D-008 | CLI exit codes: 0 success, 1 invalid scene, 2 usage/input, 3 compile failure, 4 missing export dependency, 5 output I/O failure. | FEAT-016 requires exit codes to distinguish failure classes but the docs do not enumerate them. | lead |
| D-009 | Project layout: vectr.project.json at the root; scenes/, palettes/, strokes/, recipes/ for entities; dist/ for output; default recipe flat. | schema.md models each entity as a separate document referenced by id, and FEAT-007 names flat the default; the docs leave the on-disk layout unnamed. | lead |
| D-010 | Vectr ships under MIT OR Apache-2.0. | The docs state no product licence; crates.io publishing (D-006) and NFR-041 require an SPDX licence. Dual permissive matches the Rust ecosystem and the docs' free/open intent (R-013). | user |
| D-011 | The initial scene format version is 0.1. | The docs never name one; the language is pre-1.0 while the recipes (Phase 2) and agent surface (Phase 3) settle, avoiding release.md's one-way-door rule for schema changes (risk R-005). | user |
| D-012 | The parser bounds a scene document at 64 MiB and refuses larger input with a defined size diagnostic. | NFR-021 requires bounded input size but the docs give no figure; 64 MiB covers the documented 50,000-element large scene while bounding memory, and refuses rather than truncating. | user |
| D-013 | Style assets reach the compiler through a caller-supplied context: `compile(&Scene)` stays the no-style entry point and `compile_with_style(&Scene, &StyleContext)` resolves the palette and stroke profiles. | schema.md models Palette, StrokeProfile and StyleRecipe as separate documents referenced by id, so the library cannot load them itself without filesystem access; a caller-supplied context keeps the library deterministic and free of file or network access (NFR-010, NFR-021). | lead |
| D-014 | The render model is serializable to camelCase JSON, and `vectr compile --out` writes it. | C-004 reserves `compile --out`, user-flow's compile stage yields a render model, and the MCP compile tool returns it; the model must cross the process/tool boundary as data. | lead |
| D-015 | A stroke's colour is a palette token the element names (`strokeToken`), parallel to fill; `StrokeProfile` carries geometry only, and a stroke needs both a profile and a colour token. | The docs settled no stroke-colour source, so the user directed the gap to product-shaper; the spec now keeps every colour in the palette (FEAT-007) with no silent fallback. | user |
| D-016 | The render model retains element names and group nesting: each node carries its ancestor group chain, and the SVG exporter emits nested named groups. | FEAT-011 and FEAT-012 require named-group preservation in Phase 1; the flat paint-order node list keeps the seam simple while carrying the structure the exporter needs. FEAT-026 is the accessible-metadata layer above it. | lead |
| D-017 | A text element that names no font resolves to the caller-supplied font asset with id `default`, loaded from the bundled open-licensed font. | The docs say text with no font uses the default open-licensed font but name no id; the convention gives the compiler and the font manager a stable key, and the caller decides which font fills it. | lead |
| D-018 | A font asset with id `fallback` supplies the fallback font for glyphs the resolved font lacks, and the compiler carries it into the render model's font table. | FEAT-024 requires a missing glyph to be substituted from a fallback font rather than drawn as a blank box, but the docs name no fallback id and no text node references it; the convention makes the fallback reachable. | lead |
| D-019 | Migrate the text stack from rustybuzz + ttf-parser to harfrust + skrifa. | RustSec flags rustybuzz (RUSTSEC-2026-0206) and ttf-parser (RUSTSEC-2026-0192) unmaintained with no safe upgrade; harfrust and skrifa are the named maintained successors, keeping NFR-020's dependency gate green without ignores. Supersedes the text portion of D-007. | user |
| D-020 | The project layout includes an `assets/` store of Asset documents (id, kind, path, license) for user-supplied fonts, scaffolded by `vectr init`. | D-009 named no home for a user-supplied font, which FEAT-024 requires; the Asset entity already models a font by path and licence. Extends D-009. | lead |
| D-021 | Element paint is unified: `fill` and `stroke` each carry a `{kind, ref}` paint, and a new Gradient entity (linear or radial, palette-token stops) supplies gradient paints; texture is deferred to raster-assisted layers (FEAT-015). | The flat recipe's gradient and texture edge cases had no expressible trigger, so the user directed the gap to product-shaper, which made a gradient request expressible and removed the ambiguity of separate fill and stroke fields. Extends D-013 and D-015. | user |
| D-022 | The scene format version moves to 0.2. | The paint-model change is a breaking change to the scene document; the pre-1.0 version signals it. Supersedes D-011's 0.1. | lead |
| D-023 | The project layout gains a `gradients/` directory of Gradient documents, scaffolded by `vectr init`. | Gradients are project documents referenced by paint, parallel to palettes and strokes; the docs name no on-disk layout. Extends D-009. | lead |
| D-024 | In a line-art scene, a stroke profile whose width is 0 takes the recipe's `strokeWeight`; a positive profile width is the explicit weight the recipe honors. | FEAT-008 requires the recipe to fix a consistent weight while honoring explicitly varied weights, but the scene language has no separate per-element weight field and a profile's width is required. Width 0 is the only channel that reads as "no explicit weight". | lead |
| D-025 | The minimum renderable stroke weight is 0.05 scene units; a line-art stroke below it is clamped and warned. | FEAT-008 requires a minimum renderable unit but the docs name no figure. | lead |
| D-026 | The minimum usable grid spacing is 0.05 scene units; a geometric recipe with a finer grid is reported as a performance warning and the grid is not applied. | FEAT-009 requires a performance warning for a grid finer than the renderable resolution but the docs name no figure; mirrors D-025. | lead |
| D-027 | The isometric recipe snaps to the lattice its two 30-degree axes span and orders siblings back-to-front by isometric grid row, ties broken by document order; geometry is projected only through an explicit projection element. | FEAT-010 requires axis alignment and depth ordering but names no grid geometry or depth rule, and the schema has no flag to mark a billboard. | lead |
| D-028 | A scene `recipeId` or project `defaultRecipeId` that does not resolve is a missing project asset (exit 2, `E_PROJECT_ASSET`), mirroring a missing palette. | The docs classify no error for an unresolved recipe reference; the project loader already treats other missing style documents this way. | lead |
| D-029 | A colour value is a validated capability: any format SVG supports, optionally carrying alpha, so fill and stroke carry independent transparency; a value that is not such a colour is a located error and no output is produced. | The user decided alpha is worth having as a supported capability; the docs now define the colour model (FEAT-005, FEAT-018) where values were previously an unchecked pass-through. | user |
| D-030 | Single-layer shading is a real feature (FEAT-028) scheduled in Phase 6; until it ships, a recipe that requests shading it does not apply reports a warning rather than rendering a silent flat result. | The user chose to build the look rather than retract it; the flat recipe's promise is reconciled to point at the new feature. | user |
| D-031 | The MCP server serves over stdio by default, with an opt-in loopback HTTP listener behind an explicit bind flag, rather than binding a port by default. | MCP's canonical local transport is a subprocess over stdio, which is strictly more local than a listening socket and matches NFR-024's local-only intent. Clarifies C-005. | lead |
| D-032 | A project's scene directory holds one document per scene named for the scene's identifier (`scenes/<id>.json`); a project names its default scene by that identifier, and the project root holds no scene document. | FEAT-016 addresses a scene by its identifier; the docs name the scene directory but not its file naming, so the identifier is the file name. Extends D-009. | lead |
| D-033 | The first release, `0.1.0-pre.1`, is published to crates.io and GitHub Releases; release artifacts are checksummed and signed with a project GPG key over the checksums. | GitHub's native build attestations are unavailable on a private repository on a Free plan, so a detached GPG signature over the published checksums provides the release integrity NFR-025 requires on any plan. | lead |
| D-034 | A reusable part is a separate, project-scoped Definition entity, and a scene is not instanceable (no scene-in-scene). | A scene is a deliverable (canvas, palette, recipe) while a part must take the placing scene's look; separating them keeps "restyle by one token" true for reused parts and avoids a scene-cycle surface. Confirms the split the user chose. | user |
| D-035 | A parameter is referenced inside a definition by the object `{param: "<name>"}` in a field whose type matches the parameter; an instance override names one element of the placed definition by identifier. | The docs left both unsettled; the tagged-object form is unambiguous across field types and matches the model's existing tagged-object idiom, and targeting an element keeps multi-colour parts overridable. | lead |
| D-036 | A project's reusable definitions live in a `definitions/` directory, one document per definition named for its identifier (`definitions/<id>.json`), scaffolded by `vectr init`. | Definitions are project documents referenced by identifier, parallel to scenes and palettes; the docs name the directory but not its file naming. Extends D-009. | lead |

## Definition of Done
- Every acceptance criterion in the task's FEAT file is met, including its edge cases and failure states.
- Unit tests for the unit pass; `scripts/check.sh` (cargo fmt --check, clippy -D warnings, workspace build, workspace tests, and the acceptance suite) is clean.
- Determinism holds: repeated runs on the same input produce byte-identical output (NFR-010).
- Every failure is reported with a location and a non-zero exit, with no partial output (NFR-011).
- The task's contract is implemented and marked Implemented in CONTRACTS.md.
- No writes under docs/; commit messages cite FEAT-### or C-### only.
- The engineer reports the task done; QA verifies at the phase gate.

## Key commands
- cargo build --workspace
- cargo test --workspace
- cargo test --manifest-path tests/acceptance/Cargo.toml --locked
- cargo fmt --all -- --check
- cargo clippy --workspace --all-targets -- -D warnings
- cargo run -p vectr-cli -- validate <scene-id>
- cargo run -p vectr-cli -- export <scene-id> --format svg --out dist/scene.svg
- bash scripts/check.sh
- bash scripts/deny.sh
- bash scripts/font-inventory.sh
- bash scripts/package.sh <target-triple> [out-dir]
- bash scripts/publish-crates.sh
