//! A session's metadata index meeting sync: entries that arrive by pull
//! show up in the next `ls`, and a pull that brings nothing leaves the
//! cache alone. Go's `sync_index_test.go`; each test names its row.

use crate::entry::{Entry, Timestamp};
use crate::prompter::FakePrompter;
use crate::remotesync::tests::{NAME, PASSPHRASE, SyncVault, sample};
use crate::session::{Session, SessionConfig};

/// A session over the synced vault, the way the REPL wires one (Go's
/// `newSyncSession`).
fn new_session<'a>(s: &'a SyncVault, p: &'a FakePrompter) -> Session<'a> {
    Session::new(SessionConfig {
        open: Box::new(move |_| Ok(s.vault.clone())),
        prompter: p,
        idle_timeout: None,
        current: Some(NAME.into()),
        now: None,
    })
}

/// An entry another device writes: real ciphertext for this vault's
/// recipients, committed and pushed from the other clone, which can't
/// read it but doesn't need to (Go's `encryptForTest`).
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

// Go: TestUseFetchesAndFastForwards. `use` on a vault the session already
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

// Go: TestPullDuringASessionInvalidatesTheIndex. The index is built first,
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

// Go: TestPullThatBringsNothingLeavesTheIndexAlone. Rebuilding on every
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

// Beyond Go's rows: a merge inside a write's own push moves the tree too,
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
