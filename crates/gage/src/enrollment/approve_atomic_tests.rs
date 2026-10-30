//! Approval's atomicity and its two questions, and how it meets
//! the trust cache.

use super::approve_tests::{
    approve_as, approve_one, at_head, authorize, codes_of, enrolled, head, insert_as,
    key_listed_as, opened_for, pending_names,
};
use super::fixtures_tests::{assert_enrollment_error, good_payload, hand_seal, hours, new_pubkey};
use super::name::pending_file_name;
use super::seal::Payload;
use super::*;
use crate::error::ErrorKind;
use crate::prompter::FakePrompter;
use crate::storage::vaultconfig;
use crate::sync::remotesync::tests::{SyncVault, sample, unlocks};
use crate::trust::recipient::RecipientError;
use crate::vault::reencrypt_tests::{crashed, crashing_after};
use crate::vault::twodevice_tests::{B_PASSPHRASE, Machine};

/// Lists `pubkey` as `device` in both recipient files and commits, without
/// re-encrypting or reviewing: another writer's change, as this device's
/// trust cache sees it.
fn list_by_hand(a: &SyncVault, device: &str, pubkey: &str) {
    let mut list = a.vault.recipients().unwrap();
    list.push(vaultconfig::Recipient {
        device: device.into(),
        pubkey: pubkey.into(),
    });
    a.vault.write_recipient_files(&list).unwrap();
    gitrepo::commit_all(&a.vault.path, "test: another writer's recipient change").unwrap();
}

/// Commits a request nobody will honour, under a filename expiring at
/// `expires`, and returns its name.
pub(crate) fn lay_stale_request(v: &Vault, expires: jiff::Timestamp) -> String {
    let name = pending_file_name(&uuid::Uuid::new_v4().to_string(), expires);
    super::fixtures_tests::write_pending(v, &name, b"not a seal anyone will ever open");
    gitrepo::commit_all(&v.path, "test: a request nobody will honour").unwrap();
    name
}

// The same filename and code, a different key under the seal, swapped in
// after the open. An approval that trusted what it was handed would admit
// a key nobody saw; one that writes from the re-open refuses.
#[test]
fn approval_writes_what_it_verified_under_the_lock_not_what_was_displayed() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let opened = opened_for(&a.vault, &req.code);

    let swapped = new_pubkey();
    let payload = good_payload(
        &a.vault,
        req.id.as_deref().unwrap(),
        "phone-1",
        &swapped,
        req.created,
        req.expires,
    );
    super::fixtures_tests::write_pending(
        &a.vault,
        &pending_file_name(req.id.as_deref().unwrap(), req.expires),
        &hand_seal(&payload, &req.code),
    );
    gitrepo::commit_all(
        &a.vault.path,
        "test: the request, swapped after it was shown",
    )
    .unwrap();
    let before = head(&a.vault);

    assert_enrollment_error!(
        approve_as(
            &a,
            &[Approval::new(opened.clone())],
            &codes_of(&[&req]),
            &unlocks()
        ),
        EnrollmentError::NoLongerPending { .. },
        ErrorKind::NotFound
    );
    assert_eq!(
        key_listed_as(&a.vault, &swapped),
        None,
        "wrote what wasn't shown"
    );
    assert_eq!(
        key_listed_as(&a.vault, &opened.pubkey),
        None,
        "wrote what isn't there"
    );
    assert_eq!(head(&a.vault), before);
}

/// Replaces the request `req` was filed as with `payload` sealed under its
/// code and filed as `name`, and commits: a change made between the open
/// and the lock, by the code-holder or a git writer who has the code.
fn reseal_as(a: &SyncVault, req: &EnrollmentRequest, name: &str, payload: &Payload) {
    let filed = a
        .vault
        .pending_dir()
        .join(pending_file_name(req.id.as_deref().unwrap(), req.expires));
    std::fs::remove_file(filed).unwrap();
    super::fixtures_tests::write_pending(&a.vault, name, &hand_seal(payload, &req.code));
    gitrepo::commit_all(
        &a.vault.path,
        "test: the request, changed after it was shown",
    )
    .unwrap();
}

// Found in review. The swap above changes
// the key; this one keeps it and changes only the name, which a check
// comparing keys alone would pass and then record under a name nobody was
// shown. Consent was given for what was rendered, every field of it.
#[test]
fn approval_refuses_a_request_whose_device_name_changed_after_it_was_shown() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let opened = opened_for(&a.vault, &req.code);
    let renamed = good_payload(
        &a.vault,
        req.id.as_deref().unwrap(),
        "phone-2",
        &opened.pubkey,
        req.created,
        req.expires,
    );
    reseal_as(
        &a,
        &req,
        &pending_file_name(req.id.as_deref().unwrap(), req.expires),
        &renamed,
    );
    let before = head(&a.vault);

    assert_enrollment_error!(
        approve_as(
            &a,
            &[Approval::new(opened.clone())],
            &codes_of(&[&req]),
            &unlocks()
        ),
        EnrollmentError::NoLongerPending { .. },
        ErrorKind::NotFound
    );
    assert_eq!(key_listed_as(&a.vault, &opened.pubkey), None);
    assert_eq!(head(&a.vault), before);
}

// Found in review. The commit deletes the
// file the re-open read, not the name the sealed expiry implies: the
// filename's epoch is unauthenticated, so a rebuilt path can miss, and an
// approval that missed would grant access and leave the request pending.
#[test]
fn approval_deletes_the_file_it_verified_not_the_name_its_seal_implies() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let opened = opened_for(&a.vault, &req.code);
    let payload = good_payload(
        &a.vault,
        req.id.as_deref().unwrap(),
        "phone-1",
        &opened.pubkey,
        req.created,
        req.expires,
    );
    let refiled = pending_file_name(req.id.as_deref().unwrap(), hours(1));
    assert_ne!(
        refiled,
        pending_file_name(req.id.as_deref().unwrap(), req.expires)
    );
    reseal_as(&a, &req, &refiled, &payload);

    approve_as(
        &a,
        &[Approval::new(opened.clone())],
        &codes_of(&[&req]),
        &unlocks(),
    )
    .unwrap();

    assert_eq!(
        key_listed_as(&a.vault, &opened.pubkey).as_deref(),
        Some("phone-1")
    );
    assert_eq!(pending_names(&a.vault), Vec::<String>::new());
    assert!(!at_head(&a.vault, &format!("{PENDING_DIR}/{refiled}")));
}

// Found in review. A code that opens the
// request under the lock and is refused there answers with that refusal,
// not `NoLongerPending`: "approve it in the other vault" and "another
// device resolved it" send a human to different places.
#[test]
fn a_refusal_under_the_lock_is_reported_as_itself() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let opened = opened_for(&a.vault, &req.code);
    let mut elsewhere = good_payload(
        &a.vault,
        req.id.as_deref().unwrap(),
        "phone-1",
        &opened.pubkey,
        req.created,
        req.expires,
    );
    elsewhere.vault_id = uuid::Uuid::new_v4().to_string();
    reseal_as(
        &a,
        &req,
        &pending_file_name(req.id.as_deref().unwrap(), req.expires),
        &elsewhere,
    );
    let before = head(&a.vault);

    assert_enrollment_error!(
        approve_as(
            &a,
            &[Approval::new(opened.clone())],
            &codes_of(&[&req]),
            &unlocks()
        ),
        EnrollmentError::WrongVault { .. },
        ErrorKind::Conflict
    );
    assert_eq!(key_listed_as(&a.vault, &opened.pubkey), None);
    assert_eq!(head(&a.vault), before);
}

// Found in review. The re-open finds the
// file by its id without the liveness filter, so a request that expired
// between the open and the lock is `Expired`, as its seal says, rather
// than missing.
#[test]
fn a_request_that_expired_in_the_window_is_reported_as_expired() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let opened = opened_for(&a.vault, &req.code);
    let lapsed = good_payload(
        &a.vault,
        req.id.as_deref().unwrap(),
        "phone-1",
        &opened.pubkey,
        hours(-2),
        hours(-1),
    );
    reseal_as(
        &a,
        &req,
        &pending_file_name(req.id.as_deref().unwrap(), hours(-1)),
        &lapsed,
    );
    let before = head(&a.vault);

    assert_enrollment_error!(
        approve_as(
            &a,
            &[Approval::new(opened.clone())],
            &codes_of(&[&req]),
            &unlocks()
        ),
        EnrollmentError::Expired { .. },
        ErrorKind::Conflict
    );
    assert_eq!(key_listed_as(&a.vault, &opened.pubkey), None);
    assert_eq!(head(&a.vault), before);
}

// A crash
// mid-re-encryption commits nothing and clears nothing, on disk or at
// HEAD, which is what another device would see.
#[test]
fn interrupted_approval_leaves_head_untouched_and_requests_pending() {
    let a = SyncVault::new();
    for title in ["first", "second", "third"] {
        insert_as(&a, title);
    }
    let (_m, req) = enrolled(&a, "phone-1");
    let opened = opened_for(&a.vault, &req.code);
    let before = head(&a.vault);
    let pending_before = pending_names(&a.vault);
    let file = format!("{PENDING_DIR}/{}", pending_before[0]);

    let dying = crashing_after(&a.vault, 2);
    let identity = a.unlock();
    let died = crashed(|| {
        let _ = dying.approve_enrollments(
            &[Approval::new(opened.clone())],
            &codes_of(&[&req]),
            &identity,
            &unlocks(),
        );
    });

    assert!(
        died,
        "the injected failure never fired, so this asserts nothing"
    );
    assert_eq!(head(&a.vault), before);
    assert_eq!(pending_names(&a.vault), pending_before);
    assert!(at_head(&a.vault, &file));
    assert_eq!(key_listed_as(&a.vault, &opened.pubkey), None);
}

// The
// pre-check is UX; the check under the lock is the one that has to be
// right against a concurrent writer, and it is `recipient add`'s
// `RecipientExists`, not enrollment's `NameTaken`.
#[test]
fn the_authoritative_duplicate_check_still_fires_under_the_lock() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let opened = opened_for(&a.vault, &req.code);
    a.vault
        .check_enrollment_labels(&[Approval::new(opened.clone())])
        .expect("the name is free when the pre-check runs");

    list_by_hand(&a, "phone-1", &new_pubkey());

    let trusting = unlocks().with_recipient_answers(&[true]);
    assert_enrollment_error!(
        approve_as(&a, &[Approval::new(opened)], &codes_of(&[&req]), &trusting),
        EnrollmentError::Recipient(RecipientError::RecipientExists { pubkey: None, .. }),
        ErrorKind::Conflict
    );
}

// "Let this
// device in" is the CLI's question; "do you trust the list you are about
// to encrypt to" is the trust check's, asked when someone else changed the list.
// Declining the second leaves the request pending.
#[test]
fn declining_the_trust_prompt_aborts_with_nothing_committed() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let opened = opened_for(&a.vault, &req.code);
    list_by_hand(&a, "desktop-1", &new_pubkey());
    let before = head(&a.vault);

    let declines = unlocks().with_recipient_answers(&[false]);
    let result = approve_as(&a, &[Approval::new(opened)], &codes_of(&[&req]), &declines);

    assert!(result.is_err(), "declined, and still approved");
    assert_eq!(declines.recipient_changes().len(), 1);
    assert_eq!(head(&a.vault), before);
    assert_eq!(pending_names(&a.vault).len(), 1);
}

// A caller bug, not a success that
// did no work: the no-op has an outcome and a cleared request.
#[test]
fn approval_refuses_an_empty_batch() {
    let a = SyncVault::new();
    let err = assert_enrollment_error!(
        approve_as(&a, &[], &[], &unlocks()),
        EnrollmentError::EmptyBatch,
        ErrorKind::Usage
    );
    assert!(err.to_string().contains("approve"), "{err}");
}

// Both kinds gage
// will never honour: expired by filename, and beyond the ceiling.
#[test]
fn approval_prunes_expired_requests_in_the_same_commit() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let stale = lay_stale_request(&a.vault, hours(-1));
    let beyond = lay_stale_request(&a.vault, hours(2 * 7 * 24));
    let before = super::approve_tests::commits(&a.vault);

    approve_one(&a, &req).unwrap();

    for name in [&stale, &beyond] {
        assert!(
            !a.vault.pending_dir().join(name).exists(),
            "{name} survived"
        );
        assert!(!at_head(&a.vault, &format!("{PENDING_DIR}/{name}")));
    }
    assert_eq!(super::approve_tests::commits(&a.vault), before + 1);
}

// From any other device, an approval is a recipient change made elsewhere:
// exactly what the recipient-change warning is for.
#[test]
fn a_second_device_gets_the_ordinary_warning_after_someone_else_approves() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let second = Machine::cloned_from(&a, "desktop-1");
    let key = second.identity_add("desktop-1");
    authorize(&a, "desktop-1", &key);
    // Its cache primed by a write of its own.
    let primed = FakePrompter::new()
        .with_passphrase(B_PASSPHRASE)
        .with_recipient_answers(&[true]);
    let id = second.vault.unlock(&primed).unwrap();
    second
        .vault
        .insert(&id, sample("primed"), false, &primed)
        .unwrap();
    drop(id);

    let (_m, req) = enrolled(&a, "phone-1");
    approve_one(&a, &req).unwrap();

    let asks = FakePrompter::new()
        .with_passphrase(B_PASSPHRASE)
        .with_recipient_answers(&[true]);
    let id = second.vault.unlock(&asks).unwrap();
    second
        .vault
        .insert(&id, sample("after"), false, &asks)
        .unwrap();
    let changes = asks.recipient_changes();
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert!(
        changes[0].added.iter().any(|r| r.pubkey == req.pubkey),
        "the warning doesn't name the approved key: {:?}",
        changes[0]
    );
}

// The
// operator just reviewed this list by approving; warning them at their
// next write teaches them to stop reading the warning.
#[test]
fn the_approving_device_does_not_warn_itself_about_its_own_approval() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let opened = opened_for(&a.vault, &req.code);
    let p = unlocks().with_recipient_answers(&[]);
    let identity = a.vault.unlock(&p).unwrap();

    a.vault
        .approve_enrollments(&[Approval::new(opened)], &codes_of(&[&req]), &identity, &p)
        .unwrap();
    a.vault
        .insert(&identity, sample("straight after"), false, &p)
        .unwrap();

    assert!(
        p.recipient_changes().is_empty(),
        "{:?}",
        p.recipient_changes()
    );
}

// Pending/ is inert:
// nothing in it changes who can read the vault.
#[test]
fn a_pending_request_alone_triggers_no_warning() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    insert_as(&a, "primes the cache");

    enrolled(&a, "phone-1");

    let p = unlocks().with_recipient_answers(&[]);
    let identity = a.vault.unlock(&p).unwrap();
    a.vault
        .insert(&identity, sample("with a request pending"), false, &p)
        .unwrap();
    assert!(p.recipient_changes().is_empty());
    assert!(p.warnings().is_empty(), "{:?}", p.warnings());
}
