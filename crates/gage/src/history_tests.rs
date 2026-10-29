//! `Vault::log` and `Vault::history`.

use uuid::Uuid;

use crate::entry::{Entry, NewEntry};
use crate::exitcode::{Code, HasCode};
use crate::history::HistoryError;
use crate::identity::Identity;
use crate::prompter::FakePrompter;
use crate::recipient_tests::{Fixture, head_hash, new_key};

fn insert(f: &Fixture, title: &str, value: &str) -> Uuid {
    f.vault
        .insert(
            &f.identity(),
            NewEntry {
                title: title.into(),
                value: value.into(),
                ..NewEntry::default()
            },
            false,
            &FakePrompter::new(),
        )
        .unwrap()
}

/// Rewrites `id`'s value, which commits one revision.
fn set_value(f: &Fixture, id: Uuid, value: &str) {
    let mut e: Entry = f.vault.read_entry(id, &f.identity()).unwrap();
    e.value = value.into();
    f.vault.update(id, &e, &FakePrompter::new()).unwrap();
}

fn remove(f: &Fixture, id: Uuid) {
    f.vault
        .remove(&f.identity(), &id.to_string(), &FakePrompter::new())
        .unwrap();
}

#[test]
fn log_reports_one_revision_per_write() {
    let f = Fixture::new();
    let id = insert(&f, "Account", "v1");
    let first = head_hash(&f.vault.path);
    set_value(&f, id, "v2");
    let second = head_hash(&f.vault.path);
    // Another entry's write is not this entry's revision.
    insert(&f, "Other", "x");

    let log = f.vault.log(id).unwrap();
    let hashes: Vec<_> = log.iter().map(|l| l.hash.as_str()).collect();
    assert_eq!(hashes, [second, first], "newest first");
    assert!(log.iter().all(|l| !l.deleted));
    assert!(log[0].when >= log[1].when);
}

#[test]
fn log_marks_the_deleting_commit() {
    let f = Fixture::new();
    let id = insert(&f, "Doomed", "v1");
    remove(&f, id);
    let deleting = head_hash(&f.vault.path);

    let log = f.vault.log(id).unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].hash, deleting);
    assert!(log[0].deleted);
    assert!(!log[1].deleted);
}

#[test]
fn log_on_an_entry_git_has_never_seen_is_empty_not_an_error() {
    let f = Fixture::new();
    insert(&f, "Account", "v1");
    assert!(f.vault.log(Uuid::new_v4()).unwrap().is_empty());
}

#[test]
fn history_decrypts_every_past_revision() {
    let f = Fixture::new();
    let id = insert(&f, "Account", "v1");
    set_value(&f, id, "v2");
    set_value(&f, id, "v3");

    let history = f.vault.history(id, &f.identity()).unwrap();
    let values: Vec<_> = history
        .iter()
        .map(|r| r.entry.as_ref().unwrap().value.as_str())
        .collect();
    assert_eq!(values, ["v1", "v2", "v3"], "oldest first");
    let log = f.vault.log(id).unwrap();
    let hashes: Vec<_> = history.iter().rev().map(|r| r.hash.as_str()).collect();
    assert_eq!(
        hashes,
        log.iter().map(|l| l.hash.as_str()).collect::<Vec<_>>()
    );
    assert!(history.iter().all(|r| !r.deleted));
}

#[test]
fn history_on_a_deleted_entry_still_walks_it() {
    let f = Fixture::new();
    let id = insert(&f, "Doomed", "v1");
    set_value(&f, id, "v2");
    remove(&f, id);

    let history = f.vault.history(id, &f.identity()).unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history[1].entry.as_ref().unwrap().value, "v2");
    let last = &history[2];
    assert!(last.deleted);
    assert!(last.entry.is_none());
}

// A device
// admitted later can't read what was written before it was a recipient;
// the recipient add re-encrypts, so only the pre-admission revision is out
// of its reach.
#[test]
fn history_fails_rather_than_skipping_an_undecryptable_revision() {
    let f = Fixture::new();
    let id = insert(&f, "Account", "v1");
    let before = head_hash(&f.vault.path);
    let newcomer = new_key();
    f.vault
        .add_recipient(
            "phone-1",
            &newcomer.to_public().to_string(),
            &f.identity(),
            &FakePrompter::new(),
        )
        .unwrap();

    // The original device reads everything (the re-encrypted revision too).
    assert_eq!(f.vault.history(id, &f.identity()).unwrap().len(), 2);

    let err = f
        .vault
        .history(id, &Identity::for_tests(&newcomer))
        .unwrap_err();
    let HistoryError::Decrypt {
        hash, id: named, ..
    } = &err
    else {
        panic!("not a decrypt error: {err}");
    };
    assert_eq!(hash, &before[..7]);
    assert_eq!(*named, id);
    assert_eq!(err.code(), Code::LockedOrAuth);
    assert!(err.to_string().contains(&id.to_string()), "{err}");
}
