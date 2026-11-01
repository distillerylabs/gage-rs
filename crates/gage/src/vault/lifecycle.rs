//! Creating a vault and joining one, each as one operation that leaves
//! either a registered, usable vault or nothing of its own making
//! (ADR-0061). The order is the point: the identity exists before the
//! vault that lists it, the vault before the registration that points at
//! it, and every failure undoes this call's own work and nothing else.

use std::path::{Path, PathBuf};

use crate::devicename;
use crate::error::{ErrorKind, HasKind, Remedy};
use crate::prompter::Prompter;
use crate::recovery::RECOVERY_DEVICE_LABEL;
use crate::storage::config::{self, GitMeta, RegistryError, VaultEntry};
use crate::storage::identityfile::{self, UnlockError};
use crate::storage::paths;
use crate::storage::vaultconfig::{self, UnlockMethod, VaultId, VaultType};
use crate::storage::xdgpaths::Dirs;
use crate::sync::gitrepo::{self, RemoteOptions};
use crate::vault::{CreateError, CreateSpec, LabelledRecipient, NameError, Vault, check_name};

/// What [`Vault::init`] needs. No id: `init` mints it, so the identity is
/// always filed under the id the vault carries (ADR-0006).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitSpec {
    /// Registered under, and recorded as `[vault].name`.
    pub name: String,
    /// The target directory: absent, or an empty directory.
    pub path: PathBuf,
    pub vault_type: VaultType,
    /// A default for devices joining, and this device's own method.
    pub method: UnlockMethod,
    /// This device's name: labels its key, and names its identity file.
    pub device: String,
    /// Further public keys, written after this device's own in the order
    /// given, as [`CreateSpec::recipients`] labels them.
    pub recipients: Vec<String>,
    /// The recovery key's public key, if there is one, written right after
    /// this device's key under [`RECOVERY_DEVICE_LABEL`]. The private half
    /// is the frontend's to deliver; the library never sees it.
    pub recovery_key: Option<String>,
    /// An optional `origin` URL. Set, not pushed: publishing is the
    /// frontend's.
    pub remote: Option<String>,
}

/// A vault [`Vault::init`] created and registered.
#[derive(Debug, Clone)]
pub struct Initialized {
    pub vault: Vault,
    /// This device's public key, as the registration records it.
    pub pubkey: String,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum InitError {
    /// The name is taken ([`RegistryError::AlreadyRegistered`]).
    #[error(transparent)]
    Registry(#[from] RegistryError),
    /// The global config couldn't be read.
    #[error(transparent)]
    LoadRegistry(Box<config::Error>),
    /// The spec is invalid, or creating the vault failed. Any identity
    /// this call made has been removed again.
    #[error(transparent)]
    Create(Box<CreateError>),
    /// This device's identity couldn't be created: the prompter gave no
    /// passphrase, or the file couldn't be written.
    #[error(transparent)]
    Identity(Box<UnlockError>),
    /// Creating the vault failed, and removing the identity made for it
    /// failed too: that identity is still on disk.
    #[error("{cause} (and rolling back the generated identity failed: {rollback})")]
    RollbackFailed {
        /// Boxed, like `rollback`, only to keep the error small.
        cause: Box<CreateError>,
        rollback: Box<UnlockError>,
    },
    /// The vault and this device's identity exist, but registering them
    /// failed. Nothing is rolled back.
    #[error(transparent)]
    SaveRegistry(Box<config::Error>),
}

impl From<UnlockError> for InitError {
    fn from(e: UnlockError) -> InitError {
        InitError::Identity(Box::new(e))
    }
}

impl From<CreateError> for InitError {
    fn from(e: CreateError) -> InitError {
        InitError::Create(Box::new(e))
    }
}

impl HasKind for InitError {
    fn kind(&self) -> ErrorKind {
        match self {
            InitError::Registry(e) => e.kind(),
            InitError::Create(e) => e.kind(),
            InitError::Identity(e) => e.kind(),
            InitError::LoadRegistry(e) | InitError::SaveRegistry(e) => e.kind(),
            InitError::RollbackFailed { .. } => ErrorKind::Internal,
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        match self {
            InitError::Registry(e) => e.remedy(),
            InitError::Create(e) => e.remedy(),
            InitError::Identity(e) => e.remedy(),
            InitError::LoadRegistry(e) | InitError::SaveRegistry(e) => e.remedy(),
            InitError::RollbackFailed { .. } => None,
        }
    }
}

/// What [`Vault::clone_remote`] needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneSpec {
    /// The local name to register under. Checked here whatever its source:
    /// one inferred from the URL comes from whoever controls the URL.
    pub name: String,
    /// The target directory: absent, or an empty directory.
    pub path: PathBuf,
    pub url: String,
    /// This device's name, as the registration records it.
    pub device: String,
}

/// A vault [`Vault::clone_remote`] cloned and registered.
#[derive(Debug, Clone)]
pub struct Cloned {
    pub vault: Vault,
    /// Whether this device already has an identity file for the vault, or
    /// it couldn't be checked. Either way a frontend leaves the device
    /// alone; `false` is the one answer that means "it can't read this
    /// yet".
    pub holds_identity: bool,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CloneError {
    #[error(transparent)]
    Name(#[from] NameError),
    #[error("device name {0:?} is invalid")]
    InvalidDeviceName(String),
    /// The name is taken ([`RegistryError::AlreadyRegistered`]).
    #[error(transparent)]
    Registry(#[from] RegistryError),
    /// The global config couldn't be read.
    #[error(transparent)]
    LoadRegistry(config::Error),
    /// Cloning failed; the target is as it was.
    #[error(transparent)]
    Git(#[from] gitrepo::Error),
    /// The cloned vault's config is unusable (a `format_version` this
    /// build doesn't know, say); the clone has been removed.
    #[error(transparent)]
    Config(#[from] vaultconfig::Error),
    /// The clone failed, and so did undoing it: something is left at
    /// `path`.
    #[error("{cause} (and removing the partial clone at {} failed: {rollback})", path.display())]
    RollbackFailed {
        cause: Box<CloneError>,
        path: PathBuf,
        rollback: std::io::Error,
    },
    /// The clone is in place, but registering it failed. Nothing is rolled
    /// back.
    #[error(transparent)]
    SaveRegistry(config::Error),
}

impl HasKind for CloneError {
    fn kind(&self) -> ErrorKind {
        match self {
            CloneError::Name(e) => e.kind(),
            CloneError::InvalidDeviceName(_) => ErrorKind::Usage,
            CloneError::Registry(e) => e.kind(),
            CloneError::LoadRegistry(e) | CloneError::SaveRegistry(e) => e.kind(),
            CloneError::Git(e) => e.kind(),
            CloneError::Config(e) => e.kind(),
            CloneError::RollbackFailed { .. } => ErrorKind::Internal,
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        match self {
            CloneError::Name(e) => e.remedy(),
            CloneError::Registry(e) => e.remedy(),
            CloneError::LoadRegistry(e) | CloneError::SaveRegistry(e) => e.remedy(),
            CloneError::Git(e) => e.remedy(),
            CloneError::Config(e) => e.remedy(),
            CloneError::InvalidDeviceName(_) | CloneError::RollbackFailed { .. } => None,
        }
    }
}

impl Vault {
    /// Creates a vault with this device as its first recipient, and
    /// registers it. Returns the vault and this device's public key.
    ///
    /// In order, each step only once everything before it holds:
    ///
    /// 1. Refuse a registered name, then an invalid spec. Neither asks the
    ///    prompter: a failure knowable up front must not cost a human two
    ///    blind passphrase entries first.
    /// 2. Mint the id and create this device's identity under it, asking
    ///    `prompter` for a new passphrase (ADR-0006).
    /// 3. Create the vault, listing that identity first and the recovery
    ///    key second. If that fails (a target that isn't empty, say), the
    ///    identity is removed again: it sits under an id no vault carries,
    ///    and nothing would ever point at it.
    /// 4. Register the vault, recording this device's public key while
    ///    gage holds it without an unlock (ADR-0013). A failure here is
    ///    not rolled back.
    pub fn init(
        dirs: &Dirs,
        spec: &InitSpec,
        prompter: &dyn Prompter,
    ) -> Result<Initialized, InitError> {
        let config_path = paths::global_config_file(dirs);
        let mut global = config::load_or_default(&config_path)
            .map_err(|e| InitError::LoadRegistry(Box::new(e)))?;
        if global.vaults.contains_key(&spec.name) {
            return Err(RegistryError::AlreadyRegistered(spec.name.clone()).into());
        }

        // Minted here, never taken from the caller: the identity file is
        // filed under it, so a fresh id is also what guarantees that file
        // is new, and that the rollback below can only ever remove a key
        // this call made.
        let mut create = CreateSpec {
            name: spec.name.clone(),
            id: VaultId::new(),
            path: spec.path.clone(),
            vault_type: spec.vault_type,
            method: spec.method.clone(),
            device: spec.device.clone(),
            recipients: spec.recipients.clone(),
            extra_recipients: spec
                .recovery_key
                .iter()
                .map(|pubkey| LabelledRecipient {
                    device: RECOVERY_DEVICE_LABEL.into(),
                    pubkey: pubkey.clone(),
                })
                .collect(),
            remote: spec.remote.clone(),
        };
        // The target directory is the one thing not checked here: create
        // checks it after the key exists, and the rollback covers it.
        create.validate_without_device_key()?;

        let pubkey =
            identityfile::create(dirs, &create.id, &create.name, &create.device, prompter)?;
        after_identity_created(&create.id);
        // This device's key leads: create labels the first recipient with
        // the device name.
        create.recipients.insert(0, pubkey.clone());

        let vault = match Vault::create(dirs, &create) {
            Ok(vault) => vault,
            Err(cause) => {
                return Err(
                    match identityfile::remove(dirs, &create.id, &create.device) {
                        Ok(()) => cause.into(),
                        Err(rollback) => InitError::RollbackFailed {
                            cause: Box::new(cause),
                            rollback: Box::new(rollback),
                        },
                    },
                );
            }
        };

        global.register(
            &spec.name,
            VaultEntry {
                path: vault.path.clone(),
                id: Some(vault.id.clone()),
                vault_type: spec.vault_type,
                device: spec.device.clone(),
                pubkey: Some(pubkey.clone()),
                method: spec.method.clone(),
                git: GitMeta {
                    origin: spec.remote.clone(),
                },
            },
        )?;
        config::save_creating_dir(&config_path, &global)
            .map_err(|e| InitError::SaveRegistry(Box::new(e)))?;
        Ok(Initialized { vault, pubkey })
    }

    /// Clones the vault at `spec.url` into `spec.path`, and registers it
    /// under `spec.name` for `spec.device`.
    ///
    /// Cloning grants no access: the registration records no public key,
    /// and [`Cloned::holds_identity`] says whether this device can read the
    /// vault yet. The method recorded is the vault's default, a suggestion
    /// for joining devices (ADR-0011).
    ///
    /// A clone that fails, or that turns out to hold a config this build
    /// can't use, is undone: a directory this call made is removed, and an
    /// empty one it was given is emptied again. A target that existed and
    /// wasn't empty is never touched (git refuses it). A failure to
    /// register is not rolled back.
    pub fn clone_remote(dirs: &Dirs, spec: &CloneSpec) -> Result<Cloned, CloneError> {
        check_name(&spec.name)?;
        if !devicename::valid(&spec.device) {
            return Err(CloneError::InvalidDeviceName(spec.device.clone()));
        }
        let config_path = paths::global_config_file(dirs);
        let mut global = config::load_or_default(&config_path).map_err(CloneError::LoadRegistry)?;
        if global.vaults.contains_key(&spec.name) {
            return Err(RegistryError::AlreadyRegistered(spec.name.clone()).into());
        }

        let prior = Prior::of(&spec.path);
        let undo = |cause: CloneError| match roll_back(&spec.path, prior) {
            Ok(()) => cause,
            Err(rollback) => CloneError::RollbackFailed {
                cause: Box::new(cause),
                path: spec.path.clone(),
                rollback,
            },
        };
        gitrepo::clone(&spec.url, &spec.path, &RemoteOptions::new(dirs))
            .map_err(|e| undo(e.into()))?;
        // Read before anything is registered: a format_version this build
        // doesn't know means it can't operate on the vault safely, and
        // registering it anyway would trade one clear failure now for a
        // confusing one on every later command.
        let file =
            vaultconfig::load(&super::config_path(&spec.path)).map_err(|e| undo(e.into()))?;

        global.register(
            &spec.name,
            VaultEntry {
                path: spec.path.clone(),
                id: Some(file.vault.id.clone()),
                vault_type: file.vault.vault_type,
                device: spec.device.clone(),
                // A clone holds no identity, so there is no public key to
                // record until `identity add` puts one here.
                pubkey: None,
                method: file.method.default.clone(),
                git: GitMeta {
                    origin: Some(spec.url.clone()),
                },
            },
        )?;
        config::save_creating_dir(&config_path, &global).map_err(CloneError::SaveRegistry)?;

        // Whether an identity exists, not whether it opens: asking for a
        // passphrase during a clone only to report the unlock was pointless
        // would be backwards. A path that can't be built can't be checked,
        // and is left alone like an identity that exists.
        let holds_identity = paths::identity_file_path(dirs, &file.vault.id, &spec.device)
            .map_or(true, |path| path.exists());
        Ok(Cloned {
            vault: Vault::new(spec.name.clone(), spec.path.clone(), file.vault.id, dirs),
            holds_identity,
        })
    }
}

/// What was at the clone's target before it ran.
#[derive(Clone, Copy)]
enum Prior {
    Absent,
    EmptyDir,
    /// A file, a non-empty directory, or something unreadable: not ours.
    Other,
}

impl Prior {
    fn of(path: &Path) -> Prior {
        if !path.exists() {
            return Prior::Absent;
        }
        match std::fs::read_dir(path).map(|mut entries| entries.next().is_none()) {
            Ok(true) => Prior::EmptyDir,
            _ => Prior::Other,
        }
    }
}

/// Undoes a failed clone so a retry isn't blocked by a leftover: removes
/// the directory it made, or empties the empty one it was given.
fn roll_back(path: &Path, prior: Prior) -> std::io::Result<()> {
    match prior {
        Prior::Other => return Ok(()),
        Prior::Absent => {
            return match std::fs::remove_dir_all(path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            };
        }
        Prior::EmptyDir => {}
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Called with the new id right after this device's identity is written:
/// the one moment a test can make the identity's directory unremovable
/// before the rollback runs. A no-op outside this crate's tests. There is
/// no `Vault` yet to carry it in its `TestSeams` (ADR-0056).
fn after_identity_created(id: &VaultId) {
    #[cfg(test)]
    hooks::AFTER_IDENTITY.with(|hook| {
        if let Some(hook) = &*hook.borrow() {
            hook(id);
        }
    });
    let _ = id;
}

#[cfg(test)]
pub(crate) mod hooks {
    use std::cell::RefCell;

    use crate::storage::vaultconfig::VaultId;

    type Hook = Box<dyn Fn(&VaultId)>;

    thread_local! {
        // Per thread, so a hook set by one test never reaches another.
        pub(super) static AFTER_IDENTITY: RefCell<Option<Hook>> = RefCell::new(None);
    }

    /// Runs `hook` after each identity `Vault::init` writes on this thread,
    /// until the returned guard drops.
    pub(crate) fn after_identity_created(hook: impl Fn(&VaultId) + 'static) -> Guard {
        AFTER_IDENTITY.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
        Guard
    }

    pub(crate) struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            AFTER_IDENTITY.with(|h| *h.borrow_mut() = None);
        }
    }
}
