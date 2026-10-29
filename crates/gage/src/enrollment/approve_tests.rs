//! `approve_enrollments` and `check_enrollment_labels`. Device A is [`SyncVault`], the owner; each
//! joining device is a [`Machine`] cloned from the same origin that has
//! published a real request through `Vault::enroll`, the state `gage
//! recipient approve` starts from. The fixtures here are shared by the
//! other approval suites.

use std::path::Path;

use zeroize::Zeroizing;

use super::fixtures_tests::{
    DAY, assert_enrollment_error, file_hand_sealed, good_payload, hours, new_pubkey, open_all,
    vault,
};
use super::*;
use crate::error::ErrorKind;
use crate::identity::Identity;
use crate::prompter::FakePrompter;
use crate::recipient::RecipientError;
use crate::remotesync::tests::{FakeSyncer, SyncVault, sample, unlocks};
use crate::twodevice_tests::{B_PASSPHRASE, Machine};

/// A joining device that has published a request, and the request (with
/// its code). The owner's vault is caught up to it, as it would be after
/// the unlock's automatic pull.
pub(crate) fn enrolled(a: &SyncVault, device: &str) -> (Machine, EnrollmentRequest) {
    let m = Machine::cloned_from(a, device);
    let req = m
        .vault
        .enroll(
            device,
            DAY,
            &FakePrompter::new().with_passphrase(B_PASSPHRASE),
        )
        .unwrap_or_else(|e| panic!("publishing {device:?}'s request: {e}"));
    a.vault.pull().unwrap();
    (m, req)
}

/// The one request `code` opens, from `v`'s side.
pub(crate) fn opened_for(v: &Vault, code: &str) -> OpenedRequest {
    let mut opened = open_all(v, code).expect("opening the request with its code");
    assert_eq!(opened.len(), 1, "one code, one request");
    opened.remove(0)
}

pub(crate) fn codes_of(reqs: &[&EnrollmentRequest]) -> Vec<Zeroizing<String>> {
    reqs.iter().map(|r| r.code.clone()).collect()
}

/// Approves as the owner, with a fresh identity and `p` for every
/// question the write asks.
pub(crate) fn approve_as(
    a: &SyncVault,
    approvals: &[Approval],
    codes: &[Zeroizing<String>],
    p: &FakePrompter,
) -> Result<ApprovalResult, EnrollmentError> {
    let identity = a.unlock();
    a.vault.approve_enrollments(approvals, codes, &identity, p)
}

/// One approval of one request under its own code, as the owner.
pub(crate) fn approve_one(
    a: &SyncVault,
    req: &EnrollmentRequest,
) -> Result<ApprovalResult, EnrollmentError> {
    let opened = opened_for(&a.vault, &req.code);
    approve_as(a, &[Approval::new(opened)], &codes_of(&[req]), &unlocks())
}

pub(crate) fn insert_as(a: &SyncVault, title: &str) {
    let identity = a.unlock();
    a.vault
        .insert(&identity, sample(title), false, &unlocks())
        .unwrap();
}

/// Adds `pubkey` as `device` through the ordinary `recipient add`.
pub(crate) fn authorize(a: &SyncVault, device: &str, pubkey: &str) {
    let identity = a.unlock();
    a.vault
        .add_recipient(device, pubkey, &identity, &unlocks())
        .unwrap();
}

/// Whether `m` can read every entry, after catching up: the property
/// approval exists to establish, asserted against the ciphertext rather
/// than the recipient list, which is only a claim about it.
pub(crate) fn reads_every_entry(m: &Machine) -> bool {
    m.vault.pull().unwrap();
    let ids = m.vault.entry_ids().unwrap();
    assert!(!ids.is_empty(), "no entries, so this asserts nothing");
    let identity: Identity = m.unlock();
    ids.iter()
        .all(|id| m.vault.read_entry(*id, &identity).is_ok())
}

/// The names in `pending/` on disk, live or not.
pub(crate) fn pending_names(v: &Vault) -> Vec<String> {
    let mut names: Vec<String> = v
        .pending_files()
        .unwrap()
        .into_iter()
        .map(|(n, _, _)| n)
        .collect();
    names.sort();
    names
}

pub(crate) fn commits(v: &Vault) -> usize {
    gitrepo::commit_count(&v.path).unwrap()
}

pub(crate) fn head(v: &Vault) -> String {
    let repo = git2::Repository::open(&v.path).unwrap();
    repo.head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
        .to_string()
}

/// Whether HEAD's tree carries `rel`.
pub(crate) fn at_head(v: &Vault, rel: &str) -> bool {
    let repo = git2::Repository::open(&v.path).unwrap();
    let tree = repo.head().unwrap().peel_to_tree().unwrap();
    tree.get_path(Path::new(rel)).is_ok()
}

pub(crate) fn key_listed_as(v: &Vault, pubkey: &str) -> Option<String> {
    v.recipients()
        .unwrap()
        .into_iter()
        .find(|r| r.pubkey == pubkey)
        .map(|r| r.device)
}

/// Lists `pubkey` as `device` in both recipient files, commits and
/// publishes, without re-encrypting: the hand edit that is the one way to
/// a partially admitted device since ADR-0009.
fn hand_list(a: &SyncVault, device: &str, pubkey: &str) {
    let mut list = a.vault.recipients().unwrap();
    list.push(vaultconfig::Recipient {
        device: device.into(),
        pubkey: pubkey.into(),
    });
    a.vault.write_recipient_files(&list).unwrap();
    gitrepo::commit_all(&a.vault.path, "test: listed without re-encrypting").unwrap();
    a.vault.push().unwrap();
}

// --- check_enrollment_labels ----------------------------------------------

// The
// sealed name was free when it was written and labels a different key by
// the time anyone approves it. It holds no identity: it reads the
// plaintext list, which is what lets the CLI run it before the
// confirmation and before any unlock.
#[test]
fn check_enrollment_labels_refuses_a_name_another_recipient_took() {
    let a = SyncVault::new();
    let (_m, req) = enrolled(&a, "phone-1");
    authorize(&a, "phone-1", &new_pubkey());

    let opened = opened_for(&a.vault, &req.code);
    let err = assert_enrollment_error!(
        a.vault.check_enrollment_labels(&[Approval::new(opened)]),
        EnrollmentError::NameTaken { .. },
        ErrorKind::Conflict
    );
    assert_eq!(
        err.remedy(),
        Some(crate::error::Remedy::ApproveWithDevice),
        "{err}"
    );
}

// --device is the whole fix
// for the case above.
#[test]
fn check_enrollment_labels_accepts_a_relabel() {
    let a = SyncVault::new();
    let (_m, req) = enrolled(&a, "phone-1");
    authorize(&a, "phone-1", &new_pubkey());

    let opened = opened_for(&a.vault, &req.code);
    a.vault
        .check_enrollment_labels(&[Approval::relabeled(opened, "phone-2")])
        .unwrap();
}

// A request whose
// key is already listed is the no-op, not a clash: advising --device would
// mint a second label for a key that already has access.
#[test]
fn check_enrollment_labels_ignores_the_devices_own_entry() {
    let a = SyncVault::new();
    let (_m, req) = enrolled(&a, "phone-1");
    let opened = opened_for(&a.vault, &req.code);
    authorize(&a, "phone-1", &opened.pubkey);

    a.vault
        .check_enrollment_labels(&[Approval::new(opened)])
        .unwrap();
}

// Answerable from
// plaintext, so it can run before the confirmation: no identity exists
// here, and an offline syncer is never touched.
#[test]
fn check_enrollment_labels_needs_no_identity_or_network() {
    let a = SyncVault::new();
    let (_m, req) = enrolled(&a, "phone-1");
    let opened = opened_for(&a.vault, &req.code);

    let offline = FakeSyncer::offline();
    let v = a.vault.clone().with_remote_syncer(offline.clone());
    v.check_enrollment_labels(&[Approval::new(opened.clone())])
        .unwrap();

    assert_eq!(
        offline.fetches.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the pass reached the network"
    );
    assert_eq!(
        opened.expires.duration_since(opened.created),
        jiff::SignedDuration::try_from(DAY).unwrap()
    );
}

// --- approve_enrollments ---------------------------------------------------

#[test]
fn approval_adds_one_recipient_and_keeps_the_files_in_sync() {
    let a = SyncVault::new();
    insert_as(&a, "before the phone existed");
    let (_m, req) = enrolled(&a, "phone-1");

    let res = approve_one(&a, &req).unwrap();

    assert_eq!(res.outcomes.len(), 1);
    assert!(res.outcomes[0].added);
    assert_eq!(res.outcomes[0].label, "phone-1");
    assert_eq!(
        key_listed_as(&a.vault, &req.pubkey).as_deref(),
        Some("phone-1")
    );
    assert!(a.vault.verify_recipients().unwrap().in_sync);
    assert!(pending_names(&a.vault).is_empty(), "the request survived");
}

// The device reads
// entries written long before it existed: no path produces a partially
// readable recipient.
#[test]
fn approval_makes_every_pre_existing_entry_readable() {
    let a = SyncVault::new();
    insert_as(&a, "written long before the phone existed");
    insert_as(&a, "and a second one");
    let (m, req) = enrolled(&a, "phone-1");

    let res = approve_one(&a, &req).unwrap();

    assert_eq!(res.reencrypted, 2);
    assert!(reads_every_entry(&m));
}

// The recipient files, every
// re-encrypted entry, and the request's removal.
#[test]
fn approval_lands_in_exactly_one_commit() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let file = format!("{PENDING_DIR}/{}", pending_names(&a.vault)[0]);
    assert!(at_head(&a.vault, &file));
    let before = commits(&a.vault);

    approve_one(&a, &req).unwrap();

    assert_eq!(commits(&a.vault), before + 1);
    assert!(!at_head(&a.vault, &file), "the approved request is at HEAD");
    assert!(at_head(&a.vault, ".age-recipients"));
    assert!(gitrepo::is_clean(&a.vault.path).unwrap());
}

// N codes, one
// decrypt-and-rewrite of every entry.
#[test]
fn batch_approval_is_one_pass_and_one_commit() {
    let a = SyncVault::new();
    insert_as(&a, "first");
    insert_as(&a, "second");
    let (phone, first) = enrolled(&a, "phone-1");
    let (_tablet, second) = enrolled(&a, "tablet-1");

    let codes = codes_of(&[&first, &second]);
    let pending = a.vault.pending_enrollments().unwrap();
    let opened = a.vault.open_enrollment(&pending, &codes).unwrap();
    assert_eq!(opened.len(), 2);
    let before = commits(&a.vault);

    let approvals: Vec<Approval> = opened.into_iter().map(Approval::new).collect();
    let res = approve_as(&a, &approvals, &codes, &unlocks()).unwrap();

    assert_eq!(res.reencrypted, 2, "a second pass would report 4");
    assert_eq!(commits(&a.vault), before + 1);
    assert!(pending_names(&a.vault).is_empty());
    assert!(reads_every_entry(&phone));
}

// A device that
// is listed but can't read an entry (a hand-edited list, the one way to
// that damage since ADR-0009) is refused before the lock and before the
// trust question, with a count rather than a failure on an opaque entry id.
#[test]
fn approval_refuses_an_approver_who_cannot_read_every_entry() {
    let a = SyncVault::new();
    insert_as(&a, "readable by the owner alone");
    let (_m, req) = enrolled(&a, "phone-1");

    let partial = Machine::cloned_from(&a, "desktop-1");
    let partial_key = partial.identity_add("desktop-1");
    hand_list(&a, "desktop-1", &partial_key);
    // Its unlock writes its trust baseline; then the list changes again,
    // so the trust question is live and "never shown" means something.
    partial.vault.pull().unwrap();
    let identity = partial.unlock();
    hand_list(&a, "tablet-1", &new_pubkey());
    partial.vault.pull().unwrap();

    let opened = opened_for(&partial.vault, &req.code);
    let before = head(&partial.vault);
    let trust = FakePrompter::new().with_recipient_answers(&[true]);
    let err = assert_enrollment_error!(
        partial.vault.approve_enrollments(
            &[Approval::new(opened)],
            &codes_of(&[&req]),
            &identity,
            &trust
        ),
        EnrollmentError::Recipient(RecipientError::CannotGrantFullAccess { unreadable: 1, .. }),
        ErrorKind::Conflict
    );
    assert!(err.to_string().contains("1 of 1"), "{err}");
    assert!(
        trust.recipient_changes().is_empty(),
        "the trust check asked first"
    );
    assert_eq!(head(&partial.vault), before, "the refusal committed");
    assert_eq!(pending_names(&partial.vault).len(), 1);
}

// The code authenticated
// the key, not the label.
#[test]
fn approval_relabels_without_changing_the_key() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let opened = opened_for(&a.vault, &req.code);

    let res = approve_as(
        &a,
        &[Approval::relabeled(opened.clone(), "phone-2")],
        &codes_of(&[&req]),
        &unlocks(),
    )
    .unwrap();

    assert_eq!(res.outcomes[0].label, "phone-2");
    assert_eq!(
        key_listed_as(&a.vault, &opened.pubkey).as_deref(),
        Some("phone-2")
    );
}

// A success that does
// no work: cleared in one commit, nothing re-encrypted, nothing added.
#[test]
fn approving_a_key_that_is_already_a_recipient_is_a_no_op() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    authorize(&a, "phone-1", &req.pubkey);
    let before = commits(&a.vault);

    let res = approve_one(&a, &req).unwrap();

    assert_eq!(res.outcomes.len(), 1);
    assert!(!res.outcomes[0].added);
    assert_eq!(res.outcomes[0].label, "phone-1");
    assert_eq!(res.reencrypted, 0);
    assert!(pending_names(&a.vault).is_empty(), "not cleared");
    assert_eq!(a.vault.recipients().unwrap().len(), 2);
    assert_eq!(commits(&a.vault), before + 1, "one commit clearing it");
}

// Two requests from
// one device, approved in turn: the second is the no-op, not an error.
#[test]
fn approving_the_same_device_twice_leaves_one_recipient() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let m = Machine::cloned_from(&a, "phone-1");
    let p = FakePrompter::new().with_passphrases(&[B_PASSPHRASE, B_PASSPHRASE, B_PASSPHRASE]);
    let first = m.vault.enroll("phone-1", DAY, &p).unwrap();
    let second = m.vault.enroll("phone-1", DAY, &p).unwrap();
    a.vault.pull().unwrap();

    approve_one(&a, &first).unwrap();
    let res = approve_one(&a, &second).unwrap();

    assert!(!res.outcomes[0].added, "a second grant");
    assert_eq!(a.vault.recipients().unwrap().len(), 2);
}

// The property is
// structural: a malformed payload yields no `OpenedRequest`, so a frontend
// has nothing to render, and the refusal doesn't echo the escape back.
#[test]
fn a_malformed_request_produces_nothing_to_render() {
    let f = vault();
    let id = uuid::Uuid::new_v4().to_string();
    let mut payload = good_payload(&f.vault, &id, "phone", &new_pubkey(), hours(0), hours(24));
    payload.device = "phone\x1b[2K\x1b[1GApprove? [Y/n] ".into();
    let code = file_hand_sealed(&f.vault, &id, &payload);

    let err = assert_enrollment_error!(
        open_all(&f.vault, &code),
        EnrollmentError::MalformedRequest { .. },
        ErrorKind::Conflict
    );
    assert!(!err.to_string().contains('\x1b'), "{err}");
}

// The fix is on the other
// machine, so the message names its clock rather than a stale request.
#[test]
fn clock_skew_names_the_requesting_devices_clock() {
    let f = vault();
    let id = uuid::Uuid::new_v4().to_string();
    let created = hours(0);
    let payload = good_payload(&f.vault, &id, "phone", &new_pubkey(), created, hours(-24));
    let code = file_hand_sealed(&f.vault, &id, &payload);

    let err = assert_enrollment_error!(
        open_all(&f.vault, &code),
        EnrollmentError::ClockSkew { .. },
        ErrorKind::Conflict
    );
    assert!(err.to_string().contains("clock"), "{err}");
}
