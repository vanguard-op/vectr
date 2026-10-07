# Delivery

## Source of truth
docs/Vectr/ — git submodule, pinned at 4dda288, remote /tmp/opencode/vectr-docs.git. The docs are authoritative for what to build; this file tracks state only and never restates the spec.

## Team & file ownership
| Member | Owns |
|---|---|
| lead | DELIVERY.md, CONTRACTS.md, ASSETS.md, .delivery/**, Cargo.lock |
| infra-engineer | Cargo.toml, rust-toolchain.toml, .gitignore, .github/**, scripts/**, assets/fonts/** |
| backend-engineer | crates/vectr-core/**, crates/vectr-cli/**, schema/** |
| ai-engineer | crates/vectr-mcp/**, crates/vectr-eval/**, skills/** |
| qa-engineer | tests/**, fixtures/**, corpus/** |
| frontend-engineer | (none — Vectr has no UI) |
| visual-artist | (none — the docs declare no imagery) |

Scratch: scratch/<task>/ — gitignored, private to the task's owner.

## Stack
Rust (stable, pinned by rust-toolchain.toml), cargo workspace. serde + schemars for the scene model and the generated JSON Schema; lyon + i_overlay for path geometry, boolean and offset; rustybuzz + ttf-parser for text and outlining; resvg/tiny-skia as the rasterizer behind the export layer; an in-crate vector PDF emitter. Evaluation harness: multiple pinned providers (OpenAI, Anthropic, Google) plus an OpenCode adapter. Distributed as crates.io packages and signed GitHub Release binaries with checksums for macOS, Linux and Windows.

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
CONTRACTS.md — C-001 scene document, C-002 library API, C-003 render model, C-004 CLI.

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

### Active phase: Phase 1 — First Graphic
| Task | Feature | Owner | Status | Contract |
|---|---|---|---|---|
| T-001 | — (foundation) | infra-engineer | Done | — |
| T-002 | FEAT-001 | backend-engineer | Done | C-001 |
| T-003 | FEAT-002 | backend-engineer | Done | C-003 |
| T-004 | FEAT-003 | backend-engineer | Done | C-003 |
| T-005 | FEAT-005 | backend-engineer | Ready | C-001 |
| T-006 | FEAT-004 | backend-engineer | Ready | C-003 |
| T-007 | FEAT-011 | backend-engineer | Backlog | C-002, C-003 |
| T-008 | FEAT-012 | backend-engineer | Backlog | C-002 |
| T-009 | FEAT-013 | backend-engineer | Backlog | C-002 |
| T-010 | FEAT-024 | backend-engineer | Backlog | C-002 |
| T-011 | FEAT-016 | backend-engineer | Backlog | C-004 |
| T-012 | — (foundation) | infra-engineer | Backlog | — |
| T-013 | FEAT-002, FEAT-003 | product-shaper | In Progress | C-001 |

## Decisions log
| # | Decision | Rationale | By |
|---|---|---|---|
| D-001 | Build Vectr as a Rust cargo workspace: vectr-core library, vectr CLI, vectr-mcp server. | Deterministic single-binary core; performance for 50k-element scenes; memory-safe untrusted parsing (NFR-021); in-process embedding (FEAT-021); cross-platform CI. | user |
| D-002 | Scene documents are strict JSON. | Aligns the authoring format with schema.md's JSON-Schema model; enables constrained decoding, the cheapest route to NFR-030's >=95% compile bar; keeps risk R-005 off the critical path. A custom markup can be added later as an alternate syntax over the same scene model. | user |
| D-003 | docs/ is a git submodule pinned to a local bare remote (/tmp/opencode/vectr-docs.git). | Isolates the docs repository from the product repository; a real remote can replace the local URL later without changing the submodule layout. | user |
| D-004 | Bundle Inter as the default sans with Noto Sans as glyph fallback. | Satisfies NFR-040 (SIL OFL only, no commercial fonts) while covering wordmarks, labels and non-Latin glyphs. | user |
| D-005 | The evaluation harness targets multiple pinned providers (OpenAI, Anthropic, Google) plus an OpenCode adapter. | Meets NFR-030's >=3-model bar and reaches many more models through OpenCode. | user |
| D-006 | Distribute via crates.io and signed GitHub Release binaries with checksums per OS. | Matches dependency D4 and NFR-025 release integrity. | user |
| D-007 | Rasterizer is resvg/tiny-skia behind the export layer; geometry via lyon + i_overlay; text via rustybuzz + ttf-parser. | Pure-Rust, deterministic and free of native system dependencies; swappable behind the export layer per risk R-008. | lead |
| D-008 | CLI exit codes: 0 success, 1 invalid scene, 2 usage/input, 3 compile failure, 4 missing export dependency, 5 output I/O failure. | FEAT-016 requires exit codes to distinguish failure classes but the docs do not enumerate them. | lead |
| D-009 | Project layout: vectr.project.json at the root; scenes/, palettes/, strokes/, recipes/ for entities; dist/ for output; default recipe flat. | schema.md models each entity as a separate document referenced by id, and FEAT-007 names flat the default; the docs leave the on-disk layout unnamed. | lead |
| D-010 | Vectr ships under MIT OR Apache-2.0. | The docs state no product licence; crates.io publishing (D-006) and NFR-041 require an SPDX licence. Dual permissive matches the Rust ecosystem and the docs' free/open intent (R-013). | user |
| D-011 | The initial scene format version is 0.1. | The docs never name one; the language is pre-1.0 while the recipes (Phase 2) and agent surface (Phase 3) settle, avoiding release.md's one-way-door rule for schema changes (risk R-005). | user |
| D-012 | The parser bounds a scene document at 64 MiB and refuses larger input with a defined size diagnostic. | NFR-021 requires bounded input size but the docs give no figure; 64 MiB covers the documented 50,000-element large scene while bounding memory, and refuses rather than truncating. | user |

## Definition of Done
- Every acceptance criterion in the task's FEAT file is met, including its edge cases and failure states.
- Unit tests for the unit pass; cargo test --workspace, cargo fmt --check, and cargo clippy -D warnings are clean.
- Determinism holds: repeated runs on the same input produce byte-identical output (NFR-010).
- Every failure is reported with a location and a non-zero exit, with no partial output (NFR-011).
- The task's contract is implemented and marked Implemented in CONTRACTS.md.
- No writes under docs/; commit messages cite FEAT-### or C-### only.
- The engineer reports the task done; QA verifies at the phase gate.

## Key commands
- cargo build --workspace
- cargo test --workspace
- cargo fmt --all -- --check
- cargo clippy --workspace --all-targets -- -D warnings
- cargo run -p vectr-cli -- validate fixtures/scene.json
- cargo run -p vectr-cli -- export fixtures/scene.json --format svg --out dist/
