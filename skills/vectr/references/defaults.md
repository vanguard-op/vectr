# Defaults for an ambiguous request

Read this when the request leaves something open. Choose the documented default,
state it in one line, and proceed rather than stopping to ask:

| Open point | Default |
|---|---|
| Canvas size | 512×512 |
| Scene identifier | A short kebab-case name for the request (e.g. `habit-logo`); its file is `scenes/<id>.json` and commands address it by that id |
| Background | `transparent` |
| Scope of the graphic | When the request leaves it open, one simple mark — a primary shape plus an optional wordmark. A request that asks for a detailed or complex illustration is authored in full |
| Parts of a complex request | The default decomposition in the build-up method: background and sky; midground masses; repeating or reused objects; then foreground detail |
| Shape placement | Centred, with an even margin from each edge |
| Colours | A palette named for the request (`accent`, `ink`, `paper`) so it restyles |
| Text font | The bundled open-licensed sans (omit `fontId`) |
| Recipe | The project's `defaultRecipeId`; set `recipeId` only when the request names a look |
| Icon set | Every icon on the same canvas and style, each exported to its own file |

Keep a simple request's first version valid and renderable, then refine it toward
the request. A detailed or complex request is built up in verified parts (the
procedure's **Build a complex graphic up in verified parts**), not authored in
one pass and not simplified to a simple mark.
