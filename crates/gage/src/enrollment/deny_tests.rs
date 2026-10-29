//! `deny_enrollment`, and pruning riding the ordinary recipient writes.

use std::sync::Arc;

use super::approve_atomic_tests::lay_stale_request;
use super::approve_tests::{commits, enrolled, head, pending_names};
use super::fixtures_tests::{assert_enrollment_error, hours, new_pubkey, vault};
use super::*;
use crate::exitcode::Code;
use crate::prompter::FakePrompter;
use crate::remotesync::tests::{FakeSyncer, SyncVault, unreachable_error};

/// One hex digit every pending id carries, so it matches all of them.
fn common_substring(v: &Vault) -> String {
    let pending = v.pending_enrollments().unwrap();
    "0123456789abcdef"
        .chars()
        .find(|c| pending.iter().all(|r| r.id.replace('-', "").contains(*c)))
        .expect("no hex digit common to every pending id")
        .to_string()
}

#[test]
fn deny_removes_the_request_and_grants_nothing() {
    let a = SyncVault::new();
    let (_m, req) = enrolled(&a, "phone-1");
    let before = a.vault.recipients().unwrap();
    let head_before = head(&a.vault);

    let denied = a
        .vault
        .deny_enrollment(req.id.as_deref().unwrap(), &FakePrompter::new())
        .unwrap();

    assert_eq!(Some(&denied.id), req.id.as_ref());
    assert!(pending_names(&a.vault).is_empty());
    assert_eq!(a.vault.recipients().unwrap(), before);
    assert_ne!(
        head(&a.vault),
        head_before,
        "the removal was never committed"
    );
    assert!(gitrepo::is_clean(&a.vault.path).unwrap());
}

// Deny deletes, so an ambiguous
// id is the candidates, never a guess.
#[test]
fn deny_on_an_ambiguous_id_deletes_nothing() {
    let a = SyncVault::new();
    enrolled(&a, "phone-1");
    enrolled(&a, "phone-2");
    assert_eq!(pending_names(&a.vault).len(), 2);

    let err = assert_enrollment_error!(
        a.vault
            .deny_enrollment(&common_substring(&a.vault), &FakePrompter::new()),
        EnrollmentError::Ambiguous(_),
        Code::Ambiguous
    );
    let EnrollmentError::Ambiguous(candidates) = err else {
        unreachable!()
    };
    assert_eq!(candidates.matches.len(), 2);
    assert_eq!(pending_names(&a.vault).len(), 2);
}

#[test]
fn deny_on_an_unmatched_id_deletes_nothing() {
    let a = SyncVault::new();
    enrolled(&a, "phone-1");
    let before = commits(&a.vault);

    assert_enrollment_error!(
        a.vault.deny_enrollment("ffffffff", &FakePrompter::new()),
        EnrollmentError::NoSuchRequest { .. },
        Code::NotFound
    );
    assert_eq!(pending_names(&a.vault).len(), 1);
    assert_eq!(commits(&a.vault), before);
}

// The form `recipient pending`
// shows.
#[test]
fn deny_resolves_an_eight_character_prefix() {
    let a = SyncVault::new();
    let (_m, req) = enrolled(&a, "phone-1");

    a.vault
        .deny_enrollment(&req.id.as_deref().unwrap()[..8], &FakePrompter::new())
        .unwrap();

    assert!(pending_names(&a.vault).is_empty());
}

#[test]
fn deny_prunes_expired_requests_in_the_same_commit() {
    let a = SyncVault::new();
    let (_m, req) = enrolled(&a, "phone-1");
    let stale = lay_stale_request(&a.vault, hours(-1));
    let beyond = lay_stale_request(&a.vault, hours(2 * 7 * 24));
    let before = commits(&a.vault);

    a.vault
        .deny_enrollment(req.id.as_deref().unwrap(), &FakePrompter::new())
        .unwrap();

    for name in [&stale, &beyond] {
        assert!(
            !a.vault.pending_dir().join(name).exists(),
            "{name} survived"
        );
    }
    assert_eq!(commits(&a.vault), before + 1);
}

// A silent failed deny is
// indistinguishable from a successful one, and leaves the request live
// for every other device.
#[test]
fn deny_warns_when_its_push_fails() {
    let a = SyncVault::new();
    let (_m, req) = enrolled(&a, "phone-1");
    let offline = a.vault.clone().with_remote_syncer(Arc::new(FakeSyncer {
        push_err: Some(unreachable_error()),
        ..FakeSyncer::default()
    }));
    let p = FakePrompter::new();

    offline
        .deny_enrollment(req.id.as_deref().unwrap(), &p)
        .unwrap();

    let warnings = p.warnings().join("\n");
    assert!(
        warnings.contains("still pending for every other device"),
        "{warnings}"
    );
    assert!(
        pending_names(&a.vault).is_empty(),
        "the local removal didn't stand"
    );
}

// Pruning rides "the next
// recipient change", with no enrollment involved.
#[test]
fn recipient_add_prunes_expired_requests() {
    let f = vault();
    let stale = lay_stale_request(&f.vault, hours(-1));
    let beyond = lay_stale_request(&f.vault, hours(2 * 7 * 24));
    let before = commits(&f.vault);

    f.vault
        .add_recipient(
            "desktop-1",
            &new_pubkey(),
            &f.identity(),
            &FakePrompter::new(),
        )
        .unwrap();

    for name in [&stale, &beyond] {
        assert!(
            !f.vault.pending_dir().join(name).exists(),
            "{name} survived"
        );
    }
    assert_eq!(commits(&f.vault), before + 1);
}

// "The next recipient
// change" is not "the next addition".
#[test]
fn recipient_remove_prunes_expired_requests() {
    let f = vault();
    f.vault
        .add_recipient(
            "desktop-1",
            &new_pubkey(),
            &f.identity(),
            &FakePrompter::new(),
        )
        .unwrap();
    let stale = lay_stale_request(&f.vault, hours(-1));
    let before = commits(&f.vault);

    f.vault
        .remove_recipient("desktop-1", true, &f.identity(), &FakePrompter::new())
        .unwrap();

    assert!(!f.vault.pending_dir().join(&stale).exists());
    assert_eq!(commits(&f.vault), before + 1);
}
