# Vectr project

This project holds Vectr scenes: JSON documents that compile to a render model
and export as SVG or PNG. You author the scene; the `vectr` toolchain, or the
`vectr-mcp` server, validates it against the published schema, compiles it, and
renders it. The schema is the source of truth for every type and allowed value.

This file orients you and states this project's facts. It is not the authoring
procedure: the procedure, the worked examples, the rules the schema does not
state, the build-up method, the inspect-and-correct loop, and the defaults live
in the Vectr skill's references, read on demand.

## This project

- Identifier: `project`
- Default scene: `example` — its document is `scenes/example.json`
- Default palette: `brand` — `palettes/brand.json`
- Default recipe: `flat` — `recipes/flat.json`

A command names a scene by its identifier (the scene's `id`), and its document
is `scenes/<id>.json`; the other documents are found by the `id` they declare.
Omit the scene to use the project's default. The tokens, stroke profiles,
gradients, and definitions this project provides are read from its documents
through the tools; no summary of them is written here.

## The tools

- `vectr schema` — the published language contract; read the types you write.
- `vectr validate <scene>` — check a scene against the contract and the project.
- `vectr compile <scene> --check` — confirm every reference resolves.
- `vectr export <scene> --format svg|png [--out <file>]` — render or export.
- `vectr render <part>` — preview a reusable definition on its own.

An MCP host exposes the same operations as tools. A failure is reported with its
location and writes nothing.

## Where the depth lives

When you author, read the Vectr skill's `references/authoring-guide.md`: the
single source of the procedure, the worked examples, and the rules the schema
does not state. A complex request follows its **Author for depth and structure**
and **Build a complex graphic up in verified parts**. If the Vectr skill is not
available to you, author from the published schema (`vectr schema`); the skill
is not required.

This guide targets `vectr` 0.1.0-pre.2 and scene `formatVersion` `0.2`.
