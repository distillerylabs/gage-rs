//! gage's global `$GAGE_CONFIG/config.toml`: the vaults this device knows,
//! how this device unlocks each one, and shell preferences. See the design
//! doc's "Global config" section.
//!
//! The schema (file name, table and field names) follows Go; the bytes are
//! the `toml` crate's own (Q-COMPAT).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::atomicfile;
use crate::exitcode::{Code, HasCode};

/// Git-specific metadata for a vault, namespaced under its own table so a
/// future vault type only ever adds a table rather than reshaping this one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GitMeta {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub origin: String,
}

impl GitMeta {
    fn is_empty(&self) -> bool {
        self.origin.is_empty()
    }
}

/// One vault's row: what this device calls it, how this device unlocks it,
/// and (for the git type) its remote.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VaultEntry {
    pub path: PathBuf,
    /// A copy of the vault's committed `[vault].id`, so this machine can
    /// find the vault's identities, trust cache and lock without reading
    /// the vault, which is what `vault remove` needs once the vault's
    /// files have moved or gone (A20).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(rename = "type")]
    pub vault_type: String,
    pub device: String,
    /// This device's own public key for the vault, recorded when the
    /// identity is created, so "is the key I hold still a recipient?" is
    /// answered by comparing keys rather than device names
    /// (Q-ORPHAN-BY-NAME). Public and already committed in the vault.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub pubkey: String,
    pub method: String,
    #[serde(skip_serializing_if = "GitMeta::is_empty")]
    pub git: GitMeta,
}

/// This device's session-mode preferences: CLI-terminal policy a GUI
/// frontend would not share.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shell {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub idle_timeout: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub history_file: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub clipboard_timeout: String,
}

impl Shell {
    fn is_empty(&self) -> bool {
        *self == Shell::default()
    }
}

/// The whole of `$GAGE_CONFIG/config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Global {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub current: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub vaults: BTreeMap<String, VaultEntry>,
    #[serde(skip_serializing_if = "Shell::is_empty")]
    pub shell: Shell,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No config file yet: a first run, not a fault. Callers decide whether
    /// that means "empty config" or an error.
    #[error("config: {path} does not exist")]
    NotFound { path: PathBuf },
    #[error("config: reading {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("config: parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("config: encoding {path}: {source}")]
    Encode {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("config: writing {path}: {source}")]
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

/// Parses the global config at `path`.
pub fn read(path: &Path) -> Result<Global, Error> {
    let text = std::fs::read_to_string(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            Error::NotFound {
                path: path.to_path_buf(),
            }
        } else {
            Error::Read {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    toml::from_str(&text).map_err(|source| Error::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Atomically writes `global` to `path` with mode 0600, so an interrupted
/// write never leaves a truncated config listing every registered vault.
pub fn write(path: &Path, global: &Global) -> Result<(), Error> {
    let text = toml::to_string(global).map_err(|source| Error::Encode {
        path: path.to_path_buf(),
        source,
    })?;
    atomicfile::write(path, text.as_bytes(), 0o600).map_err(|source| Error::Write {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn one_vault() -> Global {
        Global {
            current: "personal".into(),
            vaults: BTreeMap::from([(
                "personal".into(),
                VaultEntry {
                    path: "/data/vaults/personal".into(),
                    vault_type: "git".into(),
                    device: "laptop-1".into(),
                    method: "passphrase".into(),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        }
    }

    // Go: TestRoundTrip
    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let want = Global {
            current: "personal".into(),
            vaults: BTreeMap::from([
                (
                    "personal".into(),
                    VaultEntry {
                        path: "/data/vaults/personal".into(),
                        id: "9f3a1c2e-7b41-4d58-a0c6-2e5f81b3d497".into(),
                        vault_type: "git".into(),
                        device: "laptop-1".into(),
                        pubkey: "age1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0savhh7m"
                            .into(),
                        method: "passphrase".into(),
                        git: GitMeta {
                            origin: "https://github.com/you/personal-vault.git".into(),
                        },
                    },
                ),
                (
                    "work".into(),
                    VaultEntry {
                        path: "/data/vaults/work".into(),
                        id: "c41d8e07-52b9-4a36-9f18-7d0ae6c25b83".into(),
                        vault_type: "git".into(),
                        device: "laptop-1".into(),
                        method: "passphrase".into(),
                        git: GitMeta {
                            origin: "https://git.internal.example/secrets/work-vault.git".into(),
                        },
                        ..Default::default()
                    },
                ),
            ]),
            shell: Shell {
                prompt: "[{vault}{lock}] gage> ".into(),
                idle_timeout: "10m".into(),
                history_file: "/state/history".into(),
                ..Default::default()
            },
        };

        write(&path, &want).unwrap();

        assert_eq!(read(&path).unwrap(), want);
        let text = std::fs::read_to_string(&path).unwrap();
        // Go's table and field names: the schema follows Go.
        for needle in [
            "current = ",
            "[vaults.personal]",
            "type = \"git\"",
            "[vaults.personal.git]",
            "origin = ",
            "[shell]",
            "idle_timeout = ",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
    }

    // Empty optional fields are omitted, as Go's omitempty does.
    #[test]
    fn empty_optional_fields_are_omitted() {
        let text = toml::to_string(&one_vault()).unwrap();
        for absent in [
            "id =",
            "pubkey =",
            "[vaults.personal.git]",
            "origin =",
            "[shell]",
        ] {
            assert!(!text.contains(absent), "{absent:?} written in:\n{text}");
        }
        assert_eq!(toml::to_string(&Global::default()).unwrap(), "");
    }

    // Go: TestReadMissingFile. Rust adds a distinguishable variant.
    #[test]
    fn read_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let err = read(&dir.path().join("nope.toml")).unwrap_err();
        assert!(matches!(err, Error::NotFound { .. }), "{err:?}");
    }

    // Go: TestReadRejectsMalformedTOML
    #[test]
    fn read_rejects_malformed_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "current = \"unterminated").unwrap();

        let err = read(&path).unwrap_err();

        assert!(matches!(err, Error::Parse { .. }), "{err:?}");
        assert!(err.to_string().contains("parsing"), "{err}");
    }

    // Go: TestGlobalConfigWriteIsAtomic. atomicfile's own discriminator,
    // applied to config::write: a refactor that dropped the atomicfile
    // call would leave atomicfile's tests green and only this one would
    // notice.
    #[test]
    fn global_config_write_is_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write(&path, &one_vault()).unwrap();
        let mut old = std::fs::File::open(&path).unwrap();

        let work = Global {
            current: "work".into(),
            ..Default::default()
        };
        write(&path, &work).unwrap();

        let mut seen = String::new();
        old.read_to_string(&mut seen).unwrap();
        assert_eq!(toml::from_str::<Global>(&seen).unwrap(), one_vault());
        assert_eq!(read(&path).unwrap(), work);
    }

    // Go: TestFailedGlobalConfigWriteLeavesPreviousConfigParseable
    #[cfg(unix)]
    #[test]
    fn failed_global_config_write_leaves_previous_config_parseable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write(&path, &one_vault()).unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        struct Restore<'a>(&'a Path);
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(self.0, std::fs::Permissions::from_mode(0o700));
            }
        }
        let _restore = Restore(dir.path());
        // Root ignores directory permission bits; there is nothing to prove.
        if std::fs::write(dir.path().join("probe"), b"").is_ok() {
            return;
        }

        let err = write(
            &path,
            &Global {
                current: "work".into(),
                ..Default::default()
            },
        )
        .unwrap_err();

        assert!(matches!(err, Error::Write { .. }), "{err:?}");
        assert_eq!(read(&path).unwrap(), one_vault());
    }

    // Go: TestWriteThenModifyLeavesOldValueUntouchedUntilRename
    #[test]
    fn write_then_modify() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for current in ["a", "b"] {
            let g = Global {
                current: current.into(),
                ..Default::default()
            };
            write(&path, &g).unwrap();
        }
        assert_eq!(read(&path).unwrap().current, "b");
    }

    #[cfg(unix)]
    #[test]
    fn written_with_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write(&path, &one_vault()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
