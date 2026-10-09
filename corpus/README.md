# Evaluation corpus

The versioned authoring corpus for the FEAT-023 evaluation harness. It is the
canonical set of natural-language authoring prompts the harness runs through a
chosen authoring model and the toolchain, to measure the cross-model quality
bars (NFR-030, NFR-031). It is read-only input: the harness never writes here.

## Layout

| Path | Document | Purpose |
|---|---|---|
| `corpus.json` | `EvaluationCorpus` | The manifest: the corpus `id`, its `version`, and what it covers. |
| `prompts/<id>.json` | `Prompt` | One authoring task, with its coverage classification. |
| `README.md` | — | This file. |

The harness loads the manifest from `corpus.json` at the corpus root and the
prompts from `prompts/*.json`, ordered by each prompt's `order` field, ties
broken by identifier, so a run is deterministic (NFR-010). A corpus that keeps
its prompts at the root is also accepted (every JSON document at the root
other than the manifest is read as a prompt), but this corpus keeps them under
`prompts/`.

## Coverage

The published `Prompt` entity (`docs/Vectr/schema.md`) carries `id`,
`corpusId`, `intent`, `text`, `order`, `complexity`, and `qualities`. The two
coverage fields classify each prompt, because FEAT-023 requires the harness to
classify the complex end and to report a corpus that omits part of the range or
a hard-end quality:

| Field | Values | Meaning |
|---|---|---|
| `complexity` | `simple`, `moderate`, `complex` | Where the prompt sits in the range; `complex` is the complex end. |
| `qualities` | `depth`, `relative_placement`, `shared_anchors`, `subject_accuracy`, `reusable_parts` | The hard-end qualities the prompt exercises. |

`depth`, `relative_placement`, `shared_anchors`, and `subject_accuracy` are the
four hard-end qualities the guidance directs (FEAT-032) and NFR-031 scores.
`reusable_parts` is the FEAT-023 coverage that the complex end is built up
from reusable part definitions (FEAT-030). A prompt that carries none is
`moderate` by default, so a corpus that omits the classification is reported as
an omission rather than silently accepted.

The corpus holds 100 prompts. Every prompt is classified; none is
unclassified.

| `complexity` | Count | What it is |
|---|---|---|
| `simple` | 25 | A single mark or one primitive, from a dot to a simple icon. |
| `moderate` | 55 | An icon, emblem, or multi-part illustration of a scene. |
| `complex` | 20 | The complex end: a complex illustration built up from reusable parts, exercising the hard-end qualities. |

The complex end carries the hard-end qualities, each on several prompts:

| Quality | Complex-end prompts |
|---|---|
| `depth` | 12 |
| `relative_placement` | 13 |
| `shared_anchors` | 14 |
| `subject_accuracy` | 8 |
| `reusable_parts` (coverage) | 20 |

The bars measured are NFR-030 (compile success ≥ 95%, judged fidelity ≥ 80%
across at least three models) and NFR-031 (at least 80% of complex-end prompts
meet the hard-end rubric).

## Versioning

`corpus.json` carries the `version`; comparisons are only valid within one
version (FEAT-023 edge case). Any change to the prompt set, the range, or the
coverage classification is a corpus change and bumps the version:

- **Major** — a prompt is added, removed, reworded, re-classified, or a
  requirement changes; existing run records no longer compare.
- **Minor** — a non-scoring clarification that leaves every prompt's text,
  complexity, and qualities unchanged.

A run record names the corpus version it measured, so the harness flags a
comparison between runs of different versions as not meaningful rather than as
a regression.

## Prompt document shape

```json
{
  "id": "p-081-isometric-city-block",
  "corpusId": "vectr-authoring",
  "intent": "An isometric city block of several buildings.",
  "text": "Draw an isometric city block with several buildings shown in three dimensions, built from a reusable building part.",
  "order": 80,
  "complexity": "complex",
  "qualities": ["depth", "relative_placement", "reusable_parts", "shared_anchors"]
}
```

`intent` is the graphic the prompt asks for and is the reference the judge
scores fidelity against; `text` is the natural-language request given to the
authoring model. The guidance the model follows (the skill and authoring
guide, FEAT-020, FEAT-032) is supplied separately by the harness, so the
prompts stay plain requests.
