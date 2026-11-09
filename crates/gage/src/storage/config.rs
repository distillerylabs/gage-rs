//! gage's global `$GAGE_CONFIG/config.toml`: the vaults this device knows,
//! how this device unlocks each one, and shell preferences. See the design
//! doc's "Global config" section.
//!
//! The schema (file name, table and field names) is fixed by the format; the bytes are
//! the `toml` crate's own (ADR-0002).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{ErrorKind, HasKind};
use crate::storage::serde_opt::empty_as_none;
use crate::storage::vaultconfig::{UnlockMethod, VaultId, VaultType};
use crate::storage::xdgpaths::Dirs;
use crate::storage::{atomicfile, paths, vaultlock};

/// How long [`update`] waits for another process's rewrite to finish. A
/// rewrite holds the lock for moments, so this is only ever reached when
/// something is badly wrong.
pub const UPDATE_LOCK_TIMEOUT: Duration = Duration::from_secs(10);

/// Git-specific metadata for a vault, namespaced under its own table so a
/// future vault type only ever adds a table rather than reshaping this one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GitMeta {
    #[serde(
        deserialize_with = "empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub origin: Option<String>,
}

impl GitMeta {
    fn is_empty(&self) -> bool {
        self.origin.is_none()
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
    /// files have moved or gone (ADR-0006). Validated as it is read, like
    /// the committed copy: a bad one refuses the whole file (ADR-0052).
    #[serde(
        deserialize_with = "empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub id: Option<VaultId>,
    #[serde(rename = "type")]
    pub vault_type: VaultType,
    pub device: String,
    /// This device's own public key for the vault, recorded when the
    /// identity is created, so "is the key I hold still a recipient?" is
    /// answered by comparing keys rather than device names
    /// (ADR-0013). Public and already committed in the vault.
    #[serde(
        deserialize_with = "empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub pubkey: Option<String>,
    pub method: UnlockMethod,
    #[serde(skip_serializing_if = "GitMeta::is_empty")]
    pub git: GitMeta,
}

/// This device's session-mode preferences: CLI-terminal policy a GUI
/// frontend would not share.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shell {
    #[serde(
        deserialize_with = "empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub prompt: Option<String>,
    #[serde(
        deserialize_with = "empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub idle_timeout: Option<String>,
    #[serde(
        deserialize_with = "empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub history_file: Option<String>,
    #[serde(
        deserialize_with = "empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub clipboard_timeout: Option<String>,
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
    #[serde(
        deserialize_with = "empty_as_none",
        skip_serializing_if = "Option::is_none"
    )]
    pub current: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub vaults: BTreeMap<String, VaultEntry>,
    #[serde(skip_serializing_if = "Shell::is_empty")]
    pub shell: Shell,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No config file yet: a first run, not a fault. Callers decide whether
    /// that means "empty config" or an error.
    #[error("{path} does not exist")]
    NotFound { path: PathBuf },
    #[error("reading {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("encoding {path}: {source}")]
    Encode {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("creating {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("writing {path}: {source}")]
    Write {
        path: PathBuf,
        source: atomicfile::Error,
    },
    /// The lock held across a rewrite couldn't be taken.
    #[error(transparent)]
    Lock(vaultlock::Error),
}

impl HasKind for Error {
    fn kind(&self) -> ErrorKind {
        match self {
            Error::Lock(e) => e.kind(),
            _ => ErrorKind::Internal,
        }
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

impl HasKind for RegistryError {
    fn kind(&self) -> ErrorKind {
        match self {
            RegistryError::AlreadyRegistered(_) => ErrorKind::Conflict,
            RegistryError::NotRegistered(_) => ErrorKind::NotFound,
            RegistryError::NoCurrent => ErrorKind::Usage,
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
        if self.current.is_none() {
            self.current = Some(name.to_string());
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
        match name.or(self.current.as_deref()) {
            None | Some("") => Err(RegistryError::NoCurrent),
            Some(name) => Ok(name),
        }
    }

    /// Forgets a registration, and clears `current` if it named it.
    /// Returns what was removed.
    pub fn unregister(&mut self, name: &str) -> Result<VaultEntry, RegistryError> {
        let entry = self
            .vaults
            .remove(name)
            .ok_or_else(|| RegistryError::NotRegistered(name.to_string()))?;
        if self.current.as_deref() == Some(name) {
            self.current = None;
        }
        Ok(entry)
    }

    /// Makes `name`, which must be registered, the current vault.
    pub fn set_default(&mut self, name: &str) -> Result<(), RegistryError> {
        self.entry(name)?;
        self.current = Some(name.to_string());
        Ok(())
    }
}

/// Applies `f` to the global config, as the one way to change it
/// (ADR-0077). The file is read after the lock is taken and written before
/// it is released, so a change another process made in the meantime is
/// kept. Nothing is written if `f` fails.
///
/// `f` must not wait on anything slow, a person or the network, or take
/// another lock: reading first and calling this for the change is how to
/// keep a slow step outside.
pub fn update<T, E: From<Error>>(
    dirs: &Dirs,
    f: impl FnOnce(&mut Global) -> Result<T, E>,
) -> Result<T, E> {
    update_within(dirs, UPDATE_LOCK_TIMEOUT, f)
}

fn update_within<T, E: From<Error>>(
    dirs: &Dirs,
    timeout: Duration,
    f: impl FnOnce(&mut Global) -> Result<T, E>,
) -> Result<T, E> {
    let lock_path = paths::global_config_lock_file(dirs);
    if let Some(dir) = lock_path.parent() {
        std::fs::create_dir_all(dir).map_err(|source| Error::CreateDir {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    // Released by dropping it, not `release`: once the save has landed, a
    // failure to unlock mustn't report the change as failed, and closing
    // the handle lets go regardless.
    let _lock = vaultlock::Lock::acquire(&lock_path, timeout).map_err(Error::Lock)?;
    let path = paths::global_config_file(dirs);
    let mut global = load_or_default(&path)?;
    let value = f(&mut global)?;
    save_creating_dir(&path, &global)?;
    Ok(value)
}

/// Reads the global config, treating a missing file as an empty registry:
/// no vault has been registered on this machine yet.
pub fn load_or_default(path: &Path) -> Result<Global, Error> {
    match load(path) {
        Err(Error::NotFound { .. }) => Ok(Global::default()),
        other => other,
    }
}

/// Writes the global config, creating its directory (mode 0700) first if
/// this is the first vault ever registered here.
pub fn save_creating_dir(path: &Path, global: &Global) -> Result<(), Error> {
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
    save(path, global)
}

/// Parses the global config at `path`.
pub fn load(path: &Path) -> Result<Global, Error> {
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
pub fn save(path: &Path, global: &Global) -> Result<(), Error> {
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
            current: Some("personal".into()),
            vaults: BTreeMap::from([(
                "personal".into(),
                VaultEntry {
                    path: "/data/vaults/personal".into(),
                    vault_type: crate::storage::vaultconfig::VaultType::Git,
                    device: "laptop-1".into(),
                    method: "passphrase".into(),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        }
    }

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let want = Global {
            current: Some("personal".into()),
            vaults: BTreeMap::from([
                (
                    "personal".into(),
                    VaultEntry {
                        path: "/data/vaults/personal".into(),
                        id: Some(
                            VaultId::try_from("9f3a1c2e-7b41-4d58-a0c6-2e5f81b3d497").unwrap(),
                        ),
                        vault_type: crate::storage::vaultconfig::VaultType::Git,
                        device: "laptop-1".into(),
                        pubkey: Some(
                            "age1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0savhh7m".into(),
                        ),
                        method: "passphrase".into(),
                        git: GitMeta {
                            origin: Some("https://github.com/you/personal-vault.git".into()),
                        },
                    },
                ),
                (
                    "work".into(),
                    VaultEntry {
                        path: "/data/vaults/work".into(),
                        id: Some(
                            VaultId::try_from("c41d8e07-52b9-4a36-9f18-7d0ae6c25b83").unwrap(),
                        ),
                        vault_type: crate::storage::vaultconfig::VaultType::Git,
                        device: "laptop-1".into(),
                        method: "passphrase".into(),
                        git: GitMeta {
                            origin: Some(
                                "https://git.internal.example/secrets/work-vault.git".into(),
                            ),
                        },
                        ..Default::default()
                    },
                ),
            ]),
            shell: Shell {
                prompt: Some("[{vault}{lock}] gage> ".into()),
                idle_timeout: Some("10m".into()),
                history_file: Some("/state/history".into()),
                ..Default::default()
            },
        };

        save(&path, &want).unwrap();

        assert_eq!(load(&path).unwrap(), want);
        let text = std::fs::read_to_string(&path).unwrap();
        // The table and field names are the format's.
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

    // Empty optional fields are omitted.
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

    // Rust adds a distinguishable variant.
    #[test]
    fn read_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let err = load(&dir.path().join("nope.toml")).unwrap_err();
        assert!(matches!(err, Error::NotFound { .. }), "{err:?}");
    }

    // Only gage writes this file, so a bad id in it is local corruption:
    // refused as the file is read, naming the file and the key, rather
    // than at whichever path builder it would have reached (ADR-0052).
    #[test]
    fn read_refuses_an_invalid_vault_id_naming_the_file_and_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[vaults.personal]\npath = \"/p\"\nid = \"../../etc\"\ntype = \"git\"\n",
        )
        .unwrap();

        let err = load(&path).unwrap_err();

        assert!(matches!(err, Error::Parse { .. }), "{err:?}");
        let msg = err.to_string();
        assert!(msg.contains(&path.display().to_string()), "{msg}");
        assert!(msg.contains("id = \"../../etc\""), "{msg}");
        assert!(msg.contains("is not a valid id"), "{msg}");
    }

    #[test]
    fn read_rejects_malformed_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "current = \"unterminated").unwrap();

        let err = load(&path).unwrap_err();

        assert!(matches!(err, Error::Parse { .. }), "{err:?}");
        assert!(err.to_string().contains("parsing"), "{err}");
    }

    // Atomicfile's own discriminator,
    // applied to config::save: a refactor that dropped the atomicfile
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
        save(&path, &one_vault()).unwrap();
        let mut old = std::fs::File::open(&path).unwrap();

        let work = Global {
            current: Some("work".into()),
            ..Default::default()
        };
        save(&path, &work).unwrap();

        let mut seen = String::new();
        old.read_to_string(&mut seen).unwrap();
        assert_eq!(toml::from_str::<Global>(&seen).unwrap(), one_vault());
        assert_eq!(load(&path).unwrap(), work);
    }

    #[cfg(unix)]
    #[test]
    fn failed_global_config_write_leaves_previous_config_parseable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(&path, &one_vault()).unwrap();
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

        let err = save(
            &path,
            &Global {
                current: Some("work".into()),
                ..Default::default()
            },
        )
        .unwrap_err();

        assert!(matches!(err, Error::Write { .. }), "{err:?}");
        assert_eq!(load(&path).unwrap(), one_vault());
    }

    // Two writes, the second wins. This doesn't observe the old value surviving until the rename either; that
    // property is `global_config_write_is_atomic`'s, above.
    #[test]
    fn write_then_modify() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for current in ["a", "b"] {
            let g = Global {
                current: Some(current.into()),
                ..Default::default()
            };
            save(&path, &g).unwrap();
        }
        assert_eq!(load(&path).unwrap().current, Some("b".into()));
    }

    fn entry(path: &str) -> VaultEntry {
        VaultEntry {
            path: path.into(),
            vault_type: crate::storage::vaultconfig::VaultType::Git,
            ..Default::default()
        }
    }

    #[test]
    fn register_makes_the_first_vault_current_and_only_the_first() {
        let mut g = Global::default();
        g.register("personal", entry("/a")).unwrap();
        assert_eq!(g.current, Some("personal".into()));
        g.register("work", entry("/b")).unwrap();
        assert_eq!(g.current, Some("personal".into()));
        assert_eq!(g.entry("work").unwrap(), &entry("/b"));
    }

    #[test]
    fn register_refuses_a_name_already_registered_and_changes_nothing() {
        let mut g = Global::default();
        g.register("personal", entry("/a")).unwrap();
        let before = g.clone();

        let err = g.register("personal", entry("/b")).unwrap_err();

        assert_eq!(err, RegistryError::AlreadyRegistered("personal".into()));
        assert_eq!(err.kind(), ErrorKind::Conflict);
        assert_eq!(g, before);
    }

    #[test]
    fn entry_of_an_unknown_name_is_not_found() {
        let err = one_vault().entry("nope").unwrap_err();
        assert_eq!(err, RegistryError::NotRegistered("nope".into()));
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert_eq!(err.to_string(), "no such vault \"nope\"");
    }

    #[test]
    fn name_or_current_prefers_the_given_name() {
        let g = one_vault();
        assert_eq!(g.name_or_current(Some("work")).unwrap(), "work");
        assert_eq!(g.name_or_current(None).unwrap(), "personal");
        let err = Global::default().name_or_current(None).unwrap_err();
        assert_eq!(err, RegistryError::NoCurrent);
        assert_eq!(err.kind(), ErrorKind::Usage);
    }

    #[test]
    fn unregister_clears_current_only_when_it_named_the_vault() {
        let mut g = one_vault();
        g.register("work", entry("/b")).unwrap();

        assert_eq!(g.unregister("work").unwrap(), entry("/b"));
        assert_eq!(g.current, Some("personal".into()));
        g.unregister("personal").unwrap();
        assert_eq!(g.current, None);
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
        assert_eq!(g.current, Some("work".into()));

        let before = g.clone();
        let err = g.set_default("nope").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert_eq!(g, before);
    }

    #[test]
    fn load_treats_a_missing_file_as_an_empty_registry() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load_or_default(&dir.path().join("config.toml")).unwrap(),
            Global::default()
        );
    }

    #[test]
    fn load_still_reports_a_malformed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "current = ").unwrap();
        assert!(matches!(load_or_default(&path), Err(Error::Parse { .. })));
    }

    #[test]
    fn save_creates_the_config_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config").join("gage").join("config.toml");

        save_creating_dir(&path, &one_vault()).unwrap();

        assert_eq!(load_or_default(&path).unwrap(), one_vault());
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
        save(&path, &one_vault()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

// Real, separate OS processes, for the reason vaultlock's tests give: what
// matters is the file lock between processes, and a thread-based test would
// also pass if the exclusion were scoped to something in-process.
#[cfg(test)]
#[allow(clippy::disallowed_macros, clippy::disallowed_methods)]
mod update_tests {
    use std::io::{BufRead, BufReader, Read};
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::mpsc;
    use std::time::Instant;

    use super::*;
    use crate::storage::vaultlock;

    const WRITER_ROOT_ENV: &str = "GAGE_CONFIG_TEST_WRITER_ROOT";
    const WRITER_NAME_ENV: &str = "GAGE_CONFIG_TEST_WRITER_NAME";
    const HOLDER_ENV: &str = "GAGE_CONFIG_TEST_LOCK_HOLDER";

    fn dirs_under(root: &Path) -> Dirs {
        Dirs {
            config: root.join("config"),
            data: root.join("data"),
            state: root.join("state"),
        }
    }

    fn machine() -> (tempfile::TempDir, Dirs) {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_under(root.path());
        (root, dirs)
    }

    fn entry() -> VaultEntry {
        VaultEntry {
            path: "/data/vaults/any".into(),
            vault_type: VaultType::Git,
            device: "laptop-1".into(),
            method: "passphrase".into(),
            ..Default::default()
        }
    }

    fn saved(dirs: &Dirs) -> Global {
        load_or_default(&paths::global_config_file(dirs)).unwrap()
    }

    /// Writes a registration straight to the file, as whoever holds the
    /// lock would have.
    fn write_directly(dirs: &Dirs, name: &str) {
        let mut global = saved(dirs);
        global.register(name, entry()).unwrap();
        save_creating_dir(&paths::global_config_file(dirs), &global).unwrap();
    }

    /// The child side of the race: one registration, slow enough between
    /// its read and its write that every writer overlaps.
    #[test]
    fn registering_writer_trampoline() {
        let Some(root) = std::env::var_os(WRITER_ROOT_ENV) else {
            return;
        };
        let name = std::env::var(WRITER_NAME_ENV).unwrap();
        update(&dirs_under(Path::new(&root)), |global| {
            std::thread::sleep(Duration::from_millis(50));
            global.register(&name, entry()).unwrap();
            Ok::<_, Error>(())
        })
        .unwrap();
    }

    /// The child side of the tests below: takes the lock, announces it on
    /// stderr, and holds it until stdin closes.
    #[test]
    fn lock_holder_trampoline() {
        let Some(path) = std::env::var_os(HOLDER_ENV) else {
            return;
        };
        let lock = vaultlock::Lock::acquire(Path::new(&path), Duration::from_secs(5)).unwrap();
        eprintln!("ACQUIRED");
        let _ = std::io::stdin().read(&mut [0u8]);
        lock.release().unwrap();
    }

    fn test_binary(test: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            &format!("storage::config::update_tests::{test}"),
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ]);
        command
    }

    struct Holder {
        child: Child,
        stdin: Option<ChildStdin>,
    }

    impl Holder {
        /// Returns once the child has the update lock.
        fn spawn(dirs: &Dirs) -> Holder {
            let path = paths::global_config_lock_file(dirs);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut child = test_binary("lock_holder_trampoline")
                .env(HOLDER_ENV, &path)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let stderr = BufReader::new(child.stderr.take().unwrap());
            assert!(
                stderr.lines().any(|l| l.unwrap() == "ACQUIRED"),
                "the holder never took the lock"
            );
            let stdin = child.stdin.take();
            Holder { child, stdin }
        }

        fn release(&mut self) {
            drop(self.stdin.take());
        }
    }

    impl Drop for Holder {
        fn drop(&mut self) {
            self.release();
            let _ = self.child.wait();
        }
    }

    // The bug itself: writers that overlap each load the file, and without
    // the lock the last save wins and the others' registrations are gone.
    #[test]
    fn overlapping_updates_from_separate_processes_all_land() {
        let (root, dirs) = machine();
        let names: Vec<String> = (0..6).map(|i| format!("vault-{i}")).collect();

        let writers: Vec<Child> = names
            .iter()
            .map(|name| {
                test_binary("registering_writer_trampoline")
                    .env(WRITER_ROOT_ENV, root.path())
                    .env(WRITER_NAME_ENV, name)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for mut writer in writers {
            assert!(writer.wait().unwrap().success(), "a writer failed");
        }

        let got: Vec<String> = saved(&dirs).vaults.into_keys().collect();
        assert_eq!(got, names);
    }

    // It waits for the holder, then works from the file as the holder left
    // it, not from anything read before the wait.
    #[test]
    fn an_update_waits_for_the_lock_and_applies_to_what_it_finds() {
        let (_root, dirs) = machine();
        let mut holder = Holder::spawn(&dirs);

        std::thread::scope(|scope| {
            let (tx, rx) = mpsc::channel();
            let waiting = &dirs;
            scope.spawn(move || {
                let result = update(waiting, |global| {
                    global.register("mine", entry()).unwrap();
                    Ok::<_, Error>(())
                });
                let _ = tx.send(result.is_ok());
            });
            assert!(
                rx.recv_timeout(Duration::from_millis(300)).is_err(),
                "the update did not wait for the lock"
            );
            write_directly(&dirs, "theirs");
            holder.release();
            assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(true));
        });

        let got: Vec<String> = saved(&dirs).vaults.into_keys().collect();
        assert_eq!(got, ["mine", "theirs"]);
    }

    #[test]
    fn a_contended_update_gives_up_without_running_or_saving() {
        let (_root, dirs) = machine();
        let _holder = Holder::spawn(&dirs);

        let start = Instant::now();
        let err = update_within(&dirs, Duration::ZERO, |_| -> Result<(), Error> {
            panic!("ran without the lock")
        })
        .unwrap_err();

        assert!(start.elapsed() < Duration::from_millis(500));
        assert!(
            matches!(err, Error::Lock(vaultlock::Error::Contended { .. })),
            "{err:?}"
        );
        // Someone else is mid-write: a state to retry, not a fault.
        assert_eq!(err.kind(), ErrorKind::Conflict);
        assert!(!paths::global_config_file(&dirs).exists());
    }

    #[test]
    fn a_failed_update_saves_nothing_and_releases_the_lock() {
        let (_root, dirs) = machine();
        write_directly(&dirs, "kept");
        let before = saved(&dirs);

        let err = update(&dirs, |global| {
            global.unregister("kept").unwrap();
            Err::<(), _>(Error::NotFound {
                path: "/refused".into(),
            })
        })
        .unwrap_err();

        assert!(matches!(err, Error::NotFound { .. }), "{err:?}");
        assert_eq!(saved(&dirs), before);
        update_within(&dirs, Duration::ZERO, |_| Ok::<_, Error>(())).unwrap();
    }

    #[test]
    fn a_first_update_creates_both_directories_and_starts_empty() {
        let (_root, dirs) = machine();

        let seen = update(&dirs, |global| {
            let seen = global.clone();
            global.register("first", entry()).unwrap();
            Ok::<_, Error>(seen)
        })
        .unwrap();

        assert_eq!(seen, Global::default());
        assert_eq!(saved(&dirs).current.as_deref(), Some("first"));
        assert!(paths::global_config_lock_file(&dirs).exists());
    }

    #[test]
    fn an_unreadable_config_is_reported_and_not_overwritten() {
        let (_root, dirs) = machine();
        let path = paths::global_config_file(&dirs);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not [valid").unwrap();

        let err = update(&dirs, |_| -> Result<(), Error> {
            panic!("ran on a config that doesn't parse")
        })
        .unwrap_err();

        assert!(matches!(err, Error::Parse { .. }), "{err:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not [valid");
    }
}
