//! Crash-safe file helpers.
//!
//! Writes go to a uniquely named temporary file in the destination directory,
//! are flushed to stable storage and then atomically renamed over the target.
//! A crash or power loss therefore leaves either the old or the new file — never
//! a truncated mix of both.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

/// Atomically replaces `path` with `contents`.
///
/// The parent directory must exist. On Unix the directory entry is also synced
/// so the rename itself survives a crash.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;

    let mut prefix = std::ffi::OsString::from(".");
    prefix.push(file_name);
    prefix.push(".");
    let mut tmp = tempfile::Builder::new()
        .prefix(&prefix)
        .suffix(".tmp")
        .tempfile_in(dir)?;
    tmp.write_all(contents)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;

    #[cfg(unix)]
    {
        // Best effort: some filesystems refuse to fsync directories.
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

/// Reads a whole file, refusing anything larger than `max_bytes`.
///
/// Guards the application against accidentally (or maliciously) opening huge
/// files as projects or settings.
pub fn read_limited(path: &Path, max_bytes: u64) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let len = file.metadata()?.len();
    if len > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("file is {len} bytes, larger than the {max_bytes} byte limit"),
        ));
    }
    let mut buf = Vec::with_capacity(len as usize);
    // `take` also protects against files that grow while being read.
    file.take(max_bytes + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file grew beyond the size limit while reading",
        ));
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.json");
        atomic_write(&path, b"one").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"one");
        atomic_write(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        // No temporary files are left behind.
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn atomic_write_fails_cleanly_for_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("a.json");
        assert!(atomic_write(&path, b"x").is_err());
        assert!(!path.exists());
    }

    #[test]
    fn read_limited_enforces_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big");
        std::fs::write(&path, vec![0u8; 100]).unwrap();
        assert_eq!(read_limited(&path, 100).unwrap().len(), 100);
        let err = read_limited(&path, 99).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
