//! Approval fetches before it commits, against a real bare origin.
//!
//! Every test that moves the remote takes the approver's identity first:
//! the unlock runs the automatic pull, and an identity taken afterwards
//! would have fast-forwarded on approval's behalf, so the test would pass
//! with approval's own fetch removed.

use std::sync::Arc;
use std::time::Instant;

use super::approve_tests::{
    approve_one, codes_of, enrolled, head, insert_as, key_listed_as, opened_for, pending_names,
};
use super::fixtures_tests::{assert_enrollment_error, good_payload, hand_seal, write_pending};
use super::name::pending_file_name;
use super::*;
use crate::entry::WRITE_LOCK_TIMEOUT;
use crate::exitcode::Code;
use crate::gittest;
use crate::identity::Identity;
use crate::prompter::FakePrompter;
use crate::remotesync::UNPUSHED_WRITE_CLAUSE;
use crate::remotesync::tests::{FakeSyncer, SyncVault, unlocks};
use crate::syncerr;
use crate::twodevice_tests::Machine;

/// Approves `req` as the owner through `v` (a, or a with an injected
/// syncer), with an identity taken before anything here moved.
fn approve_with(
    v: &Vault,
    identity: &Identity,
    req: &EnrollmentRequest,
    p: &FakePrompter,
) -> Result<ApprovalResult, EnrollmentError> {
    let opened = opened_for(v, &req.code);
    v.approve_enrollments(&[Approval::new(opened)], &codes_of(&[req]), identity, p)
}

fn remote_has(a: &SyncVault, rel: &str) -> bool {
    gittest::Device::clone(a.remote.path()).exists(rel)
}

fn request_path(req: &EnrollmentRequest) -> String {
    format!("{PENDING_DIR}/{}", pending_file_name(&req.id, req.expires))
}

/// A local commit here and an unrelated one on the remote.
fn diverge(a: &SyncVault) {
    a.other()
        .write_commit_push("README", "theirs", "the remote moved");
    std::fs::write(a.vault.path.join("LOCAL"), "mine").unwrap();
    gitrepo::commit_all(&a.vault.path, "a local commit").unwrap();
}

/// Whether `m`'s key opens every entry in `v`'s working tree: the local
/// work, checked where it happened rather than after a push that didn't.
fn opens_every_entry_of(v: &Vault, m: &Machine) -> bool {
    let identity = m.unlock();
    let ids = v.entry_ids().unwrap();
    assert!(!ids.is_empty());
    ids.iter().all(|id| v.read_entry(*id, &identity).is_ok())
}

fn push_refused_for_access() -> Arc<FakeSyncer> {
    Arc::new(FakeSyncer {
        push_err: Some(syncerr::Error::new(syncerr::Kind::Auth, "403")),
        ..FakeSyncer::default()
    })
}

// Against a remote that moved
// ahead, the approval lands on top of its tip and its push is a
// fast-forward, not a divergence in every entry.
#[test]
fn approval_fast_forwards_before_it_writes() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let identity = a.unlock();
    a.other()
        .write_commit_push("README", "moved on", "another device wrote this");

    approve_with(&a.vault, &identity, &req, &unlocks()).unwrap();

    let readme = std::fs::read_to_string(a.vault.path.join("README")).unwrap();
    assert!(readme.contains("moved on"), "committed onto a stale tip");
    assert_eq!(gitrepo::ahead_count(&a.vault.path).unwrap(), 0);
    assert!(remote_has(&a, ".age-recipients"));
    assert!(!remote_has(&a, &request_path(&req)), "the push didn't land");
}

// The advice
// is `gage sync`, correct here and the one deliberate difference from
// enroll's: an approver can decrypt.
#[test]
fn approval_refuses_a_diverged_remote_before_writing_anything() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let identity = a.unlock();
    diverge(&a);
    let before = head(&a.vault);
    let list_before = a.vault.recipients().unwrap();

    let err = assert_enrollment_error!(
        approve_with(&a.vault, &identity, &req, &unlocks()),
        EnrollmentError::ApprovalDiverged { .. },
        Code::Conflict
    );

    assert!(err.to_string().contains("gage sync"), "{err}");
    assert_eq!(head(&a.vault), before);
    assert_eq!(a.vault.recipients().unwrap(), list_before);
    assert_eq!(pending_names(&a.vault).len(), 1);
}

// The trap's
// premise first (the pull succeeds and only its report says diverged),
// then that approval refuses rather than walking past it.
#[test]
fn approval_finds_divergence_in_the_report_rather_than_an_error() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let identity = a.unlock();
    diverge(&a);

    let report = a
        .vault
        .pull()
        .expect("a pull over a divergence succeeds; that is the trap");
    assert!(report.diverged, "the fixture is not diverged");
    let before = head(&a.vault);

    assert!(approve_with(&a.vault, &identity, &req, &unlocks()).is_err());
    assert_eq!(head(&a.vault), before, "committed onto a diverged branch");
}

// The asymmetry
// with enroll: an approval that lands locally is real work. The local
// work is checked where it happened.
#[test]
fn approval_over_an_unreachable_remote_warns_and_proceeds() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (m, req) = enrolled(&a, "phone-1");
    let identity = a.unlock();
    let offline = a.vault.clone().with_remote_syncer(FakeSyncer::offline());
    let p = unlocks();

    let res = approve_with(&offline, &identity, &req, &p).unwrap();

    assert_eq!(res.reencrypted, 1);
    assert!(key_listed_as(&a.vault, &req.pubkey).is_some());
    assert!(pending_names(&a.vault).is_empty());
    let warnings = p.warnings();
    assert!(
        warnings.iter().any(|w| w.contains("could not reach origin")
            && w.contains("approving against the local copy")),
        "{warnings:?}"
    );
    assert!(opens_every_entry_of(&a.vault, &m));
}

// A pull taken from
// inside approval's own lock waits out the full timeout and then fails as
// contended; the plain success path finishes well inside it.
#[test]
fn uncontended_approval_never_reports_contention() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let identity = a.unlock();

    let start = Instant::now();
    approve_with(&a.vault, &identity, &req, &unlocks())
        .expect("an uncontended approval failed: a pull from inside its own lock looks like this");
    assert!(
        start.elapsed() < WRITE_LOCK_TIMEOUT,
        "took {:?}, as long as a lock wait",
        start.elapsed()
    );
}

// Opened, then
// denied elsewhere before this run's fetch: reported as no longer
// pending, naming the likely cause, with nothing granted.
#[test]
fn approval_reports_a_request_resolved_by_another_device() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let identity = a.unlock();
    let opened = opened_for(&a.vault, &req.code);

    let other = a.other();
    other.remove(&request_path(&req));
    other.commit("another device denied it");
    other.push();

    let err = assert_enrollment_error!(
        a.vault.approve_enrollments(
            &[Approval::new(opened)],
            &codes_of(&[&req]),
            &identity,
            &unlocks()
        ),
        EnrollmentError::NoLongerPending { .. },
        Code::NotFound
    );
    assert!(err.to_string().contains("another device"), "{err}");
    assert_eq!(key_listed_as(&a.vault, &req.pubkey), None);
    assert_eq!(
        head(&a.vault),
        gittest::Device::clone(a.remote.path()).head()
    );
}

// Approval's clause, not the
// generic one: locally the device is listed, every entry re-encrypted and
// the request gone; on the remote none of it happened.
#[test]
fn approval_warns_when_its_push_fails() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let identity = a.unlock();
    let refusing = a
        .vault
        .clone()
        .with_remote_syncer(push_refused_for_access());
    let p = unlocks();

    let res = approve_with(&refusing, &identity, &req, &p).unwrap();

    assert!(!res.commit.is_empty(), "the local commit didn't stand");
    let warnings = p.warnings().join("\n");
    assert!(
        !warnings.contains(UNPUSHED_WRITE_CLAUSE),
        "the generic clause:\n{warnings}"
    );
    for want in ["locked out", "gage push"] {
        assert!(warnings.contains(want), "never says {want:?}:\n{warnings}");
    }
    assert!(pending_names(&a.vault).is_empty());
    assert!(remote_has(&a, &request_path(&req)), "the push didn't fail");
}

// Once the push lands, the
// request another device still saw is the already-a-recipient no-op:
// what makes the failed-push state self-healing.
#[test]
fn reapproval_after_a_failed_push_is_the_no_op() {
    let a = SyncVault::new();
    insert_as(&a, "an entry");
    let (_m, req) = enrolled(&a, "phone-1");
    let identity = a.unlock();
    let refusing = a
        .vault
        .clone()
        .with_remote_syncer(push_refused_for_access());
    approve_with(&refusing, &identity, &req, &unlocks()).unwrap();
    a.vault.push().unwrap();

    // The request as another device still sees it.
    let payload = good_payload(
        &a.vault,
        &req.id,
        &req.device,
        &req.pubkey,
        req.created,
        req.expires,
    );
    write_pending(
        &a.vault,
        &pending_file_name(&req.id, req.expires),
        &hand_seal(&payload, &req.code),
    );
    gitrepo::commit_all(&a.vault.path, "test: the request, still pending elsewhere").unwrap();

    let res = approve_one(&a, &req).unwrap();
    assert!(!res.outcomes[0].added, "a second grant");
    assert_eq!(res.reencrypted, 0);
}
