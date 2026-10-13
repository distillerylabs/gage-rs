//! A vault's `.age-recipients`: one age public key per line, with no
//! gage-specific framing, so a stock age CLI can use it directly
//! (`age -R .age-recipients -e file`). See "On-disk layout" in the design
//! doc.

use std::path::{Path, PathBuf};

use crate::atomicfile;
use crate::exitcode::{Code, HasCode};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("recipients: reading {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("recipients: writing {path}: {source}")]
    Write {
        path: PathBuf,
        source: atomicfile::Error,
    },
}

impl HasCode for Error {
    fn code(&self) -> Code {
        Code::Internal
    }
}

/// Reads one public key per line, skipping blank lines; nothing else
/// about the format is gage's own.
pub fn read(path: &Path) -> Result<Vec<String>, Error> {
    let text = std::fs::read_to_string(path).map_err(|source| Error::Read {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

/// Atomically writes `keys` to `path` in order, one per line, each
/// newline-terminated, mode 0644: no header, no comments.
pub fn write(path: &Path, keys: &[String]) -> Result<(), Error> {
    let text: String = keys.iter().map(|k| format!("{k}\n")).collect();
    atomicfile::write(path, text.as_bytes(), 0o644).map_err(|source| Error::Write {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_1: &str = "age1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0savhh7m";
    const KEY_2: &str = "age1yubikey1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s9hkmc0";

    fn keys() -> Vec<String> {
        vec![KEY_1.into(), KEY_2.into(), KEY_1.into()]
    }

    // The format age's recipients-file reader accepts: one recipient per
    // line, each line terminated, nothing else. The age crate's own
    // round trip is the next test.
    #[test]
    fn write_has_no_framing_one_per_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".age-recipients");

        write(&path, &keys()).unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{KEY_1}\n{KEY_2}\n{KEY_1}\n")
        );
    }

    // The round trip M1 deferred to M2: every non-blank, non-comment line of
    // a written file parses with age's X25519 recipient parser, the one
    // stock `age -R` uses for those lines. age's own file reader needs its
    // `cli-common` feature, a frontend dependency the library doesn't take.
    #[test]
    fn written_file_parses_with_ages_own_recipient_parser() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".age-recipients");
        let keys: Vec<String> = (0..3)
            .map(|_| age::x25519::Identity::generate().to_public().to_string())
            .collect();
        write(&path, &keys).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let parsed: Vec<String> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                l.parse::<age::x25519::Recipient>()
                    .unwrap_or_else(|e| panic!("{l:?}: {e}"))
                    .to_string()
            })
            .collect();
        assert_eq!(parsed, keys);
    }

    #[test]
    fn round_trip_keeps_order_and_repeats() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".age-recipients");
        write(&path, &keys()).unwrap();
        assert_eq!(read(&path).unwrap(), keys());
    }

    #[test]
    fn read_skips_blank_lines_and_surrounding_space() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".age-recipients");
        std::fs::write(&path, format!("\n  {KEY_1}  \r\n\n{KEY_2}")).unwrap();
        assert_eq!(read(&path).unwrap(), [KEY_1, KEY_2]);
    }

    #[test]
    fn write_wraps_an_atomicfile_failure() {
        let dir = tempfile::tempdir().unwrap();
        let err = write(&dir.path().join("missing/.age-recipients"), &keys()).unwrap_err();
        assert!(matches!(err, Error::Write { .. }), "{err:?}");
        assert!(err.to_string().starts_with("recipients: writing"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn written_with_mode_0644() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".age-recipients");
        write(&path, &keys()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o644);
    }
}
