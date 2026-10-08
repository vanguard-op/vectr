//! The server's filesystem scope (NFR-024).
//!
//! The server runs under the user's account and reads the scene and project
//! files a tool call names, so it treats every path as untrusted: a path is only
//! used when it resolves inside the server's scope. The scope starts at the
//! server's working directory and widens only when the operator passes
//! `--allow <dir>`, so an agent cannot read or write outside the project the
//! server was pointed at.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// The stable error code a path outside the scope is reported under (C-005).
pub const SCOPE: &str = "E_SCOPE";

/// A path could not be admitted by the scope.
#[derive(Debug)]
pub enum ScopeError {
    /// The path resolves outside every allowed root.
    Outside(PathBuf),
    /// The path could not be resolved on the filesystem.
    Io {
        /// The path that could not be resolved.
        path: PathBuf,
        /// The underlying failure.
        error: io::Error,
    },
}

impl fmt::Display for ScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopeError::Outside(path) => write!(
                f,
                "`{}` is outside the server's filesystem scope; start the server with `--allow <dir>` to widen it",
                path.display()
            ),
            ScopeError::Io { path, error } => {
                write!(f, "cannot access `{}`: {error}", path.display())
            }
        }
    }
}

impl std::error::Error for ScopeError {}

/// The roots a tool call may read from and write to.
#[derive(Debug, Clone)]
pub struct Scope {
    roots: Vec<PathBuf>,
}

impl Scope {
    /// Builds a scope from the operator's roots, canonicalizing each so a
    /// symlinked root still contains its own files.
    pub fn new(roots: Vec<PathBuf>) -> Self {
        let mut scope: Vec<PathBuf> = roots
            .into_iter()
            .map(|root| fs::canonicalize(&root).unwrap_or(root))
            .collect();
        if scope.is_empty() {
            scope.push(PathBuf::from("."));
        }
        Self { roots: scope }
    }

    /// The scope's primary root: where an inline scene's project is resolved
    /// from and where a default output lands.
    pub fn base(&self) -> &Path {
        &self.roots[0]
    }

    /// Whether a canonical path lies inside the scope.
    fn contains(&self, canonical: &Path) -> bool {
        self.roots
            .iter()
            .any(|root| canonical == root || canonical.starts_with(root))
    }

    /// Admits an existing file for reading, returning its canonical path.
    pub fn read_path(&self, path: &Path) -> Result<PathBuf, ScopeError> {
        let canonical = fs::canonicalize(path).map_err(|error| ScopeError::Io {
            path: path.to_path_buf(),
            error,
        })?;
        if self.contains(&canonical) {
            Ok(canonical)
        } else {
            Err(ScopeError::Outside(path.to_path_buf()))
        }
    }

    /// Admits a directory for reading, returning its canonical path.
    pub fn read_dir(&self, path: &Path) -> Result<PathBuf, ScopeError> {
        let canonical = self.read_path(path)?;
        if !canonical.is_dir() {
            return Err(ScopeError::Io {
                path: path.to_path_buf(),
                error: io::Error::new(io::ErrorKind::NotADirectory, "not a directory"),
            });
        }
        Ok(canonical)
    }

    /// Admits a destination for writing, returning the path to write to.
    ///
    /// The deepest existing ancestor must resolve inside the scope; the
    /// remaining, not-yet-created components are appended lexically, so a new
    /// output cannot be placed outside a root.
    pub fn write_path(&self, path: &Path) -> Result<PathBuf, ScopeError> {
        let absolute = if path.is_absolute() {
            clean(path)
        } else {
            clean(&self.base().join(path))
        };

        let mut ancestor = absolute.as_path();
        let mut suffix: Vec<&std::ffi::OsStr> = Vec::new();
        while !ancestor.exists() {
            match (ancestor.file_name(), ancestor.parent()) {
                (Some(name), Some(parent)) => {
                    suffix.push(name);
                    ancestor = parent;
                }
                _ => {
                    return Err(ScopeError::Io {
                        path: path.to_path_buf(),
                        error: io::Error::new(
                            io::ErrorKind::NotFound,
                            "no existing ancestor directory",
                        ),
                    })
                }
            }
        }

        let canonical = fs::canonicalize(ancestor).map_err(|error| ScopeError::Io {
            path: ancestor.to_path_buf(),
            error,
        })?;
        if !self.contains(&canonical) {
            return Err(ScopeError::Outside(path.to_path_buf()));
        }

        let mut resolved = canonical;
        for name in suffix.iter().rev() {
            resolved.push(name);
        }
        Ok(resolved)
    }
}

/// Lexically removes `.` components and resolves `..` without touching the
/// filesystem, so a path that escapes its root is caught before any I/O.
fn clean(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !output.pop() {
                    output.push("..");
                }
            }
            other => output.push(other.as_os_str()),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vectr-mcp-scope-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("creates the temp dir");
        dir
    }

    #[test]
    fn a_file_inside_the_root_is_admitted() {
        let root = tempdir("inside");
        let file = root.join("scene.json");
        fs::write(&file, "{}").expect("writes");
        let scope = Scope::new(vec![root.clone()]);

        let admitted = scope.read_path(&file).expect("inside the scope");
        assert_eq!(admitted, fs::canonicalize(&file).unwrap());
    }

    #[test]
    fn a_file_outside_the_root_is_refused() {
        let root = tempdir("root");
        let outside = tempdir("outside");
        let file = outside.join("secret.json");
        fs::write(&file, "{}").expect("writes");
        let scope = Scope::new(vec![root]);

        assert!(matches!(
            scope.read_path(&file),
            Err(ScopeError::Outside(_))
        ));
    }

    #[test]
    fn an_allow_root_widens_the_scope() {
        let root = tempdir("narrow");
        let outside = tempdir("wide");
        let file = outside.join("scene.json");
        fs::write(&file, "{}").expect("writes");
        let scope = Scope::new(vec![root, outside]);

        assert!(scope.read_path(&file).is_ok());
    }

    #[test]
    fn a_parent_escape_on_write_is_refused() {
        let root = tempdir("write-root");
        let scope = Scope::new(vec![root.clone()]);

        let target = root.join("../escape.svg");
        assert!(matches!(
            scope.write_path(&target),
            Err(ScopeError::Outside(_))
        ));
    }

    #[test]
    fn a_new_output_inside_the_root_is_admitted() {
        let root = tempdir("write-inside");
        let scope = Scope::new(vec![root.clone()]);

        let target = root.join("dist").join("out.svg");
        let admitted = scope.write_path(&target).expect("inside");
        assert!(admitted.starts_with(fs::canonicalize(&root).unwrap()));
    }

    #[test]
    fn a_missing_path_is_an_io_error() {
        let root = tempdir("missing");
        let scope = Scope::new(vec![root.clone()]);
        assert!(matches!(
            scope.read_path(&root.join("absent.json")),
            Err(ScopeError::Io { .. })
        ));
    }
}
