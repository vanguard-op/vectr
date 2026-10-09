//! The versioned evaluation corpus (FEAT-023, schema.md).
//!
//! A corpus is a directory holding its manifest and its prompts. The manifest
//! is the schema's `EvaluationCorpus` (`corpus.json`); each prompt is the
//! schema's `Prompt`, read from `prompts/*.json` and ordered by its `order`
//! field, ties broken by identifier, so a run is deterministic (NFR-010). A
//! corpus is versioned because comparisons are only valid within one version
//! (FEAT-023 edge case).
//!
//! Two optional prompt fields carry the coverage metadata FEAT-023 requires the
//! harness to check: `complexity` marks a prompt on the complex end of the
//! range, and `qualities` names the hard-end qualities it exercises (FEAT-032,
//! NFR-031). The published `Prompt` entity does not carry them, so a corpus
//! that omits them reports the coverage omission rather than guessing.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The corpus manifest's file name at the corpus root.
pub const CORPUS_FILE: &str = "corpus.json";

/// The folder holding the prompt documents.
pub const PROMPT_DIR: &str = "prompts";

/// The schema's `EvaluationCorpus`: a versioned set of authoring prompts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusManifest {
    /// Stable identifier for the corpus.
    pub id: String,
    /// Version of the corpus; comparisons are only valid within one version.
    pub version: String,
    /// What the corpus covers.
    pub description: String,
}

/// Where a prompt sits in the product's complexity range (FEAT-023).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Complexity {
    /// A simple mark.
    Simple,
    /// A moderately detailed graphic.
    #[default]
    Moderate,
    /// The complex end of the range: a very complex illustration.
    Complex,
}

/// A hard-end authoring quality the complex end of the corpus must exercise
/// (FEAT-032, NFR-031), plus the reusable-parts coverage FEAT-023 names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    /// Depth and perspective.
    Depth,
    /// Parts placed relative to one another.
    RelativePlacement,
    /// Parts joined at shared anchors.
    SharedAnchors,
    /// A named subject researched and authored accurately.
    SubjectAccuracy,
    /// An illustration built up from reusable part definitions.
    ReusableParts,
}

impl Quality {
    /// The four hard-end qualities NFR-031 scores.
    pub const HARD_END: [Quality; 4] = [
        Quality::Depth,
        Quality::RelativePlacement,
        Quality::SharedAnchors,
        Quality::SubjectAccuracy,
    ];

    /// The stable key the judge and the run record use.
    pub fn key(self) -> &'static str {
        match self {
            Quality::Depth => "depth",
            Quality::RelativePlacement => "relative_placement",
            Quality::SharedAnchors => "shared_anchors",
            Quality::SubjectAccuracy => "subject_accuracy",
            Quality::ReusableParts => "reusable_parts",
        }
    }

    /// The human-readable name used in coverage findings.
    pub fn label(self) -> &'static str {
        match self {
            Quality::Depth => "depth and perspective",
            Quality::RelativePlacement => "relative placement",
            Quality::SharedAnchors => "parts joined at shared anchors",
            Quality::SubjectAccuracy => "subject accuracy",
            Quality::ReusableParts => "illustrations built up from reusable parts",
        }
    }
}

/// The schema's `Prompt`: one authoring task in an evaluation corpus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompt {
    /// Stable identifier for the prompt.
    pub id: String,
    /// References the `EvaluationCorpus` this prompt belongs to.
    pub corpus_id: String,
    /// The graphic the prompt asks for.
    pub intent: String,
    /// The natural-language request given to the authoring model.
    pub text: String,
    /// Order of the prompt in the corpus.
    pub order: i64,
    /// Where the prompt sits in the complexity range.
    #[serde(default)]
    pub complexity: Complexity,
    /// The hard-end qualities the prompt exercises.
    #[serde(default)]
    pub qualities: Vec<Quality>,
}

/// A loaded, ordered corpus.
#[derive(Debug, Clone)]
pub struct Corpus {
    /// The corpus manifest.
    pub manifest: CorpusManifest,
    /// The prompts, ordered by `order` then identifier.
    pub prompts: Vec<Prompt>,
}

/// The coverage findings FEAT-023 requires a corpus to be checked against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    /// The coverage requirements the corpus does not meet, each naming the
    /// omission; empty when the corpus meets every requirement.
    pub omissions: Vec<String>,
}

impl Coverage {
    /// Whether the corpus meets every coverage requirement.
    pub fn satisfied(&self) -> bool {
        self.omissions.is_empty()
    }
}

/// A corpus could not be read or does not conform to its schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusError(pub String);

impl std::fmt::Display for CorpusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CorpusError {}

/// Loads and orders a corpus from a directory.
///
/// The manifest is `corpus.json` at the root; prompts live under `prompts/`.
/// For a corpus that keeps its prompts at the root, every JSON document at the
/// root other than the manifest is read as a prompt. A document that cannot be
/// read or parsed is a hard error naming the file, never a silently dropped
/// prompt (FEAT-023, NFR-011).
pub fn load(dir: &Path) -> Result<Corpus, CorpusError> {
    let manifest_path = dir.join(CORPUS_FILE);
    let manifest: CorpusManifest = read_json(&manifest_path, "corpus manifest")?;

    let mut prompts = Vec::new();
    let prompt_dir = dir.join(PROMPT_DIR);
    let sources = if prompt_dir.is_dir() {
        json_files(&prompt_dir)?
    } else {
        json_files(dir)?
            .into_iter()
            .filter(|path| path != &manifest_path)
            .collect()
    };
    if sources.is_empty() {
        return Err(CorpusError(format!(
            "corpus `{}` holds no prompts",
            manifest.id
        )));
    }

    for path in sources {
        let prompt: Prompt = read_json(&path, "prompt")?;
        if prompt.corpus_id != manifest.id {
            return Err(CorpusError(format!(
                "prompt `{}` names corpus `{}`, but the manifest is `{}`",
                prompt.id, prompt.corpus_id, manifest.id
            )));
        }
        prompts.push(prompt);
    }

    let mut seen = BTreeSet::new();
    for prompt in &prompts {
        if !seen.insert(prompt.id.clone()) {
            return Err(CorpusError(format!(
                "duplicate prompt identifier `{}`",
                prompt.id
            )));
        }
    }

    prompts.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));

    Ok(Corpus { manifest, prompts })
}

/// Checks a corpus against FEAT-023's coverage requirements.
///
/// A corpus that omits the complex end of the range, omits a hard-end quality,
/// or carries no illustration built up from reusable parts does not meet its
/// coverage requirement, and each omission is reported (FEAT-023).
pub fn coverage(corpus: &Corpus) -> Coverage {
    let mut omissions = Vec::new();

    let complex: Vec<&Prompt> = corpus
        .prompts
        .iter()
        .filter(|prompt| prompt.complexity == Complexity::Complex)
        .collect();
    if complex.is_empty() {
        omissions.push(
            "the corpus omits the complex end of the range: no prompt is marked complexity `complex`"
                .to_string(),
        );
    }

    for quality in Quality::HARD_END {
        let covered = complex
            .iter()
            .any(|prompt| prompt.qualities.contains(&quality));
        if !covered {
            omissions.push(format!(
                "the corpus omits the `{}` quality: no complex-end prompt exercises it",
                quality.label()
            ));
        }
    }

    let reusable = corpus
        .prompts
        .iter()
        .any(|prompt| prompt.qualities.contains(&Quality::ReusableParts));
    if !reusable {
        omissions.push(
            "the corpus omits illustrations built up from reusable parts: no prompt exercises `reusable_parts`"
                .to_string(),
        );
    }

    Coverage { omissions }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path, kind: &str) -> Result<T, CorpusError> {
    let source = fs::read_to_string(path).map_err(|error| {
        CorpusError(format!("cannot read {kind} `{}`: {error}", path.display()))
    })?;
    serde_json::from_str(&source)
        .map_err(|error| CorpusError(format!("{kind} `{}` is not valid: {error}", path.display())))
}

fn json_files(dir: &Path) -> Result<Vec<PathBuf>, CorpusError> {
    let entries = fs::read_dir(dir)
        .map_err(|error| CorpusError(format!("cannot read corpus `{}`: {error}", dir.display())))?;
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(std::ffi::OsStr::to_str)
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        })
        .collect();
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "vectr-eval-corpus-{}-{label}-{unique}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("creates the temporary directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(dir: &Path, name: &str, text: &str) {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().expect("a parent")).expect("creates the parent");
        fs::write(path, text).expect("writes the file");
    }

    fn manifest(id: &str) -> String {
        format!(r#"{{"id":"{id}","version":"1.0.0","description":"coverage"}}"#)
    }

    fn prompt(id: &str, order: i64, extra: &str) -> String {
        format!(
            r#"{{"id":"{id}","corpusId":"c","intent":"an intent","text":"a request","order":{order}{extra}}}"#
        )
    }

    #[test]
    fn loads_prompts_in_order_and_checks_coverage() {
        let dir = TempDir::new("load");
        write(dir.path(), "corpus.json", &manifest("c"));
        write(
            dir.path(),
            "prompts/b.json",
            &prompt(
                "b",
                1,
                r#","complexity":"complex","qualities":["depth","relative_placement","shared_anchors","subject_accuracy","reusable_parts"]"#,
            ),
        );
        write(dir.path(), "prompts/a.json", &prompt("a", 0, ""));

        let corpus = load(dir.path()).expect("loads");
        assert_eq!(corpus.manifest.version, "1.0.0");
        assert_eq!(
            corpus
                .prompts
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(corpus.prompts[0].complexity, Complexity::Moderate);
        assert!(coverage(&corpus).satisfied());
    }

    #[test]
    fn reports_every_coverage_omission() {
        let dir = TempDir::new("coverage");
        write(dir.path(), "corpus.json", &manifest("c"));
        write(dir.path(), "prompts/a.json", &prompt("a", 0, ""));

        let corpus = load(dir.path()).expect("loads");
        let coverage = coverage(&corpus);
        assert!(!coverage.satisfied());
        assert_eq!(coverage.omissions.len(), 6, "{:?}", coverage.omissions);
        assert!(coverage.omissions[0].contains("complex end"));
    }

    #[test]
    fn a_prompt_naming_the_wrong_corpus_is_refused() {
        let dir = TempDir::new("wrong-corpus");
        write(dir.path(), "corpus.json", &manifest("c"));
        write(
            dir.path(),
            "prompts/a.json",
            r#"{"id":"a","corpusId":"other","intent":"i","text":"t","order":0}"#,
        );

        let error = load(dir.path()).expect_err("refused");
        assert!(error.0.contains("other"), "{}", error.0);
    }

    #[test]
    fn duplicate_prompt_identifiers_are_refused() {
        let dir = TempDir::new("duplicate");
        write(dir.path(), "corpus.json", &manifest("c"));
        write(dir.path(), "prompts/a.json", &prompt("a", 0, ""));
        write(dir.path(), "prompts/b.json", &prompt("a", 1, ""));

        let error = load(dir.path()).expect_err("refused");
        assert!(error.0.contains("duplicate"), "{}", error.0);
    }

    #[test]
    fn a_missing_manifest_is_refused() {
        let dir = TempDir::new("missing-manifest");
        let error = load(dir.path()).expect_err("refused");
        assert!(error.0.contains("corpus manifest"), "{}", error.0);
    }
}
