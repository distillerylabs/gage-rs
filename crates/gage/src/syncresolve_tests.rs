//! Interactive conflict resolution against a real bare remote, with a
//! [`Device`] playing the other machine and both versions of an entry real
//! ciphertext.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use uuid::Uuid;

use super::*;
use crate::entry::{Entry, ResolveError, Timestamp};
use crate::error::{ErrorKind, HasKind};
use crate::gitrepo;
use crate::gittest::Device;
use crate::identity::Identity;
use crate::prompter::{
    FakePrompter, PromptError, Prompter, UnlockRequest, UnlockResponse, Warning,
};
use crate::remotesync::tests::{DEVICE, SyncVault, sample, unlocks};
use crate::remotesync::{ConflictKind, REMOTE_OP_TIMEOUT, RemoteSyncer, SyncCause, SyncReport};
use crate::syncerr::Kind;
use crate::vault::Vault;
use crate::vaultlock;

fn at(rfc3339: &str) -> Option<Timestamp> {
    Some(rfc3339.parse().unwrap())
}

/// The situation resolution exists for: two devices edited the same
/// entry, both versions are real ciphertext, and this side's write already
/// discovered the divergence.
struct Fixture {
    s: SyncVault,
    identity: Identity,
    id: Uuid,
    path: String,
    mine: Entry,
    theirs: Entry,
    /// The two-conflict fixture's second entry: id, mine, theirs.
    second: Option<(Uuid, Entry, Entry)>,
    head_before: String,
}

fn path_of(id: Uuid) -> String {
    format!("entries/{id}.age")
}

/// The other device publishes `writes` (path, entry) in one commit, each
/// encrypted to this vault's recipients: a genuine, decryptable entry.
fn publish(s: &SyncVault, writes: &[(&str, &Entry)]) -> Device {
    let other = s.other();
    for (path, e) in writes {
        other.write_bytes(path, &s.vault.encrypt_entry(e).unwrap());
    }
    other.commit("the other device's edit");
    other.push();
    other
}

fn base() -> Entry {
    let mut e = Entry {
        title: "ProtonMail".into(),
        description: "mail".into(),
        created: at("2026-08-28T09:00:00Z"),
        updated: at("2026-08-28T09:00:00Z"),
        updated_by: DEVICE.into(),
        value: "the-original-password".into(),
        ..Entry::default()
    };
    e.fields.insert("username".into(), "me".into());
    e
}

impl Fixture {
    /// One entry both sides edited.
    fn new() -> Fixture {
        let s = SyncVault::new();
        let identity = s.unlock();
        let p = FakePrompter::new();
        let id = s
            .vault
            .insert(&identity, sample("ProtonMail"), false, &p)
            .unwrap();
        let original = base();
        s.vault.update(id, &original, &p).unwrap();
        let path = path_of(id);

        let mut theirs = original.clone();
        theirs.value = "their-rotated-password".into();
        theirs.updated = at("2026-08-29T09:03:00Z");
        theirs.updated_by = "phone".into();
        publish(&s, &[(&path, &theirs)]);

        let mut mine = original;
        mine.value = "my-rotated-password".into();
        mine.updated = at("2026-08-30T14:22:00Z");
        mine.updated_by = DEVICE.into();
        // Its automatic push is what discovers the divergence.
        s.vault.update(id, &mine, &p).unwrap();
        assert!(
            p.warnings().iter().any(|w| w.contains("diverged")),
            "{:?}",
            p.warnings()
        );

        let head_before = head(&s.vault);
        Fixture {
            s,
            identity,
            id,
            path,
            mine,
            theirs,
            second: None,
            head_before,
        }
    }

    /// Two entries both sides edited, for the tests that watch what happens
    /// after one conflict is answered.
    fn two() -> Fixture {
        let s = SyncVault::new();
        let identity = s.unlock();
        let p = FakePrompter::new();
        let first = s
            .vault
            .insert(&identity, sample("ProtonMail"), false, &p)
            .unwrap();
        let second = s
            .vault
            .insert(&identity, sample("Bank"), false, &p)
            .unwrap();
        let first_base = base();
        let mut second_base = base();
        second_base.title = "Bank".into();
        s.vault.update(first, &first_base, &p).unwrap();
        s.vault.update(second, &second_base, &p).unwrap();

        let mut their_first = first_base.clone();
        their_first.value = "their-rotated-password".into();
        their_first.updated_by = "phone".into();
        let mut their_second = second_base.clone();
        their_second.value = "their-bank-password".into();
        their_second.updated_by = "phone".into();
        publish(
            &s,
            &[
                (&path_of(first), &their_first),
                (&path_of(second), &their_second),
            ],
        );

        let mut mine = first_base;
        mine.value = "my-rotated-password".into();
        let mut mine_second = second_base;
        mine_second.value = "my-bank-password".into();
        s.vault.update(first, &mine, &p).unwrap();
        s.vault.update(second, &mine_second, &p).unwrap();

        let head_before = head(&s.vault);
        Fixture {
            s,
            identity,
            id: first,
            path: path_of(first),
            mine,
            theirs: their_first,
            second: Some((second, mine_second, their_second)),
            head_before,
        }
    }

    /// This device removes an entry the other device edits.
    fn delete_modify() -> Fixture {
        let s = SyncVault::new();
        let identity = s.unlock();
        let p = FakePrompter::new();
        let id = s
            .vault
            .insert(&identity, sample("ProtonMail"), false, &p)
            .unwrap();
        let original = base();
        s.vault.update(id, &original, &p).unwrap();
        let path = path_of(id);

        let mut theirs = original.clone();
        theirs.value = "their-rotated-password".into();
        theirs.updated_by = "phone".into();
        theirs.updated = at("2026-08-29T09:03:00Z");
        publish(&s, &[(&path, &theirs)]);

        s.vault.remove(&identity, &id.to_string(), &p).unwrap();
        let head_before = head(&s.vault);
        Fixture {
            s,
            identity,
            id,
            path,
            mine: Entry::default(),
            theirs,
            second: None,
            head_before,
        }
    }

    fn vault(&self) -> &Vault {
        &self.s.vault
    }

    /// The resolving sync with `p` answering and this fixture's identity
    /// as the unlock.
    fn resolve(&self, p: &dyn Prompter) -> Result<SyncReport, SyncError> {
        self.vault()
            .sync_resolving_with(p, &mut || Ok(&self.identity))
    }

    fn read(&self, id: Uuid) -> Entry {
        self.vault().read_entry(id, &self.identity).unwrap()
    }

    /// Every entry other than the fixture's own: keep both's fresh id.
    fn others(&self) -> Vec<Uuid> {
        let skip = [Some(self.id), self.second.as_ref().map(|s| s.0)];
        self.vault()
            .entry_ids()
            .unwrap()
            .into_iter()
            .filter(|id| !skip.contains(&Some(*id)))
            .collect()
    }

    fn assert_untouched(&self) {
        assert_eq!(head(self.vault()), self.head_before, "HEAD moved");
        assert!(
            gitrepo::is_clean(&self.vault().path).unwrap(),
            "{:?}",
            gitrepo::dirty_paths(&self.vault().path)
        );
        assert_eq!(self.read(self.id).value, self.mine.value);
        if let Some((id, mine, _)) = &self.second {
            assert_eq!(self.read(*id).value, mine.value);
        }
    }
}

fn head(v: &Vault) -> String {
    git2::Repository::open(&v.path)
        .unwrap()
        .head()
        .unwrap()
        .target()
        .unwrap()
        .to_string()
}

fn answering(answers: &[Resolution]) -> FakePrompter {
    FakePrompter::new().with_resolutions(answers)
}

fn answering_by_title(answers: &[(&str, Resolution)]) -> FakePrompter {
    answers.iter().fold(FakePrompter::new(), |p, (title, r)| {
        p.with_resolution_for(title, *r)
    })
}

/// Every commit reachable from HEAD, through merges: the structural stand-in
/// for `gage log`.
fn walk_history(dir: &Path) -> Vec<(String, usize, String)> {
    let repo = git2::Repository::open(dir).unwrap();
    let mut walk = repo.revwalk().unwrap();
    walk.push_head().unwrap();
    walk.map(|id| {
        let c = repo.find_commit(id.unwrap()).unwrap();
        (
            c.id().to_string(),
            c.parent_count(),
            c.message().unwrap_or_default().to_string(),
        )
    })
    .collect()
}

fn head_message(dir: &Path) -> String {
    let repo = git2::Repository::open(dir).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    head.message().unwrap().to_string()
}

fn tracking(dir: &Path) -> String {
    let repo = git2::Repository::open(dir).unwrap();
    repo.find_reference("refs/remotes/origin/main")
        .unwrap()
        .target()
        .unwrap()
        .to_string()
}

// --- resolution mechanics ---------------------------------------------------

#[test]
fn conflict_is_presented_with_both_sides_decrypted() {
    let f = Fixture::new();
    let p = answering(&[Resolution::KeepLocal]);
    f.resolve(&p).unwrap();

    let presented = p.conflicts_presented();
    assert_eq!(presented.len(), 1);
    let c = &presented[0];
    assert_eq!(c.id, f.id);
    assert_eq!(c.path, f.path);
    let (Some(local), Some(remote)) = (&c.local.entry, &c.remote.entry) else {
        panic!("both sides changed the entry, so both must be present: {c:?}");
    };
    assert!(c.local.present() && c.remote.present());
    assert_eq!(local.value, f.mine.value);
    assert_eq!(remote.value, f.theirs.value);
    assert_eq!(local.updated_by, DEVICE);
    assert_eq!(remote.updated_by, "phone");
    assert_eq!(local.updated, f.mine.updated);
    assert_eq!(remote.updated, f.theirs.updated);
}

#[test]
fn keep_local_keeps_the_local_version() {
    let f = Fixture::new();
    let report = f.resolve(&answering(&[Resolution::KeepLocal])).unwrap();
    assert_eq!(report.resolved, 1);
    assert_eq!(f.read(f.id).value, f.mine.value);
    assert!(f.others().is_empty(), "{:?}", f.others());
}

#[test]
fn keep_remote_replaces_the_local_version() {
    let f = Fixture::new();
    f.resolve(&answering(&[Resolution::KeepRemote])).unwrap();
    assert_eq!(f.read(f.id).value, f.theirs.value);
    assert!(f.others().is_empty(), "{:?}", f.others());
}

// No version of a secret is destroyed
// by resolving a conflict.
#[test]
fn keep_both_keeps_both_versions() {
    let f = Fixture::new();
    f.resolve(&answering(&[Resolution::KeepBoth])).unwrap();

    assert_eq!(f.read(f.id).value, f.mine.value, "the local version moved");
    let others = f.others();
    assert_eq!(others.len(), 1, "{others:?}");
    assert!(!others[0].is_nil());
    let kept = f.read(others[0]);
    assert_eq!(kept.value, f.theirs.value);
    assert_eq!(
        kept.title, f.theirs.title,
        "the title must be kept verbatim"
    );
    // Every field survives, not only the value.
    assert_eq!(kept, f.theirs);
}

#[test]
fn keep_both_preserves_the_losing_sides_provenance() {
    let f = Fixture::new();
    f.resolve(&answering(&[Resolution::KeepBoth])).unwrap();
    let others = f.others();
    assert_eq!(others.len(), 1);
    let kept = f.read(others[0]);
    assert_eq!(kept.updated_by, "phone", "restamped as this device");
    assert_eq!(kept.updated, f.theirs.updated, "restamped as now");
    assert_eq!(kept.created, f.theirs.created);
}

#[test]
fn keep_both_leaves_two_entries_the_resolver_disambiguates() {
    let f = Fixture::new();
    f.resolve(&answering(&[Resolution::KeepBoth])).unwrap();
    let err = f
        .vault()
        .resolve(&f.identity, &f.mine.title)
        .expect_err("one version is shadowing the other");
    let ResolveError::Ambiguous(ref list) = err else {
        panic!("{err}");
    };
    assert_eq!(list.candidates.len(), 2, "{list:?}");
    assert_eq!(err.kind(), ErrorKind::Ambiguous);
}

// Skip is not a
// silent keep-local.
#[test]
fn skip_leaves_the_entry_conflicted_and_does_not_push() {
    let f = Fixture::new();
    let remote_before = tracking(&f.vault().path);
    let err = f
        .resolve(&answering(&[Resolution::Skip]))
        .expect_err("a skipped conflict must be reported");
    assert!(!err.report.pushed && !err.report.merged, "{:?}", err.report);
    assert_eq!(err.report.skipped, 1);
    assert_eq!(err.report.resolved, 0);
    assert_eq!(err.kind(), ErrorKind::Conflict);
    // The divergence it declined to resolve is still there.
    assert_eq!(err.remote_kind(), Some(Kind::Diverged));
    assert!(err.to_string().contains("1 of 1"), "{err}");
    f.assert_untouched();
    assert_eq!(f.s.other().head(), remote_before, "something was pushed");
}

#[test]
fn skip_still_presents_the_remaining_conflicts() {
    let f = Fixture::two();
    let p = answering(&[Resolution::Skip, Resolution::Skip]);
    let err = f.resolve(&p).unwrap_err();
    assert_eq!(p.conflicts_presented().len(), 2);
    assert_eq!(err.report.skipped, 2);
    f.assert_untouched();
}

// A merge carrying only the
// answered conflict would mark the remote's history incorporated with the
// skipped entry at its local version: skip would become keep-local.
#[test]
fn one_skip_applies_none_of_the_other_answers() {
    let f = Fixture::two();
    let p = answering_by_title(&[
        ("ProtonMail", Resolution::KeepRemote),
        ("Bank", Resolution::Skip),
    ]);
    let err = f.resolve(&p).unwrap_err();
    assert!(!err.report.merged && !err.report.pushed, "{:?}", err.report);
    assert_eq!(err.report.resolved, 0);
    assert_eq!(err.report.skipped, 1);
    assert_eq!(p.conflicts_presented().len(), 2);
    f.assert_untouched();
}

#[test]
fn two_conflicts_take_their_own_answers() {
    let f = Fixture::two();
    let p = answering_by_title(&[
        ("ProtonMail", Resolution::KeepLocal),
        ("Bank", Resolution::KeepRemote),
    ]);
    let report = f.resolve(&p).unwrap();
    assert_eq!(report.resolved, 2);
    assert_eq!(p.conflicts_presented().len(), 2);
    let (second, _, their_second) = f.second.as_ref().unwrap();
    assert_eq!(f.read(f.id).value, f.mine.value);
    assert_eq!(f.read(*second).value, their_second.value);
    let merges = walk_history(&f.vault().path)
        .iter()
        .filter(|(_, parents, _)| *parents == 2)
        .count();
    assert_eq!(merges, 1, "one merge for two answers");
    assert!(head_message(&f.vault().path).contains("2 entries"));
}

// A frontend that fails
// mid-resolution is not an answer.
#[test]
fn a_prompter_error_leaves_the_vault_untouched() {
    let f = Fixture::new();
    let p = FakePrompter::new().with_resolution_error(PromptError::Io(std::io::Error::other(
        "the terminal went away",
    )));
    let err = f.resolve(&p).unwrap_err();
    assert!(
        matches!(*err.cause, SyncCause::Prompt(PromptError::Io(_))),
        "{err}"
    );
    assert!(err.to_string().contains("the terminal went away"), "{err}");
    assert!(!err.report.merged && !err.report.pushed);
    assert_eq!(err.report.resolved, 0);
    f.assert_untouched();
}

// Abort stops asking, and HEAD, the
// working tree and its cleanliness are as they were, with nothing pushed.
#[test]
fn abort_restores_the_pre_sync_state() {
    let f = Fixture::two();
    let remote_before = tracking(&f.vault().path);
    let p = answering(&[Resolution::Abort, Resolution::KeepRemote]);
    let err = f.resolve(&p).unwrap_err();
    assert!(err.report.aborted);
    assert!(!err.report.pushed && !err.report.merged);
    assert_eq!(err.report.skipped, 0, "an abort is not a skip of each");
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(p.conflicts_presented().len(), 1, "abort must stop asking");
    assert!(err.to_string().contains("aborted"), "{err}");
    f.assert_untouched();
    assert_eq!(f.vault().entry_ids().unwrap().len(), 2);
    assert_eq!(f.s.other().head(), remote_before, "something was pushed");
}

#[test]
fn delete_modify_conflict_shows_the_surviving_version() {
    let f = Fixture::delete_modify();
    let p = answering(&[Resolution::KeepRemote]);
    f.resolve(&p).unwrap();
    let c = &p.conflicts_presented()[0];
    assert!(!c.local.present(), "the local side deleted it");
    assert_eq!(
        c.remote.entry.as_ref().expect("the survivor").value,
        f.theirs.value
    );
    // Keeping the survivor brings the entry back, decryptable.
    assert_eq!(f.read(f.id).value, f.theirs.value);
}

#[test]
fn delete_modify_conflict_keeping_the_deletion() {
    let f = Fixture::delete_modify();
    f.resolve(&answering(&[Resolution::KeepLocal])).unwrap();
    assert!(!f.vault().path.join(&f.path).exists());
    assert!(gitrepo::is_clean(&f.vault().path).unwrap());
    assert!(f.vault().entry_ids().unwrap().is_empty());
}

// --- delete/modify keep-both, file modes, history ---------------------------

// The habitual
// key never destroys the one surviving copy.
#[test]
fn keep_both_on_a_delete_modify_conflict_keeps_the_survivor() {
    let f = Fixture::delete_modify();
    f.resolve(&answering(&[Resolution::KeepBoth])).unwrap();
    let ids = f.vault().entry_ids().unwrap();
    assert_eq!(ids.len(), 1, "{ids:?}");
    assert_ne!(ids[0], f.id, "restored at the deleted id, want a fresh one");
    let kept = f.read(ids[0]);
    assert_eq!(kept, f.theirs);
}

// And the mirror case: the remote deleted it, and keep
// both keeps the local edit where it is without inventing a second copy.
#[test]
fn keep_both_when_the_remote_deleted_keeps_the_local_edit() {
    let s = SyncVault::new();
    let identity = s.unlock();
    let p = FakePrompter::new();
    let id = s
        .vault
        .insert(&identity, sample("ProtonMail"), false, &p)
        .unwrap();
    let other = s.other();
    other.remove(&path_of(id));
    other.commit("their deletion");
    other.push();
    let mut mine = s.vault.read_entry(id, &identity).unwrap();
    mine.value = "my edit".into();
    s.vault.update(id, &mine, &p).unwrap();

    let p = answering(&[Resolution::KeepBoth]);
    s.vault
        .sync_resolving_with(&p, &mut || Ok(&identity))
        .unwrap();
    assert!(!p.conflicts_presented()[0].remote.present());
    assert_eq!(s.vault.entry_ids().unwrap(), [id]);
    assert_eq!(s.vault.read_entry(id, &identity).unwrap().value, "my edit");
}

#[cfg(unix)]
#[test]
fn entry_files_a_merge_creates_are_0600() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |v: &Vault, id: Uuid| {
        std::fs::metadata(v.path.join(path_of(id)))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };

    let f = Fixture::new();
    f.resolve(&answering(&[Resolution::KeepBoth])).unwrap();
    let others = f.others();
    assert_eq!(others.len(), 1);
    assert_eq!(mode(f.vault(), others[0]), 0o600, "keep both's new entry");

    let f = Fixture::delete_modify();
    f.resolve(&answering(&[Resolution::KeepRemote])).unwrap();
    assert_eq!(mode(f.vault(), f.id), 0o600, "an entry a merge restored");
}

#[test]
fn resolved_sync_produces_a_two_parent_merge_commit() {
    let f = Fixture::new();
    let remote_head = tracking(&f.vault().path);
    let report = f.resolve(&answering(&[Resolution::KeepBoth])).unwrap();
    assert!(report.merged && report.pushed, "{report:?}");
    assert_eq!(report.ahead, 0);

    let repo = git2::Repository::open(&f.vault().path).unwrap();
    let merge = repo.head().unwrap().peel_to_commit().unwrap();
    let parents: Vec<String> = merge.parent_ids().map(|p| p.to_string()).collect();
    assert_eq!(parents, [f.head_before.clone(), remote_head]);
    // And it reached the remote.
    assert_eq!(f.s.other().head(), merge.id().to_string());
}

#[test]
fn local_commit_granularity_survives_the_merge() {
    let f = Fixture::new();
    f.resolve(&answering(&[Resolution::KeepLocal])).unwrap();
    assert!(
        walk_history(&f.vault().path)
            .iter()
            .any(|(id, _, _)| *id == f.head_before),
        "the local commit is no longer reachable"
    );
}

// Structural, through a revwalk:
// `gitrepo::log` is tested separately.
#[test]
fn history_walks_through_a_merge_commit() {
    let f = Fixture::new();
    f.resolve(&answering(&[Resolution::KeepBoth])).unwrap();
    let commits = walk_history(&f.vault().path);
    assert!(commits.len() >= 3, "{commits:?}");
    let merges = commits.iter().filter(|(_, p, _)| *p == 2).count();
    assert_eq!(merges, 1);
    // Both sides' work is reachable: this side's edit and the other's.
    assert!(
        commits
            .iter()
            .any(|(_, _, m)| m == "the other device's edit")
    );
}

#[test]
fn merge_commit_message_names_the_count_and_no_plaintext() {
    let f = Fixture::new();
    f.resolve(&answering(&[Resolution::KeepBoth])).unwrap();
    let msg = head_message(&f.vault().path);
    assert_eq!(msg, "gage: merge origin, resolving 1 entry");
    for secret in [
        &f.mine.title,
        &f.mine.value,
        &f.theirs.value,
        &f.mine.description,
        &f.mine.updated_by,
        &f.theirs.updated_by,
    ] {
        assert!(!msg.contains(secret.as_str()), "{msg:?} leaks {secret:?}");
    }
    assert_eq!(
        resolution_message(2),
        "gage: merge origin, resolving 2 entries"
    );
}

// --- unlocking, locking and the other milestones ----------------------------

/// An unlock that counts its calls.
struct Counting<'a> {
    identity: &'a Identity,
    calls: AtomicUsize,
}

impl<'a> Counting<'a> {
    fn new(identity: &'a Identity) -> Counting<'a> {
        Counting {
            identity,
            calls: AtomicUsize::new(0),
        }
    }

    fn run(&self, v: &Vault, p: &dyn Prompter) -> Result<SyncReport, SyncError> {
        v.sync_resolving_with(p, &mut || {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.identity)
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

/// A local commit `v`'s push can't publish: what leaves a divergence to
/// find once the other device has pushed too.
fn commit_offline(s: &SyncVault, rel: &str, content: &str) {
    std::fs::write(s.vault.path.join(rel), content).unwrap();
    gitrepo::commit_all(&s.vault.path, "a local change").unwrap();
}

#[test]
fn clean_fast_forward_never_unlocks() {
    let s = SyncVault::new();
    let identity = s.unlock();
    s.other()
        .write_commit_push("notes.txt", "from elsewhere", "another device's commit");
    let unlock = Counting::new(&identity);
    let p = FakePrompter::new();
    let report = unlock.run(&s.vault, &p).unwrap();
    assert!(report.pulled, "{report:?}");
    assert_eq!(unlock.calls(), 0);
    assert!(p.conflicts_presented().is_empty());
}

#[test]
fn disjoint_divergence_never_unlocks() {
    let s = SyncVault::new();
    let identity = s.unlock();
    s.other()
        .write_commit_push("notes.txt", "the other device's file", "theirs");
    commit_offline(&s, "mine.txt", "mine");
    let unlock = Counting::new(&identity);
    let p = FakePrompter::new();
    let report = unlock.run(&s.vault, &p).unwrap();
    assert!(report.merged && report.pushed, "{report:?}");
    assert_eq!(report.resolved, 0);
    assert_eq!(unlock.calls(), 0);
    assert!(p.conflicts_presented().is_empty());
}

/// A prompter that runs `before` ahead of each conflict, delegating
/// everything to a [`FakePrompter`]: the seam for observing the world
/// mid-resolution.
struct Observing<'a> {
    inner: FakePrompter,
    before: Box<dyn Fn() + 'a>,
}

impl Prompter for Observing<'_> {
    fn unlock(&self, req: &UnlockRequest) -> Result<UnlockResponse, PromptError> {
        self.inner.unlock(req)
    }
    fn confirm(&self, prompt: &str) -> Result<bool, PromptError> {
        self.inner.confirm(prompt)
    }
    fn confirm_default_yes(&self, prompt: &str) -> Result<bool, PromptError> {
        self.inner.confirm_default_yes(prompt)
    }
    fn value(&self, title: &str) -> Result<zeroize::Zeroizing<String>, PromptError> {
        self.inner.value(title)
    }
    fn choose(&self, list: &crate::entry::CandidateList) -> Result<Uuid, PromptError> {
        self.inner.choose(list)
    }
    fn warn(&self, warning: &Warning) {
        self.inner.warn(warning)
    }
    fn resolve_conflict(&self, c: &EntryConflict) -> Result<Resolution, PromptError> {
        (self.before)();
        self.inner.resolve_conflict(c)
    }
}

#[test]
fn unlock_happens_only_when_a_conflict_is_reached() {
    let f = Fixture::new();
    let unlock = Counting::new(&f.identity);
    let p = Observing {
        inner: answering(&[Resolution::KeepLocal]),
        before: Box::new(|| {
            assert_eq!(unlock.calls(), 1, "presented before unlocking");
        }),
    };
    unlock.run(f.vault(), &p).unwrap();
    assert_eq!(unlock.calls(), 1);
}

// The one-shot form: the identity is unlocked through the closure on the
// spot, once, and owned for the run.
#[test]
fn one_shot_resolution_unlocks_once_through_its_closure() {
    let f = Fixture::new();
    let calls = AtomicUsize::new(0);
    let p = answering(&[Resolution::KeepBoth]);
    f.vault()
        .sync_resolving(&p, &mut || {
            calls.fetch_add(1, Ordering::SeqCst);
            f.vault().unlock(&unlocks())
        })
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.others().len(), 1);
}

// A failed lazy unlock is reported as itself, with nothing changed.
#[test]
fn a_failed_unlock_changes_nothing() {
    let f = Fixture::new();
    let p = answering(&[]);
    let err = f
        .vault()
        .sync_resolving(&p, &mut || {
            f.vault().unlock(
                &FakePrompter::new()
                    .with_passphrases(&["wrong", "wrong", "wrong"])
                    .with_unlock_error(PromptError::TooManyAttempts),
            )
        })
        .unwrap_err();
    assert!(matches!(*err.cause, SyncCause::Unlock(_)), "{err}");
    assert_eq!(err.kind(), ErrorKind::LockedOrAuth);
    assert!(p.conflicts_presented().is_empty());
    f.assert_untouched();
}

#[test]
fn a_non_entry_conflict_is_refused_without_unlocking() {
    let s = SyncVault::new();
    let identity = s.unlock();
    s.other()
        .write_commit_push("notes.txt", "their notes", "their change");
    commit_offline(&s, "notes.txt", "my notes");
    let before = head(&s.vault);

    let unlock = Counting::new(&identity);
    let p = answering(&[Resolution::KeepLocal]);
    let err = unlock.run(&s.vault, &p).unwrap_err();
    assert!(err.to_string().contains("notes.txt"), "{err}");
    assert!(err.to_string().contains("not an entry"), "{err}");
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(p.conflicts_presented().is_empty());
    assert_eq!(unlock.calls(), 0);
    assert!(!err.report.merged && !err.report.pushed);
    assert_eq!(head(&s.vault), before);

    // An entry conflict next to it doesn't reach the unlock either.
    assert!(entry_id_from_path("entries/not-a-uuid.age").is_none());
    assert!(entry_id_from_path("entries/sub/0e0b6c43-77b2-4d6a-9a3c-1c5d36a7e9a1.age").is_none());
    assert!(entry_id_from_path("entries/0E0B6C43-77B2-4D6A-9A3C-1C5D36A7E9A1.age").is_none());
    assert_eq!(
        entry_id_from_path("entries/0e0b6c43-77b2-4d6a-9a3c-1c5d36a7e9a1.age"),
        Some("0e0b6c43-77b2-4d6a-9a3c-1c5d36a7e9a1".parse().unwrap())
    );
}

// A recipient-file conflict is refused by sync, not routed to the entry
// resolver; the trust check owns what happens to those files.
#[test]
fn recipient_conflict_does_not_reach_the_entry_resolver() {
    let s = SyncVault::new();
    let identity = s.unlock();
    let recipients = std::fs::read_to_string(s.vault.path.join(".age-recipients")).unwrap();
    s.other().write_commit_push(
        ".age-recipients",
        &format!("{recipients}age1theirextrarecipient\n"),
        "their recipient change",
    );
    commit_offline(
        &s,
        ".age-recipients",
        &format!("{recipients}age1myextrarecipient\n"),
    );

    let unlock = Counting::new(&identity);
    let p = answering(&[Resolution::KeepLocal]);
    let err = unlock.run(&s.vault, &p).unwrap_err();
    assert!(matches!(*err.cause, SyncCause::Conflict), "{err}");
    assert_eq!(err.report.conflict_kind(), ConflictKind::Recipients);
    assert!(p.conflicts_presented().is_empty());
    assert_eq!(unlock.calls(), 0);
}

// A prompter that
// doesn't implement resolve_conflict is a script or CI: refused, with
// nothing chosen on the human's behalf.
#[test]
fn non_interactive_prompter_refuses_on_the_first_conflict() {
    /// Unlocks and warns; has nobody to ask about a conflict.
    struct Plain;
    impl Prompter for Plain {
        fn unlock(&self, _: &UnlockRequest) -> Result<UnlockResponse, PromptError> {
            unreachable!("the fixture's identity is already unlocked")
        }
        fn confirm(&self, _: &str) -> Result<bool, PromptError> {
            unreachable!()
        }
        fn confirm_default_yes(&self, _: &str) -> Result<bool, PromptError> {
            unreachable!()
        }
        fn value(&self, _: &str) -> Result<zeroize::Zeroizing<String>, PromptError> {
            unreachable!()
        }
        fn choose(&self, _: &crate::entry::CandidateList) -> Result<Uuid, PromptError> {
            unreachable!()
        }
        fn warn(&self, _: &Warning) {}
    }

    let f = Fixture::two();
    let err = f.resolve(&Plain).unwrap_err();
    assert!(
        matches!(*err.cause, SyncCause::Prompt(PromptError::NotInteractive)),
        "{err}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(err.to_string().contains("needs someone to ask"), "{err}");
    assert_eq!(err.report.resolved, 0);
    f.assert_untouched();

    // A FakePrompter with no resolutions scripted is the same frontend.
    let err = f.resolve(&FakePrompter::new()).unwrap_err();
    assert!(
        matches!(*err.cause, SyncCause::Prompt(PromptError::NotInteractive)),
        "{err}"
    );
    f.assert_untouched();
}

// A separate process probes
// the lock while a conflict waits for its answer, and again afterwards:
// file locks belong to the open file description, so a probe from a
// thread would prove nothing.
#[test]
fn resolution_holds_the_write_lock_throughout() {
    let f = Fixture::new();
    let lock = f.vault().lock_file_path();
    let seen = Mutex::new(None);
    let p = Observing {
        inner: answering(&[Resolution::KeepLocal]),
        before: Box::new(|| *seen.lock().unwrap() = Some(probe(&lock))),
    };
    f.resolve(&p).unwrap();
    assert_eq!(
        seen.lock().unwrap().as_deref(),
        Some("BUSY"),
        "the lock was free while a conflict waited for its answer"
    );
    assert_eq!(probe(&lock), "FREE", "still held after resolution");
}

const PROBE_ENV: &str = "GAGE_SYNCRESOLVE_TEST_PROBE";

/// The child side: try the lock at `$PROBE_ENV` once, without waiting,
/// and say whether it was free.
#[test]
#[allow(clippy::disallowed_macros)]
fn lock_probe_trampoline() {
    let Some(path) = std::env::var_os(PROBE_ENV) else {
        return;
    };
    match vaultlock::Lock::try_acquire(Path::new(&path)).unwrap() {
        Some(lock) => {
            lock.release().unwrap();
            eprintln!("FREE");
        }
        None => eprintln!("BUSY"),
    }
}

/// Runs [`lock_probe_trampoline`] in a child process against `lock`.
#[allow(clippy::disallowed_methods)]
fn probe(lock: &Path) -> String {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "syncresolve::tests::lock_probe_trampoline",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(PROBE_ENV, lock)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = BufReader::new(child.stderr.take().unwrap());
    let answer = stderr
        .lines()
        .map(|l| l.unwrap())
        .find(|l| l == "FREE" || l == "BUSY")
        .unwrap_or_default();
    child.wait().unwrap();
    answer
}

/// Fetches and pushes for real, recording the deadline each push was
/// given, so a test can see which window the publish ran in.
struct Recording {
    dir: std::path::PathBuf,
    push_deadlines: Mutex<Vec<Instant>>,
}

impl RemoteSyncer for Recording {
    fn fetch(&self, _: Instant) -> Result<bool, gitrepo::Error> {
        gitrepo::fetch(&self.dir, &crate::gittest::remote_options())
    }

    fn push(&self, deadline: Instant) -> Result<bool, gitrepo::Error> {
        self.push_deadlines.lock().unwrap().push(deadline);
        gitrepo::push(&self.dir, &crate::gittest::remote_options())
    }
}

// ADR-0018: the push after a resolution starts
// a fresh REMOTE_OP_TIMEOUT window from after the last answer. The window
// a sync starts with bounds network work, and a human weighing up two
// versions of a password sits in the middle of it: reusing it would fail
// to publish a merge already committed. Proved with the deadline the push
// is handed, not with real time.
#[test]
fn the_push_after_resolution_starts_a_fresh_deadline() {
    let f = Fixture::new();
    let syncer = std::sync::Arc::new(Recording {
        dir: f.vault().path.clone(),
        push_deadlines: Mutex::new(Vec::new()),
    });
    let vault = f.vault().clone().with_remote_syncer(syncer.clone());
    let answered = Mutex::new(None);
    let p = Observing {
        inner: answering(&[Resolution::KeepLocal]),
        before: Box::new(|| *answered.lock().unwrap() = Some(Instant::now())),
    };
    let report = vault
        .sync_resolving_with(&p, &mut || Ok(&f.identity))
        .unwrap();
    assert!(report.pushed, "{report:?}");

    let deadlines = syncer.push_deadlines.lock().unwrap().clone();
    assert_eq!(deadlines.len(), 2, "the rejected push, then the publish");
    let answered = answered.lock().unwrap().unwrap();
    assert!(
        deadlines[0] < answered + REMOTE_OP_TIMEOUT,
        "the first push should run in the sync's own window"
    );
    assert!(
        deadlines[1] >= answered + REMOTE_OP_TIMEOUT,
        "the publish reused a window that started before the human answered"
    );
}

// The resolution report keeps what the pull found.
#[test]
fn a_resolution_report_says_it_diverged_and_how_much_it_resolved() {
    let f = Fixture::new();
    let report = f.resolve(&answering(&[Resolution::KeepRemote])).unwrap();
    assert!(
        report.diverged && report.merged && report.pushed,
        "{report:?}"
    );
    assert_eq!(report.resolved, 1);
    assert_eq!(report.conflicts, std::slice::from_ref(&f.path));
    assert!(!report.aborted);
}
