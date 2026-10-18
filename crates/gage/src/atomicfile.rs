//! Writes a file's full contents so that a reader never observes a partial
//! write: the new content lands in a temp file in the same directory, is
//! synced, and only becomes visible through a rename. Every config writer
//! uses it: the global config, a vault's `.gage/config.toml`, and the trust
//! cache's `known-config.toml`. [`write_new`] is the create-only form, for
//! `--recovery-key-out`.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::exitcode::{Code, HasCode};

/// Which step of an atomic write failed. The destination is untouched in
/// every case, and the temp file has been removed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("atomicfile: creating temp file for {path}: {source}")]
    CreateTemp {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("atomicfile: writing temp file for {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("atomicfile: setting permissions for {path}: {source}")]
    Permissions {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("atomicfile: renaming into place at {path}: {source}")]
    Rename {
        path: PathBuf,
        source: std::io::Error,
    },
    /// [`write_new`]'s destination already exists. `source` is the
    /// rename's own error, of kind `AlreadyExists`.
    #[error("atomicfile: {path} already exists")]
    AlreadyExists {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl HasCode for Error {
    fn code(&self) -> Code {
        Code::Internal
    }
}

/// Atomically replaces `path`'s contents with `data`. On Unix the file ends
/// up with permission bits `mode`; on Windows `mode` is ignored.
///
/// An interrupted write (crash, kill) leaves either the old content or the
/// new content in place, never a truncated mix, because the destination is
/// never opened for writing: a new file is renamed over it.
pub fn write(path: &Path, data: &[u8], mode: u32) -> Result<(), Error> {
    let tmp = stage(path, data, mode)?;
    tmp.persist(path).map_err(|e| Error::Rename {
        path: path.to_path_buf(),
        source: e.error,
    })?;
    Ok(())
}

/// Like [`write`], but only creates: if `path` already exists, nothing is
/// written and the error is [`Error::AlreadyExists`]. The check and the
/// rename are one operation, so a file that appears after a caller looked
/// is refused rather than replaced (Q-RECOVERY-NOCLOBBER).
pub fn write_new(path: &Path, data: &[u8], mode: u32) -> Result<(), Error> {
    let tmp = stage(path, data, mode)?;
    tmp.persist_noclobber(path).map_err(|e| {
        let path = path.to_path_buf();
        if e.error.kind() == std::io::ErrorKind::AlreadyExists {
            Error::AlreadyExists {
                path,
                source: e.error,
            }
        } else {
            Error::Rename {
                path,
                source: e.error,
            }
        }
    })?;
    Ok(())
}

/// A synced temp file beside `path` holding `data`, with `mode` set: every
/// step of a write but making it visible. Dropping it on an early return
/// removes it.
fn stage(path: &Path, data: &[u8], mode: u32) -> Result<tempfile::NamedTempFile, Error> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let at = |source| (path.to_path_buf(), source);

    let mut tmp = tempfile::Builder::new()
        .prefix(&format!(".{name}.tmp-"))
        .tempfile_in(dir)
        .map_err(|e| {
            let (path, source) = at(e);
            Error::CreateTemp { path, source }
        })?;
    tmp.write_all(data)
        .and_then(|()| tmp.as_file().sync_all())
        .map_err(|e| {
            let (path, source) = at(e);
            Error::Write { path, source }
        })?;
    set_mode(tmp.as_file(), mode).map_err(|e| {
        let (path, source) = at(e);
        Error::Permissions { path, source }
    })?;
    Ok(tmp)
}

#[cfg(unix)]
fn set_mode(file: &std::fs::File, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_file: &std::fs::File, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    // Go: TestWriteFileCreatesAndReads
    #[test]
    fn creates_and_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        write(&path, b"current = \"personal\"\n", 0o600).unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"current = \"personal\"\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    // The temp file is created 0600 already, so only a mode other than
    // 0600 proves `mode` is applied rather than inherited. Not
    // umask-dependent: the bits are set with chmod, not at creation.
    #[cfg(unix)]
    #[test]
    fn applies_the_requested_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        for mode in [0o644, 0o640, 0o600] {
            let path = dir.path().join(format!("file-{mode:o}"));
            write(&path, b"x", mode).unwrap();
            let got = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(got & 0o777, mode, "{mode:o}");
        }
    }

    // Go: TestWriteFileOverwritesAtomically
    #[test]
    fn overwrites_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        write(&path, b"version 1", 0o600).unwrap();
        write(&path, b"version 2", 0o600).unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"version 2");
        assert_eq!(names_in(dir.path()), ["config.toml"]);
    }

    // Go: TestWriteFileReplacesRatherThanTruncatesInPlace. The test that
    // separates an atomic implementation from a naive one without any
    // fault injection: temp+rename points the path at a new file, so a
    // handle opened on the old one still reads the old bytes. An in-place
    // write reuses the same file, which is exactly the state in which a
    // crash leaves a truncated config.
    // Unix only: Windows refuses to rename over a file that has an open
    // handle, so the old-handle probe cannot run there.
    #[cfg(unix)]
    #[test]
    fn replaces_rather_than_truncates_in_place() {
        use std::io::Read;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write(&path, b"current = \"personal\"\n", 0o600).unwrap();
        let mut old = fs::File::open(&path).unwrap();

        write(&path, b"current = \"work\"\n", 0o600).unwrap();

        let mut seen = String::new();
        old.read_to_string(&mut seen).unwrap();
        assert_eq!(
            seen, "current = \"personal\"\n",
            "the old file was written in place, not replaced"
        );
        assert_eq!(fs::read(&path).unwrap(), b"current = \"work\"\n");
    }

    // Go: TestFailedWriteLeavesPreviousFileIntact. A real failure, not a
    // simulated one: the directory is made unwritable so creating the temp
    // file fails before the destination is touched.
    #[cfg(unix)]
    #[test]
    fn failed_write_leaves_previous_file_intact() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write(&path, b"current = \"personal\"\n", 0o600).unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
        struct Restore<'a>(&'a Path);
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o700));
            }
        }
        let _restore = Restore(dir.path());
        // Root ignores directory permission bits; there is nothing to prove.
        // Said out loud, so a suite run as root (a CI container) shows the
        // test proved nothing rather than reporting a silent pass.
        if fs::write(dir.path().join("probe"), b"").is_ok() {
            #[allow(clippy::disallowed_macros)]
            {
                eprintln!(
                    "skipping: running as root, which ignores the unwritable directory this test relies on"
                );
            }
            return;
        }

        let err = write(&path, b"current = \"CORRUPTED", 0o600).unwrap_err();

        assert!(matches!(err, Error::CreateTemp { .. }), "{err}");
        assert_eq!(fs::read(&path).unwrap(), b"current = \"personal\"\n");
    }

    // Go: TestWriteFileFailedRenameLeavesNoTempFileBehind. Renaming onto a
    // directory fails at the last step on every platform, after the temp
    // file exists, which is what the cleanup has to handle.
    #[test]
    fn failed_rename_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        fs::create_dir(&target).unwrap();

        let err = write(&target, b"x", 0o600).unwrap_err();

        assert!(matches!(err, Error::Rename { .. }), "{err}");
        assert_eq!(names_in(dir.path()), ["config.toml"]);
    }

    // `--recovery-key-out`'s backstop (Q-RECOVERY-NOCLOBBER): a file that
    // appears between the CLI's up-front check and the write is refused,
    // never replaced, and its own error kind says why.
    #[test]
    fn write_new_refuses_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.key");
        fs::write(&path, b"someone else's key").unwrap();

        let err = write_new(&path, b"AGE-SECRET-KEY-1NEW\n", 0o600).unwrap_err();

        match &err {
            Error::AlreadyExists { source, .. } => {
                assert_eq!(source.kind(), std::io::ErrorKind::AlreadyExists)
            }
            other => panic!("{other}"),
        }
        assert_eq!(fs::read(&path).unwrap(), b"someone else's key");
        assert_eq!(names_in(dir.path()), ["recovery.key"], "temp file left");
    }

    #[test]
    fn write_new_creates_the_file_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.key");

        write_new(&path, b"AGE-SECRET-KEY-1NEW\n", 0o600).unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"AGE-SECRET-KEY-1NEW\n");
        assert_eq!(names_in(dir.path()), ["recovery.key"]);
    }

    #[cfg(unix)]
    #[test]
    fn write_new_creates_a_0600_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.key");
        write_new(&path, b"x", 0o600).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // Go: TestWriteFileFailsForMissingDir
    #[test]
    fn fails_for_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does/not/exist/config.toml");

        let err = write(&path, b"x", 0o600).unwrap_err();

        assert!(matches!(err, Error::CreateTemp { .. }), "{err}");
        assert_eq!(err.code(), Code::Internal);
    }
}
