//! Removing a vault from this device: its registration, its clone and,
//! if asked, its identity (ADR-0082). For a frontend that owns the
//! directory it clones into, such as the mobile app; `gage vault remove`
//! only forgets the registration and leaves the files to the person.
//!
//! The order is what makes a failure retryable: the clone goes first, then
//! the identity and the trust cache, and the registration last. Until the
//! last step the registration still names the vault, so calling again
//! finishes the job; a clone that is already gone, or half gone, is
//! removed without question.

use std::path::{Path, PathBuf};

use crate::entry::{LockError, WRITE_LOCK_TIMEOUT};
use crate::error::{ErrorKind, HasKind, Remedy};
use crate::storage::config::{self, RegistryError, VaultEntry};
use crate::storage::identityfile::{self, UnlockError};
use crate::storage::vaultconfig::{self, VaultId};
use crate::storage::xdgpaths::Dirs;
use crate::storage::{paths, vaultlock};
use crate::sync::gitrepo::{self, RemoteState};
use crate::trust::trustcache::{self, TrustError};
use crate::vault::{Layout, Vault};

/// What [`Vault::remove_from_device`] removes besides the registration and
/// the clone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RemoveOptions {
    /// Delete this device's identity file for the vault, the only copy of
    /// its private key, whether or not the key is still a recipient. Kept
    /// otherwise, so cloning the vault again can read it straight away.
    pub delete_identity: bool,
    /// Remove even what exists only on this device: see [`LocalOnly`].
    /// Without it, the removal is refused while there is any.
    pub discard_local_only: bool,
    /// Where the clone is looked for: the bindings pass
    /// [`Layout::Managed`], so a clone whose roots have moved is still
    /// found (ADR-0084).
    pub layout: Layout,
}

/// What [`Vault::remove_from_device`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub clone: CloneRemoval,
    /// An identity file was there and has been deleted.
    pub identity_removed: bool,
    /// Whether the key this registration records as this device's own is
    /// still on the vault's recipient list, as the clone listed it before it
    /// went. A device stays a recipient until someone who can write the
    /// vault removes it, so a frontend says so. `None` when it couldn't be
    /// told: no key recorded, or no usable clone to read.
    pub still_a_recipient: Option<bool>,
}

/// What happened to the vault's directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloneRemoval {
    Removed,
    /// Nothing was at the registered path.
    Absent,
    /// The directory is not this call's to delete, and is left as it was.
    Kept(KeptClone),
}

/// Why a vault's directory was left alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeptClone {
    /// It isn't directly inside gage's own vaults directory, so someone
    /// chose where it lives, and deleting it is theirs to do.
    OutsideVaultsDir,
    /// Another registration, named here, points at the same directory.
    SharedWith(String),
}

/// What only this device holds, so removing the vault here loses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalOnly {
    /// The vault has no remote: this clone is its only copy.
    NoRemote,
    /// The working tree has changes no commit holds.
    Uncommitted,
    /// The vault has a remote it has never been pushed to.
    NeverPushed,
    /// Commits the remote doesn't have, as of the last fetch.
    Unpushed { commits: usize },
    /// The identity being deleted holds the vault's only key: nothing could
    /// decrypt any copy of it afterwards.
    LastKey,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RemoveError {
    /// Nothing is registered under the name.
    #[error(transparent)]
    Registry(#[from] RegistryError),
    #[error(transparent)]
    LoadRegistry(config::Error),
    /// Everything else is gone; the registration couldn't be removed.
    /// Calling again finishes it.
    #[error(transparent)]
    SaveRegistry(config::Error),
    /// Refused, with nothing removed: see [`RemoveOptions::discard_local_only`].
    #[error("{}; removing it from this device would lose that", local_only_message(name, *what))]
    LocalOnly { name: String, what: LocalOnly },
    /// The vault's write lock couldn't be taken; nothing was removed.
    #[error(transparent)]
    Lock(#[from] LockError),
    /// The clone couldn't be (wholly) deleted; the registration is kept, so
    /// calling again carries on.
    #[error("removing {}: {source}", path.display())]
    Clone {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The clone is gone, the identity file isn't; the registration is kept.
    #[error(transparent)]
    Identity(#[from] UnlockError),
    /// The clone (and identity, if asked) are gone, the trust cache isn't;
    /// the registration is kept.
    #[error(transparent)]
    Trust(#[from] TrustError),
}

impl From<config::Error> for RemoveError {
    fn from(e: config::Error) -> RemoveError {
        RemoveError::SaveRegistry(e)
    }
}

fn local_only_message(name: &str, what: LocalOnly) -> String {
    match what {
        LocalOnly::NoRemote => format!("{name:?} has no remote, so this is its only copy"),
        LocalOnly::Uncommitted => format!("{name:?} has uncommitted changes"),
        LocalOnly::NeverPushed => format!("{name:?} has never been pushed to its remote"),
        LocalOnly::Unpushed { commits: 1 } => {
            format!("{name:?} has 1 commit its remote doesn't have")
        }
        LocalOnly::Unpushed { commits } => {
            format!("{name:?} has {commits} commits its remote doesn't have")
        }
        LocalOnly::LastKey => {
            format!("this device's key is the only one that can decrypt {name:?}")
        }
    }
}

impl HasKind for RemoveError {
    fn kind(&self) -> ErrorKind {
        match self {
            RemoveError::Registry(e) => e.kind(),
            RemoveError::LoadRegistry(e) | RemoveError::SaveRegistry(e) => e.kind(),
            RemoveError::LocalOnly { .. } => ErrorKind::Conflict,
            RemoveError::Lock(e) => e.kind(),
            RemoveError::Clone { .. } => ErrorKind::Internal,
            RemoveError::Identity(e) => e.kind(),
            RemoveError::Trust(e) => e.kind(),
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        match self {
            RemoveError::Registry(e) => e.remedy(),
            RemoveError::LoadRegistry(e) | RemoveError::SaveRegistry(e) => e.remedy(),
            RemoveError::LocalOnly { what, .. } => match what {
                LocalOnly::NoRemote => Some(Remedy::SetRemote),
                LocalOnly::NeverPushed | LocalOnly::Unpushed { .. } => Some(Remedy::Push),
                LocalOnly::Uncommitted | LocalOnly::LastKey => None,
            },
            RemoveError::Lock(e) => e.remedy(),
            RemoveError::Identity(e) => e.remedy(),
            RemoveError::Trust(e) => e.remedy(),
            RemoveError::Clone { .. } => None,
        }
    }
}

impl Vault {
    /// Removes the vault registered as `name` from this device: its clone,
    /// its trust cache, its identity if `options` asks, and then its
    /// registration. Contacts nothing, and changes nothing in the vault
    /// itself or on its remote: this device stays a recipient
    /// ([`Removed::still_a_recipient`]).
    ///
    /// The clone is looked for where [`RemoveOptions::layout`] places it,
    /// and deleted only when that is directly inside
    /// `$GAGE_DATA/vaults/`, where a frontend that names no path puts it,
    /// and no other registration points at it; otherwise it is kept and
    /// [`CloneRemoval::Kept`] says why. The saved token for the remote's
    /// host is never touched: other vaults may use it.
    ///
    /// Before anything goes, it refuses ([`RemoveError::LocalOnly`]) when
    /// removing would lose something that exists only here, unless
    /// [`RemoveOptions::discard_local_only`] is set. A clone that is
    /// missing, or isn't a repository gage can read, has nothing left to
    /// lose and never refuses.
    ///
    /// Holds the vault's write lock throughout, so no write is half done
    /// under it.
    pub fn remove_from_device(
        dirs: &Dirs,
        name: &str,
        options: &RemoveOptions,
    ) -> Result<Removed, RemoveError> {
        let global = config::load_or_default(&paths::global_config_file(dirs))
            .map_err(RemoveError::LoadRegistry)?;
        let layout = options.layout;
        let entry = layout.resolve(dirs, name, global.entry(name)?);
        let shared_with = global
            .vaults
            .iter()
            .find(|(other, e)| {
                other.as_str() != name && layout.resolve(dirs, other, e).path == entry.path
            })
            .map(|(other, _)| other.clone());

        // Released by dropping it, as `config::update` does: once things are
        // deleted, a failure to unlock mustn't report the removal as failed.
        let _lock = match &entry.id {
            Some(id) => Some(lock(dirs, id)?),
            None => None,
        };

        let clone = if !directly_inside(&entry.path, &paths::vaults_dir(dirs)) {
            Plan::Keep(KeptClone::OutsideVaultsDir)
        } else if let Some(other) = shared_with {
            Plan::Keep(KeptClone::SharedWith(other))
        } else if std::fs::symlink_metadata(&entry.path).is_err() {
            Plan::Absent
        } else {
            Plan::Delete
        };

        let listed = listed_keys(&entry);
        let still_a_recipient = match (&entry.pubkey, &listed) {
            (Some(pubkey), Some(keys)) => Some(keys.contains(pubkey)),
            _ => None,
        };

        if !options.discard_local_only {
            let refuse = |what| RemoveError::LocalOnly {
                name: name.to_string(),
                what,
            };
            if matches!(clone, Plan::Delete)
                && let Some(what) = local_only(&entry.path)
            {
                return Err(refuse(what));
            }
            if options.delete_identity
                && let (Some(pubkey), Some(keys)) = (&entry.pubkey, &listed)
                && keys.as_slice() == [pubkey.clone()]
            {
                return Err(refuse(LocalOnly::LastKey));
            }
        }

        let clone = match clone {
            Plan::Keep(why) => CloneRemoval::Kept(why),
            Plan::Absent => CloneRemoval::Absent,
            Plan::Delete => {
                remove_clone(&entry.path).map_err(|source| RemoveError::Clone {
                    path: entry.path.clone(),
                    source,
                })?;
                CloneRemoval::Removed
            }
        };

        let mut identity_removed = false;
        if let Some(id) = &entry.id {
            if options.delete_identity && !entry.device.is_empty() {
                // A device name no path can be built from names no file.
                if let Ok(path) = paths::identity_file_path(dirs, id, &entry.device) {
                    identity_removed = path.is_file();
                    identityfile::remove(dirs, id, &entry.device)?;
                }
            }
            trustcache::remove(dirs, id)?;
        }

        config::update(dirs, |global| -> Result<(), RemoveError> {
            // Gone already (another call finished first) is what was asked.
            match global.unregister(name) {
                Ok(_) | Err(RegistryError::NotRegistered(_)) => Ok(()),
                Err(e) => Err(e.into()),
            }
        })?;

        Ok(Removed {
            clone,
            identity_removed,
            still_a_recipient,
        })
    }
}

enum Plan {
    Delete,
    Absent,
    Keep(KeptClone),
}

/// Whether `path` names an entry of `dir` itself: one normal last
/// component, no `..`, nothing nested deeper.
fn directly_inside(path: &Path, dir: &Path) -> bool {
    path.parent() == Some(dir) && path.file_name().is_some()
}

fn lock(dirs: &Dirs, id: &VaultId) -> Result<vaultlock::Lock, LockError> {
    let path = paths::lock_file_path(dirs, id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| LockError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    Ok(vaultlock::Lock::acquire(&path, WRITE_LOCK_TIMEOUT)?)
}

/// The recipient keys the clone lists, if it is the registered vault and
/// its config can be read.
fn listed_keys(entry: &VaultEntry) -> Option<Vec<String>> {
    let file = vaultconfig::load(&super::config_path(&entry.path)).ok()?;
    if entry.id.as_ref().is_some_and(|id| *id != file.vault.id) {
        return None;
    }
    Some(file.recipients.into_iter().map(|r| r.pubkey).collect())
}

/// What the clone at `path` holds that its remote doesn't. Answers from
/// what the last fetch left; contacts nothing. A clone that can't be read
/// as a repository has nothing to offer, and answers `None`.
fn local_only(path: &Path) -> Option<LocalOnly> {
    let clean = gitrepo::is_clean(path).ok()?;
    if !gitrepo::has_remote(path).ok()? {
        return Some(LocalOnly::NoRemote);
    }
    if !clean {
        return Some(LocalOnly::Uncommitted);
    }
    match gitrepo::compare(path).ok()? {
        RemoteState::None => Some(LocalOnly::NeverPushed),
        RemoteState::InSync | RemoteState::Behind => None,
        RemoteState::Ahead | RemoteState::Diverged => match gitrepo::ahead_count(path).ok()? {
            0 => None,
            commits => Some(LocalOnly::Unpushed { commits }),
        },
    }
}

/// Deletes the clone, its repository first: once that has gone, what's left
/// can't be read as a repository, so a call after a partial failure removes
/// the rest rather than refusing it as uncommitted changes.
fn remove_clone(path: &Path) -> std::io::Result<()> {
    // A symlink is removed, not followed; so is a stray file.
    if !std::fs::symlink_metadata(path)?.is_dir() {
        return absent_is_fine(std::fs::remove_file(path));
    }
    let git = path.join(".git");
    if std::fs::symlink_metadata(&git).is_ok_and(|m| m.is_dir()) {
        absent_is_fine(std::fs::remove_dir_all(&git))?;
    }
    absent_is_fine(std::fs::remove_dir_all(path))
}

fn absent_is_fine(result: std::io::Result<()>) -> std::io::Result<()> {
    match result {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}
