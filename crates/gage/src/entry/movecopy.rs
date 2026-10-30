//! Cross-vault `mv`/`cp`: decrypt an entry from one vault and encrypt it
//! to another vault's recipients. With one flat recipient list per vault
//! this is the whole sharing story ("`mv`/`cp` are now cross-vault" in the
//! design doc).
//!
//! The destination is never unlocked: encrypting to public keys needs no
//! private key. Both vaults' write locks are held for the whole operation,
//! taken in vault-id order (ADR-0047). The write order is fixed:
//! the destination is written, committed and pushed first, and only then
//! does a move remove the entry from the source. A crash between the two
//! leaves a recoverable duplicate, never an entry lost from both.

use uuid::Uuid;

use crate::crypto::identity::Identity;
use crate::entry::{
    Entry, Error, LockError, ResolveError, Timestamp, WRITE_LOCK_TIMEOUT, new_id, resolve_query,
};
use crate::error::{ErrorKind, HasKind, Remedy};
use crate::prompter::Prompter;
use crate::sync::gitrepo;
use crate::trust::trustcache::TrustError;
use crate::vault::Vault;

/// What a move or copy did.
#[derive(Debug)]
pub struct MoveResult {
    /// The entry's id in the source vault.
    pub source_id: Uuid,
    /// The new entry's id in the destination, which is always a fresh one.
    pub dest_id: Uuid,
    /// The destination's entry as written, for a session's index.
    pub entry: Entry,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MoveError {
    /// Source and destination are one vault (the same id, whatever names
    /// they were registered under). Refused before any lock is taken:
    /// locking one vault twice would wait out the whole timeout.
    #[error("{name:?} is the source vault; --to-vault must name a different vault")]
    SameVault { name: String },
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    Entry(#[from] Error),
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error(transparent)]
    Git(#[from] gitrepo::Error),
    /// The destination's recipient list is one this device never reviewed
    /// and the change wasn't confirmed.
    #[error(transparent)]
    Trust(#[from] TrustError),
    #[error("removing {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
}

impl HasKind for MoveError {
    fn kind(&self) -> ErrorKind {
        match self {
            MoveError::SameVault { .. } => ErrorKind::Usage,
            MoveError::Resolve(e) => e.kind(),
            MoveError::Entry(e) => e.kind(),
            MoveError::Lock(e) => e.kind(),
            MoveError::Git(e) => e.kind(),
            MoveError::Trust(e) => e.kind(),
            MoveError::Io { .. } => ErrorKind::Internal,
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        match self {
            MoveError::Resolve(e) => e.remedy(),
            MoveError::Entry(e) => e.remedy(),
            MoveError::Lock(e) => e.remedy(),
            MoveError::Git(e) => e.remedy(),
            MoveError::Trust(e) => e.remedy(),
            _ => None,
        }
    }
}

impl Vault {
    /// Moves the entry `query` names in this vault into `dest`: written,
    /// committed and pushed there first, then removed here.
    pub fn move_to(
        &self,
        query: &str,
        dest: &Vault,
        identity: &Identity,
        prompter: &dyn Prompter,
    ) -> Result<MoveResult, MoveError> {
        self.move_or_copy(query, dest, identity, prompter, true)
    }

    /// Like [`Vault::move_to`], leaving the original where it is.
    pub fn copy_to(
        &self,
        query: &str,
        dest: &Vault,
        identity: &Identity,
        prompter: &dyn Prompter,
    ) -> Result<MoveResult, MoveError> {
        self.move_or_copy(query, dest, identity, prompter, false)
    }

    /// The copy is a new entry: a fresh id, fresh `created` and `updated`,
    /// and `updated_by` this device. Title, description, value and fields
    /// carry over. A title the destination already has is allowed: there
    /// is no `-f`, and refusing would make sharing fail on the entry most
    /// likely to be shared twice.
    fn move_or_copy(
        &self,
        query: &str,
        dest: &Vault,
        identity: &Identity,
        prompter: &dyn Prompter,
        remove_source: bool,
    ) -> Result<MoveResult, MoveError> {
        if self.id == dest.id {
            return Err(MoveError::SameVault {
                name: dest.name.clone(),
            });
        }
        Vault::with_two_vault_locks(self, dest, WRITE_LOCK_TIMEOUT, prompter, || {
            // Resolved under both locks and after both resets: resolving
            // first would let a concurrent write change what it means.
            let source_id = resolve_query(query, &self.titles(identity)?)?;
            let original = self.read_entry(source_id, identity)?;
            // The destination's cache, not ours: it is the list this
            // entry is about to be encrypted to.
            dest.confirm_recipient_trust(prompter)?;

            let now = Timestamp::now();
            let copy = Entry {
                created: Some(now),
                updated: Some(now),
                updated_by: identity.device().to_string(),
                ..original
            };
            let dest_id = new_id();
            dest.write_entry(dest_id, &copy)?;
            gitrepo::commit_all(&dest.path, &dest_id.to_string())?;
            dest.push_after_write(prompter);

            if remove_source {
                self.move_hook.after_dest_committed();
                let path = self.entry_path(source_id);
                std::fs::remove_file(&path).map_err(|source| MoveError::Io { path, source })?;
                gitrepo::commit_all(&self.path, &source_id.to_string())?;
                self.push_after_write(prompter);
            }
            Ok(MoveResult {
                source_id,
                dest_id,
                entry: copy,
            })
        })
    }
}
