# Vectr

Vectr turns a plain-text scene description into clean, editable vector graphics — SVG and PNG — entirely on your machine. It is a language plus a deterministic compiler: any model, or any person, can author a scene, and the same input always produces the same output.

- **Model-agnostic authoring.** The scene language is published as a JSON Schema, so any AI model can write it without having been trained on Vectr, and any tool can validate it.
- **Deterministic.** Identical input and seed produce byte-identical output.
- **Local only.** No server and no telemetry; the only network access is a model call you configure yourself.
- **Portable output.** SVG that opens in any editor, plus PNG. Open-licensed fonts only.

## Install

Vectr is in pre-release. Two channels:

**From crates.io** — pin the pre-release, because a bare `cargo install` will not match a pre-release version:

```sh
cargo install vectr-cli --version 0.1.0-pre.2   # the `vectr` command
cargo install vectr-mcp --version 0.1.0-pre.2   # the MCP server
```

**Signed binaries** — download `vectr` (and `vectr-mcp`) for your OS from the [releases page](https://github.com/vanguard-op/vectr/releases), then verify:

```sh
gpg --import vectr-signing-key.asc
gpg --verify SHA256SUMS.asc SHA256SUMS
sha256sum -c SHA256SUMS
```

## Quick start

```sh
vectr init my-project      # writes config, a starter scene, a palette, and AGENTS.md
cd my-project
# author scenes/<id>.json
vectr validate logo        # check a scene against the language contract
vectr compile logo         # compile it into the render model
vectr export logo --format svg --out dist/logo.svg
```

A scene is addressed by its identifier, and the project is found from the working directory. Omit the scene to use the project's default scene.

## The scene language

A project is a directory: `vectr.project.json` plus `scenes/`, `palettes/`, `strokes/`, `recipes/`, `gradients/`, `assets/`, and `dist/` for output. Each scene is a JSON document, and each entity is its own document referenced by identifier.

```jsonc
// palettes/brand.json
{ "id": "brand", "projectId": "project", "name": "Brand",
  "tokens": [ { "name": "accent", "value": "#e94560" } ] }
```

```jsonc
// scenes/logo.json
{
  "id": "logo", "projectId": "project", "name": "Logo", "formatVersion": "0.2",
  "canvas": { "width": 256, "height": 256, "background": "#ffffff" },
  "paletteId": "brand",
  "elements": [
    { "id": "card", "sceneId": "logo", "order": 0, "kind": "rect",
      "geometry": { "x": 32, "y": 32, "width": 192, "height": 192, "rx": 24, "ry": 24 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "fill": { "kind": "token", "ref": "accent" },
      "opacity": 1, "visible": true }
  ]
}
```

Elements compose by intent rather than by coordinates: groups and transforms, repetition and grids, boolean operations, placement along a path, outline offsetting, and projection helpers, related by constraints. Colours live in the palette as named tokens — in any format SVG supports, including alpha — so changing one token restyles every element that references it. A scene may apply a style recipe — **flat**, **line-art**, **geometric**, or **isometric** — to restyle the whole graphic at once.

## Commands

| Command | Purpose |
|---|---|
| `vectr init [dir]` | Scaffold a project |
| `vectr validate [<scene>]` | Check a scene against the language contract |
| `vectr compile [<scene>]` | Compile a scene into its render model |
| `vectr export [<scene>] --format svg\|png` | Export a scene |
| `vectr schema [--type <name>] [--compact]` | Print the language contract |

`vectr schema` is what makes the language model-agnostic: a model reads the contract and authors a valid scene without prior training.

## For agents

- **MCP server** — `vectr-mcp` exposes `validate`, `compile`, `render`, and `schema` as discoverable tools. It serves over stdio by default and opens a loopback listener only on explicit opt-in; it makes no network calls unless configured.
- **Agent skill** — `skills/vectr/` packages instructions and worked examples that teach a coding agent the author → validate → render → inspect → refine workflow. `vectr init` also writes the guide into a project as `AGENTS.md`.

## Status

Pre-release, `0.1.0-pre.2`. Shipped: the scene language and deterministic compiler, SVG and PNG export, the four version-one style recipes, gradients, schema discovery, scene validation, the MCP server, the agent skill, and reusable part definitions with part-scoped rendering. Next: quality and reach (PDF export, the embedded library, render-in-the-loop verification, the evaluation harness, icon sets, and accessibility), then single-layer shading.

## License

MIT OR Apache-2.0 (see `LICENSE-MIT` and `LICENSE-APACHE`). Bundled fonts are Inter and Noto Sans, both under the SIL Open Font License. The full specification lives under `docs/Vectr/`.
