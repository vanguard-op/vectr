//! Test-only temporary directories for the command tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A unique directory that removes itself when the test ends.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates a fresh directory under the system temporary directory.
    pub fn new(label: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("vectr-cli-{}-{label}-{unique}", std::process::id()));
        fs::create_dir_all(&path).expect("creates the temporary directory");
        Self { path }
    }

    /// The directory's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
