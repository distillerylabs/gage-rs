//! A session's metadata index meeting sync: entries that arrive by pull
//! show up in the next `ls`, and a pull that brings nothing leaves the
//! cache alone.

use crate::entry::{Entry, Timestamp};
use crate::prompter::FakePrompter;
use crate::session::{Session, SessionConfig, SystemClock};
use crate::sync::remotesync::tests::{NAME, PASSPHRASE, SyncVault, sample};
use crate::sync::syncresolve::Resolution;

/// A session over the synced vault, the way the REPL wires one.
fn new_session<'a>(s: &'a SyncVault, p: &'a FakePrompter) -> Session<'a> {
    Session::new(SessionConfig {
        open: Box::new(move |_| Ok(s.vault.clone())),
        prompter: p,
        idle_timeout: None,
        current: Some(NAME.into()),
        clock: &SystemClock,
    })
}

/// An entry another device writes: real ciphertext for this vault's
/// recipients, committed and pushed from the other clone, which can't
/// read it but doesn't need to.
fn other_device_adds_an_entry(s: &SyncVault) {
    let other = s.other();
    let mut there = s.vault.clone();
    there.path = other.path().to_path_buf();
    let id = crate::entry::new_id();
    let now = Timestamp::now();
    there
        .write_entry(
            id,
            &Entry {
                title: "Written elsewhere".into(),
                value: "v".into(),
                created: Some(now),
                updated: Some(now),
                ..Entry::default()
            },
        )
        .unwrap();
    other.commit(&id.to_string());
    other.push();
}

// `use` on a vault the session already
// holds unlocked runs no Vault::unlock, so it has to ask for the pull.
#[test]
fn use_fetches_and_fast_forwards() {
    let s = SyncVault::new();
    let p = FakePrompter::new().with_passphrase(PASSPHRASE);
    let mut session = new_session(&s, &p);
    session.use_vault(NAME).unwrap();

    s.other()
        .write_commit_push("notes.txt", "from elsewhere", "another device's commit");
    session.use_vault(NAME).unwrap();

    assert!(
        s.vault.path.join("notes.txt").exists(),
        "`use` on an unlocked vault did not fast-forward it"
    );
    assert_eq!(p.unlock_requests().len(), 1, "re-selecting re-prompted");
}

// The index is built first,
// so a stale cache surviving the pull would fail the second count.
#[test]
fn pull_during_a_session_invalidates_the_index() {
    let s = SyncVault::new();
    let identity = s.unlock();
    s.vault
        .insert(&identity, sample("ProtonMail"), false, &FakePrompter::new())
        .unwrap();
    drop(identity);

    let p = FakePrompter::new().with_passphrase(PASSPHRASE);
    let mut session = new_session(&s, &p);
    assert_eq!(session.list(None).unwrap().len(), 1);

    other_device_adds_an_entry(&s);
    session.use_vault(NAME).unwrap();

    let titles: Vec<String> = session
        .list(None)
        .unwrap()
        .into_iter()
        .map(|r| r.title)
        .collect();
    assert_eq!(
        titles,
        ["ProtonMail", "Written elsewhere"],
        "the index survived a pull that added an entry"
    );
}

// Rebuilding on every
// unlock would defeat the cache.
#[test]
fn pull_that_brings_nothing_leaves_the_index_alone() {
    let s = SyncVault::new();
    let identity = s.unlock();
    s.vault
        .insert(&identity, sample("ProtonMail"), false, &FakePrompter::new())
        .unwrap();
    drop(identity);

    let p = FakePrompter::new().with_passphrase(PASSPHRASE);
    let mut session = new_session(&s, &p);
    session.list(None).unwrap();
    let after_first_build = s.vault.decrypt_count();
    assert!(after_first_build > 0, "the index was never built");

    session.use_vault(NAME).unwrap();
    session.list(None).unwrap();

    assert_eq!(
        s.vault.decrypt_count(),
        after_first_build,
        "the index was rebuilt across a pull that brought nothing"
    );
}

// A merge inside a write's own push moves the tree too,
// and the index must not then serve the cached rows (nor have the
// write's own entry patched into them).
#[test]
fn a_merge_inside_a_sessions_write_invalidates_the_index() {
    let s = SyncVault::new();
    let p = FakePrompter::new().with_passphrase(PASSPHRASE);
    let mut session = new_session(&s, &p);
    assert!(session.list(None).unwrap().is_empty());

    other_device_adds_an_entry(&s);
    let id = {
        let (vault, identity) = session.vault(None).unwrap();
        vault.insert(identity, sample("Mine"), false, &p).unwrap()
    };
    session.refresh_entry(None, id);

    let titles: Vec<String> = session
        .list(None)
        .unwrap()
        .into_iter()
        .map(|r| r.title)
        .collect();
    assert_eq!(titles, ["Mine", "Written elsewhere"]);
}

// And the sync verbs reach the vault without unlocking it, while a pull
// through them still invalidates the index.
#[test]
fn a_sync_verb_needs_no_unlock_and_still_invalidates_the_index() {
    let s = SyncVault::new();
    let p = FakePrompter::new().with_passphrase(PASSPHRASE);
    let mut session = new_session(&s, &p);
    session.list(None).unwrap();
    assert_eq!(p.unlock_requests().len(), 1);

    other_device_adds_an_entry(&s);
    let report = session
        .vault_without_unlocking(None)
        .unwrap()
        .pull()
        .unwrap();
    assert!(report.pulled, "{report:?}");
    assert_eq!(session.list(None).unwrap().len(), 1);
    assert_eq!(p.unlock_requests().len(), 1, "a sync verb unlocked");

    // On a session that never unlocked anything, too.
    let fresh = FakePrompter::new();
    let mut locked = new_session(&s, &fresh);
    locked
        .vault_without_unlocking(None)
        .unwrap()
        .pull()
        .unwrap();
    assert!(fresh.unlock_requests().is_empty());
}

/// One entry both sides changed, with this side's version committed and
/// the divergence found: the session tests' version of the resolution
/// fixture. Returns the entry's id.
fn diverge_on_one_entry(s: &SyncVault) -> uuid::Uuid {
    let identity = s.unlock();
    let p = FakePrompter::new();
    let id = s
        .vault
        .insert(&identity, sample("ProtonMail"), false, &p)
        .unwrap();
    let mut theirs = s.vault.read_entry(id, &identity).unwrap();
    theirs.value = "theirs".into();
    theirs.updated_by = "phone".into();
    let other = s.other();
    other.write_bytes(
        &format!("entries/{id}.age"),
        &s.vault.encrypt_entry(&theirs).unwrap(),
    );
    other.commit("their edit");
    other.push();
    let mut mine = s.vault.read_entry(id, &identity).unwrap();
    mine.value = "mine".into();
    s.vault.update(id, &mine, &p).unwrap();
    id
}

// An unlocked session
// resolves without a second passphrase request.
#[test]
fn resolution_reuses_a_sessions_cached_identity() {
    let s = SyncVault::new();
    let id = diverge_on_one_entry(&s);
    let p = FakePrompter::new()
        .with_passphrase(PASSPHRASE)
        .with_resolutions(&[Resolution::KeepRemote]);
    let mut session = new_session(&s, &p);
    session.vault(None).unwrap();
    assert_eq!(p.unlock_requests().len(), 1);

    let report = session.sync_resolving(None).unwrap().unwrap();
    assert_eq!(report.resolved, 1);
    assert_eq!(p.unlock_requests().len(), 1, "resolution re-prompted");
    assert_eq!(p.conflicts_presented().len(), 1);
    let (vault, identity) = session.vault(None).unwrap();
    assert_eq!(vault.read_entry(id, identity).unwrap().value, "theirs");
}

// A locked session unlocks once, when the conflict is reached, and keeps
// the key like any other unlock; a sync with nothing to decrypt asks for
// nothing.
#[test]
fn a_locked_session_unlocks_only_for_a_conflict_and_keeps_the_key() {
    let s = SyncVault::new();
    s.other()
        .write_commit_push("notes.txt", "from elsewhere", "another device's commit");
    let p = FakePrompter::new();
    let mut session = new_session(&s, &p);
    let report = session.sync_resolving(None).unwrap().unwrap();
    assert!(report.pulled, "{report:?}");
    assert!(p.unlock_requests().is_empty(), "a fast-forward unlocked");
    assert!(!session.status()[0].unlocked);

    diverge_on_one_entry(&s);
    let p = FakePrompter::new()
        .with_passphrase(PASSPHRASE)
        .with_resolutions(&[Resolution::KeepLocal]);
    let mut session = new_session(&s, &p);
    session.sync_resolving(None).unwrap().unwrap();
    assert_eq!(p.unlock_requests().len(), 1);
    assert!(session.status()[0].unlocked, "the key was not kept");
    session.vault(None).unwrap();
    assert_eq!(p.unlock_requests().len(), 1, "the kept key was not reused");
}

// The index is built first, so
// a stale cache surviving the resolution would still count one entry.
#[test]
fn resolution_updates_the_session_index() {
    let s = SyncVault::new();
    diverge_on_one_entry(&s);
    let p = FakePrompter::new()
        .with_passphrase(PASSPHRASE)
        .with_resolutions(&[Resolution::KeepBoth]);
    let mut session = new_session(&s, &p);
    assert_eq!(session.list(None).unwrap().len(), 1);

    session.sync_resolving(None).unwrap().unwrap();

    assert_eq!(session.list(None).unwrap().len(), 2, "the index went stale");
}
