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
    #[error("config: creating {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
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

/// Why a lookup or change in the vault registry was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// Refused rather than overwritten: there is no story for what
    /// replacing a registration should do to the vault it points at, and
    /// refusing is the choice that's safe to widen later.
    #[error("{0:?} is already a registered vault")]
    AlreadyRegistered(String),
    #[error("no such vault {0:?}")]
    NotRegistered(String),
    #[error("no vault name given and no current vault is set")]
    NoCurrent,
}

impl HasCode for RegistryError {
    fn code(&self) -> Code {
        match self {
            RegistryError::AlreadyRegistered(_) => Code::Conflict,
            RegistryError::NotRegistered(_) => Code::NotFound,
            RegistryError::NoCurrent => Code::Usage,
        }
    }
}

impl Global {
    /// Adds a new registration, which becomes `current` if nothing is.
    pub fn register(&mut self, name: &str, entry: VaultEntry) -> Result<(), RegistryError> {
        if self.vaults.contains_key(name) {
            return Err(RegistryError::AlreadyRegistered(name.to_string()));
        }
        self.vaults.insert(name.to_string(), entry);
        if self.current.is_empty() {
            self.current = name.to_string();
        }
        Ok(())
    }

    /// The registration named `name`.
    pub fn entry(&self, name: &str) -> Result<&VaultEntry, RegistryError> {
        self.vaults
            .get(name)
            .ok_or_else(|| RegistryError::NotRegistered(name.to_string()))
    }

    /// `name` if given, else `current`: the vault a command defaults to.
    pub fn name_or_current<'a>(&'a self, name: Option<&'a str>) -> Result<&'a str, RegistryError> {
        match name.unwrap_or(&self.current) {
            "" => Err(RegistryError::NoCurrent),
            name => Ok(name),
        }
    }

    /// Forgets a registration, and clears `current` if it named it.
    /// Returns what was removed.
    pub fn unregister(&mut self, name: &str) -> Result<VaultEntry, RegistryError> {
        let entry = self
            .vaults
            .remove(name)
            .ok_or_else(|| RegistryError::NotRegistered(name.to_string()))?;
        if self.current == name {
            self.current.clear();
        }
        Ok(entry)
    }

    /// Makes `name`, which must be registered, the current vault.
    pub fn set_default(&mut self, name: &str) -> Result<(), RegistryError> {
        self.entry(name)?;
        self.current = name.to_string();
        Ok(())
    }
}

/// Reads the global config, treating a missing file as an empty registry:
/// no vault has been registered on this machine yet.
pub fn load(path: &Path) -> Result<Global, Error> {
    match read(path) {
        Err(Error::NotFound { .. }) => Ok(Global::default()),
        other => other,
    }
}

/// Writes the global config, creating its directory (mode 0700) first if
/// this is the first vault ever registered here.
pub fn save(path: &Path, global: &Global) -> Result<(), Error> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir).map_err(|source| Error::CreateDir {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    write(path, global)
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
    // Unix only: Windows refuses to rename over a file that has an open
    // handle, so the old-handle probe cannot run there.
    #[cfg(unix)]
    #[test]
    fn global_config_write_is_atomic() {
        use std::io::Read;
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
        // Said out loud, so a suite run as root (a CI container) shows the
        // test proved nothing rather than reporting a silent pass.
        if std::fs::write(dir.path().join("probe"), b"").is_ok() {
            #[allow(clippy::disallowed_macros)]
            {
                eprintln!(
                    "skipping: running as root, which ignores the unwritable directory this test relies on"
                );
            }
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

    // Go: TestWriteThenModifyLeavesOldValueUntouchedUntilRename, ported as
    // Go's body is: two writes, the second wins. Despite the name, Go's test
    // doesn't observe the old value surviving until the rename either; that
    // property is `global_config_write_is_atomic`'s, above.
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

    fn entry(path: &str) -> VaultEntry {
        VaultEntry {
            path: path.into(),
            vault_type: "git".into(),
            ..Default::default()
        }
    }

    #[test]
    fn register_makes_the_first_vault_current_and_only_the_first() {
        let mut g = Global::default();
        g.register("personal", entry("/a")).unwrap();
        assert_eq!(g.current, "personal");
        g.register("work", entry("/b")).unwrap();
        assert_eq!(g.current, "personal");
        assert_eq!(g.entry("work").unwrap(), &entry("/b"));
    }

    #[test]
    fn register_refuses_a_name_already_registered_and_changes_nothing() {
        let mut g = Global::default();
        g.register("personal", entry("/a")).unwrap();
        let before = g.clone();

        let err = g.register("personal", entry("/b")).unwrap_err();

        assert_eq!(err, RegistryError::AlreadyRegistered("personal".into()));
        assert_eq!(err.code(), Code::Conflict);
        assert_eq!(g, before);
    }

    #[test]
    fn entry_of_an_unknown_name_is_not_found() {
        let err = one_vault().entry("nope").unwrap_err();
        assert_eq!(err, RegistryError::NotRegistered("nope".into()));
        assert_eq!(err.code(), Code::NotFound);
        assert_eq!(err.to_string(), "no such vault \"nope\"");
    }

    #[test]
    fn name_or_current_prefers_the_given_name() {
        let g = one_vault();
        assert_eq!(g.name_or_current(Some("work")).unwrap(), "work");
        assert_eq!(g.name_or_current(None).unwrap(), "personal");
        let err = Global::default().name_or_current(None).unwrap_err();
        assert_eq!(err, RegistryError::NoCurrent);
        assert_eq!(err.code(), Code::Usage);
    }

    #[test]
    fn unregister_clears_current_only_when_it_named_the_vault() {
        let mut g = one_vault();
        g.register("work", entry("/b")).unwrap();

        assert_eq!(g.unregister("work").unwrap(), entry("/b"));
        assert_eq!(g.current, "personal");
        g.unregister("personal").unwrap();
        assert_eq!(g.current, "");
        assert!(g.vaults.is_empty());
        assert_eq!(
            g.unregister("personal").unwrap_err(),
            RegistryError::NotRegistered("personal".into())
        );
    }

    #[test]
    fn set_default_requires_a_registered_name_and_changes_nothing_otherwise() {
        let mut g = one_vault();
        g.register("work", entry("/b")).unwrap();
        g.set_default("work").unwrap();
        assert_eq!(g.current, "work");

        let before = g.clone();
        let err = g.set_default("nope").unwrap_err();
        assert_eq!(err.code(), Code::NotFound);
        assert_eq!(g, before);
    }

    #[test]
    fn load_treats_a_missing_file_as_an_empty_registry() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load(&dir.path().join("config.toml")).unwrap(),
            Global::default()
        );
    }

    #[test]
    fn load_still_reports_a_malformed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "current = ").unwrap();
        assert!(matches!(load(&path), Err(Error::Parse { .. })));
    }

    #[test]
    fn save_creates_the_config_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config").join("gage").join("config.toml");

        save(&path, &one_vault()).unwrap();

        assert_eq!(load(&path).unwrap(), one_vault());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
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
