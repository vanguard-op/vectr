//! Shared test fixtures: scratch directories, a valid corpus, and recorded
//! model replies. Compiled only for tests.

use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A unique directory that removes itself when it drops.
pub struct TempDir(PathBuf);

impl TempDir {
    /// Creates an empty scratch directory labelled for the test.
    pub fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "vectr-eval-test-{}-{label}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("creates the temporary directory");
        Self(path)
    }

    /// The directory's path.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A corpus that meets FEAT-023's coverage requirement: a simple prompt and a
/// complex-end prompt exercising every hard-end quality and reusable parts.
pub fn corpus_dir() -> TempDir {
    let dir = TempDir::new("corpus");
    write(
        dir.path(),
        "corpus.json",
        r#"{"id":"c","version":"1.0.0","description":"a test corpus"}"#,
    );
    write(
        dir.path(),
        "prompts/p1.json",
        r#"{"id":"p1","corpusId":"c","intent":"a simple mark","text":"draw a dot","order":0,"complexity":"simple"}"#,
    );
    write(
        dir.path(),
        "prompts/p2.json",
        r#"{"id":"p2","corpusId":"c","intent":"a complex illustration","text":"draw a landscape","order":1,"complexity":"complex","qualities":["depth","relative_placement","shared_anchors","subject_accuracy","reusable_parts"]}"#,
    );
    dir
}

/// A directory of recorded model replies: for each `(id, scene, judge)` a
/// `<id>.scene.json` and a `<id>.judge.json`.
pub fn replay_dir(replies: &[(&str, &str, &str)]) -> TempDir {
    let dir = TempDir::new("replay");
    for (id, scene, judge) in replies {
        write(dir.path(), &format!("{id}.scene.json"), scene);
        write(dir.path(), &format!("{id}.judge.json"), judge);
    }
    dir
}

fn write(dir: &Path, name: &str, text: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("creates the parent");
    }
    fs::write(path, text).expect("writes the file");
}
