# Defaults for an ambiguous request

Read this when the request leaves something open. Choose the documented default,
state it in one line, and proceed rather than stopping to ask:

| Open point | Default |
|---|---|
| Canvas size | 512×512 |
| Scene identifier | A short kebab-case name for the request (e.g. `habit-logo`); its file is `scenes/<id>.json` and commands address it by that id |
| Background | `transparent` |
| Scope of the graphic | When the request leaves it open, one simple mark — a primary shape plus an optional wordmark. A request that asks for a detailed or complex illustration is authored in full |
| Sections of the graphic | When the sections are not obvious, the documented default decomposition: background and sky; the midground masses; the repeating or reused objects; then the foreground detail |
| Shape placement | Centred, with an even margin from each edge |
| Colours | A palette named for the request (`accent`, `ink`, `paper`) so it restyles |
| Text font | The bundled open-licensed sans (omit `fontId`) |
| Recipe | The project's `defaultRecipeId`; set `recipeId` only when the request names a look |
| Icon set | Every icon on the same canvas and style, each exported to its own file |

Every request runs the same method: sketch the whole at low fidelity, then
refine its sections one at a time, integrating and verifying each. A request
that leaves the sections open still gets the default decomposition above rather
than a stall. Keep the sketch valid and renderable, then refine each section
toward the request; a detailed request is authored in full, never simplified to
a simple mark.

When one section cannot be verified until another exists, the dependency is
refined and verified first: author a section that depends on another after the
part it sits on, and record that order. The whole is never advanced past an
unverified section — a section that fails verification is corrected and
re-verified before it is integrated.
