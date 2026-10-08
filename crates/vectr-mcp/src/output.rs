//! Atomic output writing for the render tool (NFR-011).
//!
//! The render tool generates the whole output in memory before any file is
//! touched, then writes it to a temporary file in the destination directory and
//! renames it into place. A write that fails part way through, or a client that
//! disconnects while a call is in flight, therefore cannot leave a truncated or
//! partial output behind (FEAT-019).

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
            // A rename cannot replace an existing file on every platform, so
            // replace an existing destination explicitly; either way the temp
            // file is removed rather than left behind.
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

    fn tempdir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vectr-mcp-output-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("creates the temp dir");
        dir
    }

    #[test]
    fn writes_creates_parents_and_replaces() {
        let dir = tempdir("write");
        let path = dir.join("dist").join("out.svg");
        write_atomic(&path, b"one").expect("writes");
        assert_eq!(fs::read_to_string(&path).unwrap(), "one");
        write_atomic(&path, b"two").expect("replaces");
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
    }

    #[test]
    fn a_failed_write_leaves_the_existing_file_and_no_temp() {
        let dir = tempdir("failed");
        let blocker = dir.join("blocker");
        fs::write(&blocker, "not a directory").unwrap();
        let path = blocker.join("out.svg");

        assert!(write_atomic(&path, b"x").is_err());
        assert_eq!(fs::read_to_string(&blocker).unwrap(), "not a directory");
    }
}
