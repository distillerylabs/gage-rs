//! `gage sync` with interactive conflict resolution: for each entry both sides of a divergence changed,
//! decrypt both versions, ask, and land every answer as one real
//! two-parent merge commit. See "Resolving an entry conflict" in the
//! design doc.
//!
//! Three rules shape it:
//!
//! - **Nothing is written until every conflict has a real answer.** That
//!   is what makes skip and abort all-or-nothing for free: there is no
//!   partial state to undo. A merge carrying only some answers would mark
//!   the remote's history incorporated with a skipped entry still at its
//!   local version, and skip would have become keep-local.
//! - **Keep both loses nothing.** The other side's version is re-encrypted
//!   under a fresh id with its own title, `created`, `updated` and
//!   `updated_by`, and the resolver already handles two entries with one
//!   title.
//! - **Unlocking is lazy.** A fast-forward and a disjoint merge decrypt
//!   nothing, so they never reach the unlock; neither does a conflict on a
//!   path that isn't an entry.
//!
//! A merge can also straddle a key change: one side replaced
//! `.gage/vault-key.age` and the other changed entries, which are then
//! tagged under a key the merged vault no longer accepts, and encrypted to
//! a list it no longer has. Those are re-sealed here, unasked, in the merge
//! commit (ADR-0066).

use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::time::Instant;

use uuid::Uuid;

use crate::crypto::crypt;
use crate::crypto::identity::Identity;
use crate::crypto::vaultkey::{self, VAULT_KEY_FILE, VaultKey};
use crate::entry::{self, ENTRIES_DIR_NAME, ENTRY_FILE_EXT, Entry, new_id, seal_entry};
use crate::prompter::Prompter;
use crate::storage::identityfile::UnlockError;
use crate::sync::gitrepo::{self, MergeSide, PendingMerge};
use crate::sync::remotesync::{
    ConflictKind, MERGE_COMMIT_MESSAGE, RECIPIENTS_FILE, REMOTE_OP_TIMEOUT, SyncCause, SyncError,
    SyncReport, local, remote,
};
use crate::trust::recipient::parse_recipients;
use crate::vault::Vault;

/// One device's version of a conflicted entry, decrypted. `None` is the
/// side that deleted it: the delete/modify case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictSide {
    /// This side's content, including the `updated` and `updated_by` a
    /// prompt shows to justify a choice.
    pub entry: Option<Entry>,
}

impl ConflictSide {
    /// Whether this side still has the entry.
    pub fn present(&self) -> bool {
        self.entry.is_some()
    }
}

/// One entry both sides changed, handed to a frontend as data rather than
/// printed text: a CLI renders the design doc's `[l/r/b/s/q]` menu from
/// it, a GUI a dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryConflict {
    /// The entry's id, which both sides share: keep both is what
    /// introduces a second one.
    pub id: Uuid,
    /// The vault-relative path that conflicted, as git names it.
    pub path: String,
    pub local: ConflictSide,
    pub remote: ConflictSide,
}

impl EntryConflict {
    /// What to call the entry: whichever side still has a title, since a
    /// delete/modify conflict has only the survivor to name it by.
    pub fn title(&self) -> &str {
        self.local
            .entry
            .as_ref()
            .or(self.remote.entry.as_ref())
            .map_or("", |e| e.title.as_str())
    }
}

/// A frontend's answer to one [`EntryConflict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Keep this device's version and discard the other's.
    KeepLocal,
    /// Replace this device's version with the other's, a deletion
    /// included.
    KeepRemote,
    /// Keep this device's version where it is, and the other's as a new
    /// entry under a fresh id with its own provenance. Nothing is
    /// destroyed, which is why the option exists.
    KeepBoth,
    /// Leave this entry conflicted and go on to the next. The sync then
    /// ends with nothing applied and nothing pushed.
    Skip,
    /// Stop asking and change nothing.
    Abort,
}

/// What a resolved merge commits under. Like every message gage commits,
/// it names no entry and carries no plaintext: how much was decided, never
/// what.
pub fn resolution_message(resolved: usize) -> String {
    format!(
        "{MERGE_COMMIT_MESSAGE}, resolving {}",
        entry_count(resolved)
    )
}

/// A number of entries, as English: "1 entry", "2 entries".
pub fn entry_count(n: usize) -> String {
    if n == 1 {
        "1 entry".to_string()
    } else {
        format!("{n} entries")
    }
}

/// An entry's path as git names it: slash-separated on every platform.
fn entry_file_path(id: Uuid) -> String {
    format!("{ENTRIES_DIR_NAME}/{id}{ENTRY_FILE_EXT}")
}

/// The entry id a conflicting path names, or `None` for a path that is not
/// an entry. Only the canonical spelling counts, the one gage writes:
/// anything else is a file this vault could never read back by id.
pub(crate) fn entry_id_from_path(path: &str) -> Option<Uuid> {
    let stem = path
        .strip_prefix(ENTRIES_DIR_NAME)?
        .strip_prefix('/')?
        .strip_suffix(ENTRY_FILE_EXT)?;
    let id = Uuid::try_parse(stem).ok()?;
    (id.hyphenated().to_string() == stem).then_some(id)
}

/// The side a key change left behind: the one that didn't touch
/// `.gage/vault-key.age` while the other did. `None` when neither did, or
/// when both did, which is a conflict on the key file itself.
fn stale_side(merge: &PendingMerge) -> Option<MergeSide> {
    let changed = |side| merge.changed(side).contains(VAULT_KEY_FILE);
    match (changed(MergeSide::Local), changed(MergeSide::Remote)) {
        (true, false) => Some(MergeSide::Remote),
        (false, true) => Some(MergeSide::Local),
        _ => None,
    }
}

/// The entries `merge` would land carrying a stale tag, sorted: what the
/// side a key change left behind added or changed. One both sides changed
/// is a conflict, and is settled by its answer instead.
pub(crate) fn stale_entries(merge: &PendingMerge) -> Vec<String> {
    let Some(side) = stale_side(merge) else {
        return Vec::new();
    };
    merge
        .changed(side)
        .iter()
        .filter(|p| entry_id_from_path(p).is_some() && !merge.conflicts().contains(p))
        .cloned()
        .collect()
}

/// The keys a merge reads and writes under.
struct MergeKeys {
    local: VaultKey,
    /// The remote's key, when a key change makes it differ from `local`.
    remote: Option<VaultKey>,
    /// The side a key change left behind.
    stale: Option<MergeSide>,
    /// Who the merged vault encrypts to.
    to: Vec<crypt::Recipient>,
}

impl MergeKeys {
    /// The key `side`'s entries are tagged under.
    fn of(&self, side: MergeSide) -> &VaultKey {
        match (side, &self.remote) {
            (MergeSide::Remote, Some(remote)) => remote,
            _ => &self.local,
        }
    }

    /// The key the merged vault carries: the side that changed it.
    fn merged(&self) -> &VaultKey {
        match self.stale {
            Some(MergeSide::Local) => self.of(MergeSide::Remote),
            _ => &self.local,
        }
    }

    /// `e` as entry `id`, under the merged key and list.
    fn seal(&self, id: Uuid, e: &Entry) -> Result<Vec<u8>, SyncCause> {
        seal_entry(id, e, &self.to, self.merged()).map_err(SyncCause::Entry)
    }
}

impl Vault {
    /// `gage sync` with interactive resolution: pull, merge, and for each
    /// entry both sides changed, present both decrypted versions through
    /// `prompter` and apply the answer, landing the result as one real
    /// two-parent merge commit and pushing it.
    ///
    /// `unlock` supplies the identity resolution decrypts with. It is
    /// called at most once, and only when an entry conflict is actually
    /// reached: a fast-forward, a disjoint merge, and a conflict with
    /// nothing to decrypt never ask for a key. One-shot mode unlocks on
    /// the spot; a session uses [`crate::session::Session::sync_resolving`],
    /// which reuses its cached identity.
    ///
    /// The write lock is held for the whole run, the time a human spends
    /// answering included: taking it per answer would let a write land
    /// between two conflicts and move HEAD out from under the merge being
    /// assembled. The other terminal waiting is the expected behavior.
    pub fn sync_resolving(
        &self,
        prompter: &dyn Prompter,
        unlock: &mut dyn FnMut() -> Result<Identity, UnlockError>,
    ) -> Result<SyncReport, SyncError> {
        // Owned for the run: lives here, lent to the resolution.
        let slot = OnceCell::new();
        self.sync_resolving_with(prompter, &mut || {
            if let Some(identity) = slot.get() {
                return Ok(identity);
            }
            let identity = unlock()?;
            Ok(slot.get_or_init(|| identity))
        })
    }

    /// [`Vault::sync_resolving`] with an unlock that lends its identity:
    /// how a session hands over the key it already holds.
    pub(crate) fn sync_resolving_with<'i>(
        &self,
        prompter: &dyn Prompter,
        unlock: &mut dyn FnMut() -> Result<&'i Identity, UnlockError>,
    ) -> Result<SyncReport, SyncError> {
        self.under_write_lock(|deadline| {
            // Everything resolution might do is first something the
            // automatic paths do alone, decrypting nothing.
            //
            // The opportunistic trust check has its own hook here, not only on
            // unlock: a clean fast-forward never unlocks anything, so a
            // recipient change it pulled in would otherwise arrive in
            // silence. After the pull, as the unlock hook runs after
            // sync_on_unlock: the list worth warning about is the one that
            // just landed. `Vault::sync` takes no prompter and is left
            // alone; `gage sync` comes through here in both modes.
            let warn = &mut || self.warn_recipient_trust(prompter);
            let err = match self.sync_locked_after_pull(deadline, warn) {
                Ok(report) => return Ok(report),
                Err(err) => err,
            };
            // Only an entry conflict is ours. A recipient-file conflict is
            // refused exactly as the automatic push refuses it: the merged
            // file would be an access list neither device wrote. The refusal stands; once a human has reconciled the
            // files by hand, the trust cache's check before the next
            // encrypt is what reviews the result. A push rejected twice, with no
            // conflict found, is the remote moving again.
            if !matches!(*err.cause, SyncCause::Conflict)
                || err.report.conflict_kind() != ConflictKind::Entries
            {
                return Err(err);
            }
            self.resolve_conflicts(prompter, unlock, err.report)
        })
    }

    /// The interactive half: ask about every conflicting entry, then apply
    /// every answer at once.
    fn resolve_conflicts<'i>(
        &self,
        prompter: &dyn Prompter,
        unlock: &mut dyn FnMut() -> Result<&'i Identity, UnlockError>,
        mut report: SyncReport,
    ) -> Result<SyncReport, SyncError> {
        let fail = |cause: SyncCause, report: &SyncReport| {
            SyncError::new(cause).with_report(report.clone())
        };
        let merge =
            gitrepo::prepare_merge(&self.path).map_err(|e| local(e).with_report(report.clone()))?;
        let conflicts = merge.conflicts().to_vec();

        // Every path is checked before anything is unlocked: a conflict on
        // something that isn't an entry has no answer and is refused either
        // way, and a passphrase asked for first would be one nothing then
        // decrypts with.
        let mut ids = Vec::with_capacity(conflicts.len());
        for path in &conflicts {
            match entry_id_from_path(path) {
                Some(id) => ids.push(id),
                None => return Err(fail(SyncCause::NotAnEntry(path.clone()), &report)),
            }
        }

        // The lazy unlock, at the one point plaintext is needed.
        let identity = unlock().map_err(|e| fail(SyncCause::Unlock(e), &report))?;
        let keys = self
            .merge_keys(&merge, identity)
            .map_err(|cause| fail(cause, &report))?;

        let mut choices = BTreeMap::new();
        let mut adds = BTreeMap::new();
        let mut restamped = BTreeMap::new();
        let mut skipped = 0;
        // The version a human kept, re-sealed if a key change left it
        // behind. A deletion has nothing to seal.
        let keep = |side: MergeSide,
                    c: &EntryConflict,
                    restamped: &mut BTreeMap<String, Vec<u8>>|
         -> Result<(), SyncCause> {
            let kept = match side {
                MergeSide::Local => &c.local.entry,
                MergeSide::Remote => &c.remote.entry,
            };
            if let (true, Some(e)) = (keys.stale == Some(side), kept) {
                restamped.insert(c.path.clone(), keys.seal(c.id, e)?);
            }
            Ok(())
        };
        for (path, id) in conflicts.iter().zip(ids) {
            let conflict = self
                .conflict_at(&merge, id, path, identity, &keys)
                .map_err(|cause| fail(cause, &report))?;
            let answer = prompter
                .resolve_conflict(&conflict)
                .map_err(|e| fail(SyncCause::Prompt(e), &report))?;
            match answer {
                Resolution::KeepLocal => {
                    choices.insert(path.clone(), MergeSide::Local);
                    keep(MergeSide::Local, &conflict, &mut restamped)
                        .map_err(|cause| fail(cause, &report))?;
                }
                Resolution::KeepRemote => {
                    choices.insert(path.clone(), MergeSide::Remote);
                    keep(MergeSide::Remote, &conflict, &mut restamped)
                        .map_err(|cause| fail(cause, &report))?;
                }
                Resolution::KeepBoth => {
                    // This side's version stays at its own id (a deletion
                    // stays a deletion); the other's becomes a new entry.
                    // Nothing to add when the other side deleted it: its one
                    // version is the one staying put.
                    choices.insert(path.clone(), MergeSide::Local);
                    keep(MergeSide::Local, &conflict, &mut restamped)
                        .map_err(|cause| fail(cause, &report))?;
                    if let Some(theirs) = &conflict.remote.entry {
                        // Re-encrypted rather than copied: normalized to
                        // the merged vault's recipients and stamped under
                        // its key. Written through verbatim: its
                        // created/updated/updated_by are the losing side's
                        // own, the provenance the prompt just showed.
                        let new = new_id();
                        let ciphertext = keys
                            .seal(new, theirs)
                            .map_err(|cause| fail(cause, &report))?;
                        adds.insert(entry_file_path(new), ciphertext);
                    }
                }
                Resolution::Skip => skipped += 1,
                Resolution::Abort => {
                    // One decision about the whole resolution, not a skip
                    // of each remaining entry, most of which were never
                    // presented.
                    report.aborted = true;
                    return Err(fail(SyncCause::Aborted, &report));
                }
            }
        }
        if skipped > 0 {
            report.skipped = skipped;
            let total = conflicts.len();
            return Err(fail(SyncCause::Unresolved { total }, &report));
        }
        for path in stale_entries(&merge) {
            let sealed = self
                .restamp_stale(&merge, &path, identity, &keys)
                .map_err(|cause| fail(cause, &report))?;
            restamped.insert(path, sealed);
        }
        // Only what a human actually answered stays listed as a conflict.
        report.conflicts.retain(|p| conflicts.contains(p));
        let report = self.apply_resolution(merge.replacing(restamped), report, &choices, &adds)?;
        // A list the merge brought in, which the entries above were just
        // encrypted to. This device hasn't reviewed it, and nothing pulled
        // it in ahead of the check that runs after a pull.
        if keys.stale == Some(MergeSide::Local) {
            self.warn_recipient_trust(prompter);
        }
        Ok(report)
    }

    /// The keys and list `merge` works with. When a key change splits the
    /// two sides, the side that changed it has to descend from the side
    /// that didn't, or its file wasn't written by a holder of this vault's
    /// key: anyone can seal a key to the public recipient list (ADR-0066).
    fn merge_keys(
        &self,
        merge: &PendingMerge,
        identity: &Identity,
    ) -> Result<MergeKeys, SyncCause> {
        let local = self.vault_key(identity).map_err(SyncCause::Entry)?;
        let stale = stale_side(merge);
        let Some(stale_side) = stale else {
            let to = self.encrypt_recipients().map_err(SyncCause::Entry)?;
            return Ok(MergeKeys {
                local,
                remote: None,
                stale,
                to,
            });
        };
        let path = self.path.join(VAULT_KEY_FILE);
        let keyed = |e| SyncCause::Entry(entry::Error::from(e));
        let sealed = merge
            .content(MergeSide::Remote, VAULT_KEY_FILE)
            .map_err(SyncCause::Local)?
            .ok_or_else(|| keyed(vaultkey::Error::Missing { path: path.clone() }))?;
        let remote = VaultKey::open(&sealed, identity).map_err(|e| keyed(e.at(path.clone())))?;
        let descends = match stale_side {
            MergeSide::Local => remote.continues(&local),
            MergeSide::Remote => local.continues(&remote),
        };
        if !descends {
            return Err(keyed(vaultkey::Error::Unrelated { path }));
        }
        let to = match stale_side {
            MergeSide::Remote => self.encrypt_recipients().map_err(SyncCause::Entry)?,
            MergeSide::Local => {
                let list = merge
                    .content(MergeSide::Remote, RECIPIENTS_FILE)
                    .map_err(SyncCause::Local)?
                    .unwrap_or_default();
                parse_recipients(&String::from_utf8_lossy(&list))
                    .iter()
                    .map(|k| crypt::Recipient::parse(k))
                    .collect::<Result<_, _>>()
                    .map_err(|e| SyncCause::Entry(entry::Error::Crypt(e)))?
            }
        };
        Ok(MergeKeys {
            local,
            remote: Some(remote),
            stale,
            to,
        })
    }

    /// One of the entries a key change left behind, read and authenticated
    /// under its own side's key and sealed under the merged one.
    fn restamp_stale(
        &self,
        merge: &PendingMerge,
        path: &str,
        identity: &Identity,
        keys: &MergeKeys,
    ) -> Result<Vec<u8>, SyncCause> {
        let side = keys
            .stale
            .expect("only a straddling merge has stale entries");
        let id = entry_id_from_path(path).expect("stale entries are entry paths");
        let ciphertext = merge
            .content(side, path)
            .map_err(SyncCause::Local)?
            .expect("a stale entry is one its side has");
        let entry = self
            .decrypt_entry_under(id, &ciphertext, identity, keys.of(side))
            .map_err(SyncCause::Entry)?;
        keys.seal(id, &entry)
    }

    /// Writes every answer as one merge commit, and publishes it.
    fn apply_resolution(
        &self,
        merge: PendingMerge,
        mut report: SyncReport,
        choices: &BTreeMap<String, MergeSide>,
        adds: &BTreeMap<String, Vec<u8>>,
    ) -> Result<SyncReport, SyncError> {
        let resolved = choices.len();
        // Nothing answered means a merge that only re-stamped: to history,
        // an ordinary merge.
        let message = match resolved {
            0 => MERGE_COMMIT_MESSAGE.to_string(),
            n => resolution_message(n),
        };
        merge
            .commit(&message, choices, adds)
            .map_err(|e| local(e).with_report(report.clone()))?;
        report.merged = true;
        report.resolved = resolved;
        // Entries were replaced or added under anyone caching this vault's
        // metadata, as a pull does.
        self.pulled.note();
        // Counted before the push, so a resolution whose publish fails
        // still says how much is pending, the merge commit included.
        report.ahead =
            gitrepo::ahead_count(&self.path).map_err(|e| local(e).with_report(report.clone()))?;
        // A fresh window: the one the sync started with bounds network
        // work, and the questions sat in the middle of it. Reusing it would
        // fail to publish a merge already committed for no reason but that
        // someone thought about the answer.
        let sent = self
            .syncer()
            .push(Instant::now() + REMOTE_OP_TIMEOUT)
            .map_err(|e| remote(e).with_report(report.clone()))?;
        report.pushed = sent;
        report.ahead = 0;
        Ok(report)
    }

    /// Decrypts both sides of one conflicting path. Both come out of git
    /// objects: the remote's version was never written to disk, and
    /// reading the local one from the working tree instead would be a
    /// second source of truth for what "local" means.
    fn conflict_at(
        &self,
        merge: &PendingMerge,
        id: Uuid,
        path: &str,
        identity: &Identity,
        keys: &MergeKeys,
    ) -> Result<EntryConflict, SyncCause> {
        let side = |side| -> Result<ConflictSide, SyncCause> {
            let entry = match merge.content(side, path).map_err(SyncCause::Local)? {
                Some(ciphertext) => Some(
                    self.decrypt_entry_under(id, &ciphertext, identity, keys.of(side))
                        .map_err(SyncCause::Entry)?,
                ),
                None => None,
            };
            Ok(ConflictSide { entry })
        };
        Ok(EntryConflict {
            id,
            path: path.to_string(),
            local: side(MergeSide::Local)?,
            remote: side(MergeSide::Remote)?,
        })
    }
}

#[cfg(test)]
#[path = "syncresolve_tests.rs"]
mod tests;
