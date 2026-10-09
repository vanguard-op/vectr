# When authoring fails

Read this when validation or compilation reports a finding, or a render comes
back wrong at the structural level. A diagnostic names the problem and its
location — severity, code, message, and a JSON path or element id. Read the
whole list, correct the scene, and re-validate.

## Validate, then compile

Name the scene by its identifier; the document is `scenes/<id>.json`:

```sh
vectr validate habit-logo
vectr compile habit-logo --check
```

Omitting the identifier uses the project's default scene, so `vectr validate`
alone validates that one. `validate` checks the scene against the contract and
the project's references — a palette token, stroke profile, gradient, or font
that does not resolve is an error naming the element. `compile --check` confirms
the scene resolves to a render model without writing anything. On success both
exit `0` and print nothing; a broken scene exits non-zero with diagnostics.

Use `vectr validate --json habit-logo` (or the MCP `validate` result) for a
machine-readable array:

```json
[{"severity":"error","code":"E_SCHEMA","message":"`opacity` must be between 0 and 1","location":{"jsonPath":"/elements/0/opacity"}}]
```

The MCP tools return the same diagnostics in the tool body under `diagnostics`,
with `isError: true` on failure: call `validate` with `{"scene": "habit-logo"}`,
or `{"draft": {...}}` to validate an inline document. Exit codes: `0` success,
`1` invalid scene, `2` usage or unreadable input, `3` compilation failure,
`4` export dependency missing, `5` output I/O failure.

An identifier no scene document provides, a project that names no default when
the scene is omitted, and a `defaultSceneId` that resolves to no document each
report `E_SCENE` and exit `2`, naming the scene or the missing default.

## The common findings

- `E_SCHEMA` — a missing required field, a wrong type, or an out-of-range value;
  the `jsonPath` points at it. The most common cause is a partial `transform` or
  a missing `geometry`.
- `E_PARSE` — the document is not valid JSON. Reparse the file.
- `E_FORMAT_VERSION` — `formatVersion` is not `"0.2"`; set it and retry.
- `E_INVALID_COLOR` — a canvas background or export background is not a colour
  SVG supports and not `transparent`.
- `E_PROJECT_ASSET` — a referenced palette, recipe, gradient, or stroke
  document is missing, unreadable, or declares a different `id`; check the
  folder and the `id`.
- `E_SCENE` — the scene identifier the command named has no document at
  `scenes/<id>.json`, the project names no default scene, or its
  `defaultSceneId` resolves to no document; run the command from inside the
  project and check the identifier.
- `E_MALFORMED` — an MCP call named both a `scene` identifier and an inline
  `draft`; the two are mutually exclusive, so name one.
- `E_SCHEMA_TYPE` — a `schema --type` name does not exist; pick one of the
  listed similar names.
- `E_CYCLE` — elements reference each other as parents; break the loop.
- `E_DEFINITION` — an `instance` references a definition that does not resolve;
  check `definitions/<id>.json` and the `definitionRef` id.
- `E_DEFINITION_CYCLE` — two definitions place each other, directly or through a
  chain; break the loop.
- `E_BINDING` — an instance binds a parameter the definition does not declare,
  or a parameter has neither a binding nor a default; fix the binding or give the
  parameter a default.
- `E_PART` — a part identifier resolves to no definition or element in the
  project; check the id and run the command from inside the project.
- `W_UNUSED_DEFINITION` — a definition no scene places renders nothing; place it
  or remove it. It is a warning, not an error.
- An undefined-token, undefined-stroke, undefined-gradient, or missing-font
  error — the element names something the project does not provide; add it or
  fix the reference.

## Retry once, then report

Correct the scene from the diagnostics and re-validate. **Retry once.** If the
scene still fails after that correction, report the failure together with its
diagnostics and produce no output. **Never export** from a scene that did not
validate and compile. If the authoring model is unavailable, stop and report
that authoring cannot proceed and produce no output; never emit a placeholder
asset.
