//! The session's command history file. gage owns it rather than the line
//! editor (ADR-0050): it is created 0600, or
//! tightened to 0600 if it already exists, and only ever appended to, one
//! typed command line per submit. A line editor that saves by rewriting
//! the file would be one more place its mode could be lost.
//!
//! Every line written here is one a human typed at the prompt. A
//! passphrase, a value or a candidate number is read by the prompter,
//! never by the REPL, so it can't reach this file: see "Command history
//! must never contain plaintext" in the design doc.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use gage::exitcode::Code;

use crate::error::CliError;

pub struct History {
    path: Option<PathBuf>,
    file: Option<File>,
}

impl History {
    /// Opens (creating, with its directory) the history file at `path`.
    /// `None` disables history and yields a usable no-op, which is also
    /// what a session falls back to when the file can't be opened: losing
    /// line recall is not a reason to refuse a session.
    pub fn open(path: Option<&Path>) -> Result<History, CliError> {
        let Some(path) = path else {
            return Ok(History {
                path: None,
                file: None,
            });
        };
        let failed = |action: &str, e: std::io::Error| {
            CliError::new(Code::Internal, format!("gage: {action} history file: {e}"))
        };
        if let Some(dir) = path.parent() {
            create_private_dir(dir).map_err(|e| failed("creating the directory of the", e))?;
        }
        let file = open_private(path).map_err(|e| failed("opening the", e))?;
        Ok(History {
            path: Some(path.to_path_buf()),
            file: Some(file),
        })
    }

    /// Appends one typed line. A blank one (Enter at the prompt) is not a
    /// command and isn't recorded.
    pub fn add(&mut self, line: &str) -> Result<(), CliError> {
        let Some(file) = &mut self.file else {
            return Ok(());
        };
        let line = line.trim();
        if line.is_empty() {
            return Ok(());
        }
        file.write_all(format!("{line}\n").as_bytes())
            .map_err(|e| CliError::new(Code::Internal, format!("gage: writing history: {e}")))
    }

    /// The file's lines, oldest first and blanks dropped, for preloading
    /// the line editor's recall. A missing or unreadable file is no
    /// history rather than an error: a session must start either way.
    pub fn lines(&self) -> Vec<String> {
        let Some(path) = &self.path else {
            return Vec::new();
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Vec::new();
        };
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Opens for appending, creating 0600. The create mode only applies to a
/// new file, so an existing one is tightened rather than trusted.
#[cfg(unix)]
fn open_private(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

/// Windows has no mode bits; the file sits under the user's own profile,
/// as every other file gage writes 0600 does.
#[cfg(not(unix))]
fn open_private(path: &Path) -> std::io::Result<File> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // History is a
    // convenience: neither a blank line nor a file it can't read may
    // become an error in a working session.
    #[test]
    fn history_ignores_blank_lines_and_unreadable_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history");
        let mut h = History::open(Some(&path)).unwrap();

        for blank in ["", "   ", "\t"] {
            h.add(blank).unwrap();
        }
        assert!(h.lines().is_empty(), "{:?}", h.lines());

        // A real line is recorded, so the check above isn't passing
        // because nothing is ever written.
        h.add("ls").unwrap();
        assert_eq!(h.lines(), ["ls"]);

        // Here the unreadable file is one
        // that isn't UTF-8.
        std::fs::write(&path, [0xff, 0xfe, b'\n']).unwrap();
        assert!(h.lines().is_empty());
        // Writing still works: the session carries on.
        h.add("ls").unwrap();
    }

    // Lines are appended, never rewritten: a second session's history
    // follows the first's.
    #[test]
    fn history_appends_across_opens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("history");
        History::open(Some(&path)).unwrap().add("one").unwrap();
        let mut h = History::open(Some(&path)).unwrap();
        h.add("  two  ").unwrap();
        assert_eq!(h.lines(), ["one", "two"]);
    }

    // The mode half of creating the file 0600, including the
    // tightening of a file that already existed with a looser mode.
    #[cfg(unix)]
    #[test]
    fn history_file_is_0600_even_if_it_existed_looser() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history");
        std::fs::write(&path, "old\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        History::open(Some(&path)).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
