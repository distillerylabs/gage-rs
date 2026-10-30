//! `recipient`: who a vault is encrypted to, and changing that.
//!
//! The read side (who the vault lists, whether its two copies of that list
//! agree) needs no identity: both files are plaintext and committed, so
//! anyone with the vault can inspect them.
//!
//! The write side re-encrypts, always: `add` (ADR-0009) and `remove` alike. Every entry and both recipient files
//! land in exactly one commit, or none of it does. The ordering in
//! [`Vault::commit_recipient_list`] is what makes an interrupted run
//! invisible in history. See "Recipient / access management" and "Why
//! adding a recipient always re-encrypts" in the design doc.

use uuid::Uuid;

mod file;
pub use file::RecipientsFileError;
#[cfg(any(test, feature = "test-support"))]
pub use file::{load, save};
#[cfg(not(any(test, feature = "test-support")))]
pub(crate) use file::{load, save};

#[cfg(test)]
pub(crate) mod recipient_batch_tests;
#[cfg(test)]
pub(crate) mod recipient_tests;

use crate::crypto::identity::Identity;
use crate::enrollment::EnrollmentError;
use crate::entry::{self, LockError, WRITE_LOCK_TIMEOUT};
use crate::error::{ErrorKind, HasKind, Remedy};
use crate::prompter::{PromptError, Prompter, Warning};
use crate::recovery::{ReservedLabel, check_device_label_free};
use crate::storage::vaultconfig::{self, Recipient};
use crate::sync::remotesync::{PushClause, UNPUSHED_WRITE_CLAUSE};
use crate::trust::trustcache::TrustError;
use crate::vault::Vault;
use crate::{
    crypto::agekey, crypto::crypt, devicename, storage::paths, storage::vaultlock, sync::gitrepo,
};

/// Where `.gage/config.toml` and `.age-recipients` differ.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecipientVerification {
    pub in_sync: bool,
    /// Keys `.age-recipients` carries that no `[[recipients]]` lists:
    /// entries are encrypted to someone the config doesn't name.
    pub only_in_recipients_file: Vec<String>,
    /// Keys a `[[recipients]]` lists that `.age-recipients` lacks: someone
    /// the config names who can't decrypt new entries.
    pub only_in_config: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VerifyError {
    #[error(transparent)]
    Config(#[from] vaultconfig::Error),
    #[error(transparent)]
    Recipients(#[from] file::RecipientsFileError),
}

impl HasKind for VerifyError {
    fn kind(&self) -> ErrorKind {
        match self {
            VerifyError::Config(e) => e.kind(),
            VerifyError::Recipients(e) => e.kind(),
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        match self {
            VerifyError::Config(e) => e.remedy(),
            VerifyError::Recipients(e) => e.remedy(),
        }
    }
}

impl Vault {
    /// Every recipient, with its device label, exactly as `.gage/config.toml`
    /// lists them.
    pub fn recipients(&self) -> Result<Vec<vaultconfig::Recipient>, vaultconfig::Error> {
        Ok(vaultconfig::load(&self.config_path())?.recipients)
    }

    /// Compares the keys in `.age-recipients` (what entries are encrypted
    /// to) with those in `.gage/config.toml` (what the vault says it
    /// grants). Order and repeats don't matter: a key is either on both
    /// sides or it isn't.
    ///
    /// Takes no identity and no prompter: it reads two plaintext files.
    pub fn verify_recipients(&self) -> Result<RecipientVerification, VerifyError> {
        let config = self.recipients()?;
        let file = load(&self.path.join(".age-recipients"))?;
        let only_in_recipients_file = missing_from(&file, config.iter().map(|r| &r.pubkey));
        let only_in_config = missing_from(config.iter().map(|r| &r.pubkey), &file);
        Ok(RecipientVerification {
            in_sync: only_in_recipients_file.is_empty() && only_in_config.is_empty(),
            only_in_recipients_file,
            only_in_config,
        })
    }
}

/// The keys of `ours` that `theirs` lacks, in first-seen order, once each.
fn missing_from<'a, 'b>(
    ours: impl IntoIterator<Item = &'a String>,
    theirs: impl IntoIterator<Item = &'b String>,
) -> Vec<String> {
    let theirs: std::collections::HashSet<&String> = theirs.into_iter().collect();
    let mut seen = std::collections::HashSet::new();
    ours.into_iter()
        .filter(|k| !theirs.contains(k) && seen.insert(*k))
        .cloned()
        .collect()
}

/// What an add or a remove did: which recipient, in which commit, and how
/// many entries rode along in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipientChange {
    pub device: String,
    pub pubkey: String,
    /// The one commit's id.
    pub commit: String,
    /// Always the whole vault: both verbs re-encrypt every entry.
    pub reencrypted: usize,
}

/// What a recipient-list commit carries beyond the list itself.
pub(crate) struct RecipientWrite<'a> {
    /// The commit message.
    pub(crate) message: &'a str,
    /// Vault-relative, slash-separated paths deleted in the same commit.
    pub(crate) delete_paths: &'a [String],
    /// What a failed push's warning says went unpublished; `None` is
    /// [`UNPUSHED_WRITE_CLAUSE`].
    pub(crate) push_clause: Option<PushClause<'a>>,
}

impl<'a> RecipientWrite<'a> {
    /// The ordinary write: this message, nothing deleted, the default
    /// clause.
    pub(crate) fn message(message: &'a str) -> RecipientWrite<'a> {
        RecipientWrite {
            message,
            delete_paths: &[],
            push_clause: None,
        }
    }
}

/// Why a recipient change was refused or failed. Every refusal is decided
/// before the first byte is written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RecipientError {
    #[error("device name {0:?} is invalid")]
    InvalidDeviceName(String),
    #[error("{0}")]
    ReservedDeviceLabel(#[from] ReservedLabel),
    #[error(transparent)]
    InvalidRecipient(#[from] agekey::Error),
    /// The key, or the device name, is already listed. A list carrying
    /// either twice makes `verify` and the trust cache's diff harder to
    /// reason about, and buys nothing.
    #[error("this vault already lists that recipient: {}", exists(.device, .pubkey))]
    RecipientExists {
        device: String,
        /// The listed key, when the key is what collided; `None` when it
        /// is the device name.
        pubkey: Option<String>,
    },
    /// Reported rather than a no-op, so a typo can't read as a completed
    /// revocation.
    #[error("no recipient of this vault matches that name or key: {query:?}")]
    RecipientNotFound { query: String },
    /// Removing a key while every existing entry stays encrypted to it
    /// revokes nothing at all.
    #[error(
        "removing a recipient requires --reencrypt: without it the removed key still opens every entry that already exists, so the revocation would only look like one"
    )]
    ReencryptRequired,
    /// There is no `--force`: an empty list can't be encrypted to, and the
    /// re-encryption would already have rewritten every entry to a list
    /// nobody holds a key for.
    #[error(
        "a vault must keep at least one recipient: removing {device:?} would leave nothing to encrypt to, and --reencrypt would rewrite every entry to a list no key opens"
    )]
    LastRecipient { device: String },
    /// The refusal when an actor who can't read the whole vault has nothing
    /// whole to grant. The count, never the ids: those are opaque to it by
    /// construction.
    #[error(
        "this device cannot read every entry in the vault, so it cannot grant full access: {unreadable} of {total} {} cannot be read by {device:?}; a device that can read all of {vault:?} has to make this change",
        if *.total == 1 { "entry" } else { "entries" }
    )]
    CannotGrantFullAccess {
        unreadable: usize,
        total: usize,
        device: String,
        vault: String,
    },
    #[error("not removing {device:?}; nothing was written")]
    SelfRemovalDeclined { device: String },
    /// `recipient verify --repair`, answered no.
    #[error("not repairing the recipient files of {vault:?}; nothing was written")]
    RepairDeclined { vault: String },
    /// `recipient verify --repair` with nobody to confirm what it drops:
    /// never a silent repair.
    #[error(
        "the recipient files of {vault:?} disagree, and the repair could not be confirmed ({}); nothing was written",
        source
    )]
    RepairUnconfirmed { vault: String, source: PromptError },
    #[error(
        "removing {device:?} would lock this device out of {vault:?}, and that could not be confirmed: {source}"
    )]
    Confirm {
        device: String,
        vault: String,
        source: PromptError,
    },
    /// One entry couldn't be decrypted or rewritten; the whole change
    /// aborted, and this names the entry.
    #[error("re-encrypting entry {id}: {source}")]
    Reencrypt { id: Uuid, source: entry::Error },
    /// A key in the new list this build can't encrypt to.
    #[error("re-encrypting to {device:?} ({pubkey}): {source}")]
    ReencryptTo {
        device: String,
        pubkey: String,
        source: crypt::Error,
    },
    /// The vault's two recipient files disagree (the verbs refuse to
    /// rebuild one from the other), or a change this device never
    /// reviewed wasn't confirmed.
    #[error(transparent)]
    Trust(#[from] TrustError),
    #[error(transparent)]
    Entry(#[from] entry::Error),
    #[error(transparent)]
    Config(#[from] vaultconfig::Error),
    #[error(transparent)]
    Recipients(#[from] file::RecipientsFileError),
    #[error(transparent)]
    Path(#[from] paths::Error),
    #[error(transparent)]
    Lock(#[from] vaultlock::Error),
    #[error(transparent)]
    Git(#[from] gitrepo::Error),
    #[error("creating {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
}

pub(crate) fn exists(device: &str, pubkey: &Option<String>) -> String {
    match pubkey {
        Some(key) => format!("{key} is already listed as {device:?}"),
        None => format!("{device:?} is already a recipient of this vault"),
    }
}

impl HasKind for RecipientError {
    fn kind(&self) -> ErrorKind {
        use RecipientError::*;
        match self {
            InvalidDeviceName(_)
            | ReservedDeviceLabel(_)
            | InvalidRecipient(_)
            | ReencryptRequired => ErrorKind::Usage,
            RecipientExists { .. }
            | LastRecipient { .. }
            | CannotGrantFullAccess { .. }
            | SelfRemovalDeclined { .. }
            | RepairDeclined { .. }
            | RepairUnconfirmed { .. } => ErrorKind::Conflict,
            RecipientNotFound { .. } => ErrorKind::NotFound,
            Confirm { .. } => ErrorKind::LockedOrAuth,
            Reencrypt { source, .. } => source.kind(),
            ReencryptTo { source, .. } => source.kind(),
            Trust(e) => e.kind(),
            Entry(e) => e.kind(),
            Config(e) => e.kind(),
            Recipients(e) => e.kind(),
            Path(e) => e.kind(),
            Lock(e) => e.kind(),
            Git(e) => e.kind(),
            Io { .. } => ErrorKind::Internal,
        }
    }

    fn remedy(&self) -> Option<Remedy> {
        use RecipientError::*;
        match self {
            Reencrypt { source, .. } => source.remedy(),
            ReencryptTo { source, .. } => source.remedy(),
            Trust(e) => e.remedy(),
            Entry(e) => e.remedy(),
            Config(e) => e.remedy(),
            Recipients(e) => e.remedy(),
            Path(e) => e.remedy(),
            Lock(e) => e.remedy(),
            Git(e) => e.remedy(),
            _ => None,
        }
    }
}

impl From<LockError> for RecipientError {
    fn from(err: LockError) -> RecipientError {
        match err {
            LockError::Lock(e) => RecipientError::Lock(e),
            LockError::Io { path, source } => RecipientError::Io { path, source },
        }
    }
}

impl Vault {
    /// The pre-flight: can `identity` read every entry in this vault?
    ///
    /// Public and standalone so approval can run it after its late unlock
    /// and before its own lock, as `recipient add` runs it before its. Either way the refusal never arrives mid-write on an
    /// opaque entry id.
    ///
    /// It counts only entries `identity` is not a recipient of. One that
    /// fails for any other reason (corrupt, truncated) is a different
    /// problem with a different fix, and the useful answer there is which
    /// entry, which the re-encryption itself reports. Asks nothing.
    pub(crate) fn require_full_access(&self, identity: &Identity) -> Result<(), RecipientError> {
        let ids = self.entry_ids()?;
        let unreadable = ids
            .iter()
            .filter(|id| {
                matches!(
                    self.read_entry(**id, identity),
                    Err(entry::Error::Crypt(crypt::Error::NotARecipient))
                )
            })
            .count();
        if unreadable == 0 {
            return Ok(());
        }
        Err(RecipientError::CannotGrantFullAccess {
            unreadable,
            total: ids.len(),
            device: identity.device().to_string(),
            vault: self.name.clone(),
        })
    }

    /// Adds `pubkey` as `device` and re-encrypts every entry to the new
    /// list, always (ADR-0009): there is no invocation that leaves a recipient
    /// able to read only part of the vault. That isn't an access tier
    /// anyone chose but an artifact of age baking recipients into each
    /// file, and it is contagious: a partially admitted device can neither
    /// repair itself nor grant full access to anyone else.
    ///
    /// Every entry and both recipient files land in one commit, then the
    /// ordinary post-write push runs; a failed push warns through
    /// `prompter` and fails nothing.
    pub fn add_recipient(
        &self,
        device: &str,
        pubkey: &str,
        identity: &Identity,
        prompter: &dyn Prompter,
    ) -> Result<RecipientChange, RecipientError> {
        if !devicename::valid(device) {
            return Err(RecipientError::InvalidDeviceName(device.to_string()));
        }
        check_device_label_free(device)?;
        agekey::validate_recipient(pubkey)?;

        // The pre-flight, ahead of the lock and of every question. It
        // reads entries off disk, so it is only authoritative on a clean
        // tree: an interrupted `remove <this device> --reencrypt` leaves
        // entries/ encrypted to a list without this device while HEAD,
        // which the reset is about to restore, is entirely readable by it.
        // Refusing on that would send the operator elsewhere to repair a
        // vault that was never damaged. So on a dirty tree the check moves
        // to just after the reset: still before the first byte is written.
        let clean = gitrepo::is_clean(&self.path)?;
        if clean {
            self.require_full_access(identity)?;
        }
        self.with_vault_write(WRITE_LOCK_TIMEOUT, prompter, || {
            if !clean {
                self.require_full_access(identity)?;
            }
            let message = format!("gage: recipient add {device} (reencrypt)");
            let add = Recipient {
                device: device.to_string(),
                pubkey: pubkey.to_string(),
            };
            let (reencrypted, commit) = self.add_recipient_locked(
                std::slice::from_ref(&add),
                &RecipientWrite::message(&message),
                identity,
                prompter,
            )?;
            Ok(RecipientChange {
                device: add.device,
                pubkey: add.pubkey,
                commit,
                reencrypted,
            })
        })
    }

    /// Removes the recipient `query` names (a device name or a public key:
    /// the design spells it `<pubkey-or-name>`) and re-encrypts every entry
    /// to the reduced list.
    ///
    /// `reencrypt` is required, not optional: see
    /// [`RecipientError::ReencryptRequired`]. Two removals are treated
    /// apart because they differ in recoverability: the last recipient is
    /// refused outright, and this device's own key goes only behind a
    /// `prompter.confirm` naming the device that loses access.
    pub fn remove_recipient(
        &self,
        query: &str,
        reencrypt: bool,
        identity: &Identity,
        prompter: &dyn Prompter,
    ) -> Result<RecipientChange, RecipientError> {
        // A rejection of the command line, not a finding about the vault:
        // before the lock, and before anything is read.
        if !reencrypt {
            return Err(RecipientError::ReencryptRequired);
        }
        self.with_vault_write(WRITE_LOCK_TIMEOUT, prompter, || {
            // The trust check, first, as in add, and for the same reasons: ahead of the
            // self-removal question, so nobody is asked to approve an
            // operation about to be refused.
            self.check_recipient_preconditions(prompter)?;
            let current = self.recipients()?;
            let Some(index) = current
                .iter()
                .position(|r| r.device == query || r.pubkey == query)
            else {
                return Err(RecipientError::RecipientNotFound {
                    query: query.to_string(),
                });
            };
            let mut updated = current;
            let removed = updated.remove(index);
            // Before the confirmation below, so a "yes" never meets a
            // refusal.
            if updated.is_empty() {
                return Err(RecipientError::LastRecipient {
                    device: removed.device,
                });
            }
            self.confirm_self_removal(&removed, identity, prompter)?;
            // Said before the work, not after: by the time the commit
            // lands there is nothing left to reconsider, and the point is
            // that revocation is narrower than it sounds.
            prompter.warn(&Warning::from(format!(
                "removing {:?} revokes future access only — anything {} already read can't be unread.",
                removed.device, removed.device
            )));
            let message = format!("gage: recipient remove {} (reencrypt)", removed.device);
            let (reencrypted, commit) = self.commit_recipient_list(
                &updated,
                &RecipientWrite::message(&message),
                identity,
                prompter,
            )?;
            // A list the operator changed here needs no second review at
            // their next write.
            self.note_recipients_reviewed()?;
            Ok(RecipientChange {
                device: removed.device,
                pubkey: removed.pubkey,
                commit,
                reencrypted,
            })
        })
    }

    /// Gates removing the key this vault was unlocked with. Legitimate
    /// (decommissioning this laptop from a vault others still use), and it
    /// works, since every entry is decrypted with the still-valid identity
    /// before the list shrinks; but it is the one way to lock yourself out,
    /// so it asks. `confirm` defaults to no, so a scripted run can't fall
    /// into it.
    fn confirm_self_removal(
        &self,
        removed: &Recipient,
        identity: &Identity,
        prompter: &dyn Prompter,
    ) -> Result<(), RecipientError> {
        if removed.pubkey != identity.recipient() && removed.device != identity.device() {
            return Ok(());
        }
        let question = format!(
            "Remove {:?} — this device's own key? It will no longer be able to read vault {:?}.",
            removed.device, self.name
        );
        match prompter.confirm(&question) {
            Ok(true) => Ok(()),
            Ok(false) => Err(RecipientError::SelfRemovalDeclined {
                device: removed.device.clone(),
            }),
            Err(source) => Err(RecipientError::Confirm {
                device: removed.device.clone(),
                vault: self.name.clone(),
                source,
            }),
        }
    }

    /// The add itself, for one recipient or several, with the write lock
    /// held. Split from [`Vault::add_recipient`] so enrollment's approval can call
    /// it under a lock it took itself: a batch is one trust question, one
    /// re-encryption pass and one commit, which also carries whatever
    /// `write` says to delete. Returns how many entries were rewritten and
    /// the commit's id.
    ///
    /// Every member is validated here, the function that writes the list,
    /// and checked against the vault's list and against the rest of the
    /// batch before the first byte is written.
    pub(crate) fn add_recipient_locked(
        &self,
        adds: &[Recipient],
        write: &RecipientWrite<'_>,
        identity: &Identity,
        prompter: &dyn Prompter,
    ) -> Result<(usize, String), RecipientError> {
        self.check_recipient_preconditions(prompter)?;
        let current = self.recipients()?;
        // A device name in the list becomes a path component, and a config
        // carrying an invalid one is a vault nobody can open.
        for add in adds {
            if !devicename::valid(&add.device) {
                return Err(RecipientError::InvalidDeviceName(add.device.clone()));
            }
            check_device_label_free(&add.device)?;
            agekey::validate_recipient(&add.pubkey)?;
        }
        // The vault's list, then each member against those before it: a
        // batch whose two members claim one key or one name is refused for
        // the same reason a single add of either would be.
        for (i, add) in adds.iter().enumerate() {
            let earlier = current.iter().chain(&adds[..i]);
            if let Some(existing) = earlier.clone().find(|r| r.pubkey == add.pubkey) {
                return Err(RecipientError::RecipientExists {
                    device: existing.device.clone(),
                    pubkey: Some(existing.pubkey.clone()),
                });
            }
            if earlier.into_iter().any(|r| r.device == add.device) {
                return Err(RecipientError::RecipientExists {
                    device: add.device.clone(),
                    pubkey: None,
                });
            }
        }
        let mut updated = current;
        updated.extend_from_slice(adds);
        let done = self.commit_recipient_list(&updated, write, identity, prompter)?;
        // The operator just reviewed this list by typing the command, so
        // the cache is regenerated as the last step, once. Without it the
        // next write would warn them about their own add, the fastest way
        // to teach someone to stop reading the warning.
        self.note_recipients_reviewed()?;
        Ok(done)
    }

    /// The two preconditions on add and remove, in this order, both
    /// under the write lock and before the first byte (and so before the
    /// re-encryption pass, not beside the recipient-file write):
    ///
    /// 1. The two recipient files agree. Both verbs rebuild
    ///    `.age-recipients` from `config.toml`, which over a divergence
    ///    would erase the stray key and commit it as an ordinary change.
    ///    First, so nobody is asked to approve an operation about to be
    ///    refused.
    /// 2. The list is one this device reviewed. A change pulled from
    ///    another device is consistent and still unreviewed here; without
    ///    the question, the regeneration at the end of the verb would
    ///    bless it along with the key the operator named, after rewriting
    ///    every entry to it.
    fn check_recipient_preconditions(&self, prompter: &dyn Prompter) -> Result<(), TrustError> {
        self.require_recipients_in_sync()?;
        self.confirm_recipient_trust(prompter)
    }

    /// The shared tail of add and remove: re-encrypt every entry to
    /// `updated`, write both recipient files, and commit all of it once.
    /// There is no path through it that skips the re-encryption.
    ///
    /// The order is the crash-safety property. Entries are written first,
    /// to the new list passed explicitly; the recipient files last, just
    /// before the commit. An interrupted run's dirty surface is then
    /// entries/ (which the next write's reset discards) for as long as
    /// possible, instead of a recipient file naming a key no entry is
    /// encrypted to. Runs with the write lock held.
    pub(crate) fn commit_recipient_list(
        &self,
        updated: &[Recipient],
        write: &RecipientWrite<'_>,
        identity: &Identity,
        prompter: &dyn Prompter,
    ) -> Result<(usize, String), RecipientError> {
        // Resolved before the first byte, so an unsafe path refuses with
        // nothing written.
        let doomed = write
            .delete_paths
            .iter()
            .map(|p| Ok((p.as_str(), gitrepo::safe_join(&self.path, p)?)))
            .collect::<Result<Vec<_>, gitrepo::Error>>()?;
        let reencrypted = self.reencrypt_to(updated, identity)?;
        // After the entries and before the recipient files, which stay
        // last: a failure from here on leaves the deletions
        // uncommitted, not undone, and the next write's dirty-tree reset
        // restores them.
        for (done, (rel, full)) in doomed.iter().enumerate() {
            gitrepo::remove_if_present(full, rel)?;
            self.reencrypt_hook.after_delete(done + 1);
        }
        // Expired enrollment requests ride this commit, at the deletions'
        // crash position: "pruning rides the next recipient change", with
        // no lock taken for housekeeping alone.
        self.prune_pending_enrollments().map_err(|e| match e {
            EnrollmentError::Io { path, source } => RecipientError::Io { path, source },
            other => RecipientError::Io {
                path: self.pending_dir(),
                source: std::io::Error::other(other.to_string()),
            },
        })?;
        self.write_recipient_files(updated)?;
        let commit = gitrepo::commit_all(&self.path, write.message)?;
        // A failed push is a publishing problem, not a half-done migration:
        // the guarantee is about HEAD, and HEAD already has all of it.
        let clause = match &write.push_clause {
            Some(c) => PushClause {
                text: c.text,
                remedy: c.remedy.clone(),
            },
            None => UNPUSHED_WRITE_CLAUSE.into(),
        };
        self.push_after_write_saying(prompter, clause);
        Ok((reencrypted, commit))
    }

    /// Rewrites every entry to `to`, returning how many. Every entry is
    /// decrypted first, and the list parsed, before anything is written: a
    /// failure in either aborts with nothing touched and names what failed.
    fn reencrypt_to(&self, to: &[Recipient], identity: &Identity) -> Result<usize, RecipientError> {
        let ids = self.entry_ids()?;
        let entries = ids
            .iter()
            .map(|&id| {
                self.read_entry(id, identity)
                    .map_err(|source| RecipientError::Reencrypt { id, source })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let parsed = to
            .iter()
            .map(|r| {
                crypt::Recipient::parse(&r.pubkey).map_err(|source| RecipientError::ReencryptTo {
                    device: r.device.clone(),
                    pubkey: r.pubkey.clone(),
                    source,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (done, (&id, e)) in ids.iter().zip(&entries).enumerate() {
            self.write_entry_to(id, e, &parsed)
                .map_err(|source| RecipientError::Reencrypt { id, source })?;
            self.reencrypt_hook.after_entry(done + 1);
        }
        Ok(ids.len())
    }

    /// Writes `.age-recipients` and `.gage/config.toml` from one list, in
    /// that order. Neither is ever written without the other.
    pub(crate) fn write_recipient_files(&self, list: &[Recipient]) -> Result<(), RecipientError> {
        let keys: Vec<String> = list.iter().map(|r| r.pubkey.clone()).collect();
        save(&self.path.join(".age-recipients"), &keys)?;
        let mut file = vaultconfig::load(&self.config_path())?;
        file.recipients = list.to_vec();
        vaultconfig::save(&self.config_path(), &file)?;
        Ok(())
    }
}
