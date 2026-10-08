//! Output writing shared by the commands (NFR-011).
//!
//! A command generates its whole result in memory before any file is touched, so
//! a failed step leaves an existing file untouched. The write itself goes to a
//! temporary file in the destination directory and is then renamed into place,
//! so a write that fails part way through cannot leave a truncated output.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;
use std::process;

/// Writes `bytes` to `path`, creating the parent directory as needed.
///
/// The destination is only replaced once the temporary file holds the complete
/// result; a failure before the rename leaves `path` as it was.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    fs::create_dir_all(parent)?;

    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "the output path names no file")
    })?;

    let mut temp_name = OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(format!(".tmp-{}", process::id()));
    let temp = parent.join(temp_name);

    if let Err(error) = fs::write(&temp, bytes) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }

    match fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            // A rename cannot replace an existing file on every platform. When
            // the destination is an existing file, replace it explicitly so a
            // rebuild overwrites its previous output rather than failing.
            if path.is_file() {
                if let Err(remove) = fs::remove_file(path) {
                    let _ = fs::remove_file(&temp);
                    return Err(remove);
                }
                if let Err(rename) = fs::rename(&temp, path) {
                    let _ = fs::remove_file(&temp);
                    return Err(rename);
                }
                Ok(())
            } else {
                let _ = fs::remove_file(&temp);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn writes_the_complete_content() {
        let dir = TempDir::new("output-write");
        let path = dir.path().join("out.txt");
        write_atomic(&path, b"hello").expect("writes");
        assert_eq!(fs::read_to_string(&path).expect("reads"), "hello");
    }

    #[test]
    fn replaces_an_existing_file() {
        let dir = TempDir::new("output-replace");
        let path = dir.path().join("out.txt");
        fs::write(&path, "old").expect("seeds the file");
        write_atomic(&path, b"new").expect("writes");
        assert_eq!(fs::read_to_string(&path).expect("reads"), "new");
    }

    #[test]
    fn creates_a_missing_parent_directory() {
        let dir = TempDir::new("output-parent");
        let path = dir.path().join("dist").join("out.txt");
        write_atomic(&path, b"x").expect("writes");
        assert!(path.exists());
    }

    #[test]
    fn an_unwritable_destination_is_an_error_and_leaves_no_partial_file() {
        let dir = TempDir::new("output-unwritable");
        // A file standing where the parent directory should be makes the path
        // unwritable on every platform.
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "not a directory").expect("seeds the blocker");
        let path = blocker.join("out.txt");

        assert!(write_atomic(&path, b"x").is_err());
        assert_eq!(
            fs::read_to_string(&blocker).expect("reads"),
            "not a directory",
            "the blocking file is untouched"
        );
    }
}
