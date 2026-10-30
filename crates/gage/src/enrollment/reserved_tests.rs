//! The approval half of the reservation: a request can't claim
//! `recovery-paper-key`, and an approver can't relabel one to it. The rest
//! of the reservation is in `recovery/reservation_tests.rs`.

use super::approve_tests::{approve_as, codes_of, commits, enrolled, opened_for};
use super::fixtures_tests::{file_hand_sealed, good_payload, hours, new_pubkey};
use super::*;
use crate::error::{ErrorKind, HasKind};
use crate::prompter::FakePrompter;
use crate::recovery::RECOVERY_DEVICE_LABEL;
use crate::sync::remotesync::tests::SyncVault;

#[test]
fn approval_refuses_a_request_that_claims_the_recovery_label() {
    let a = SyncVault::new();
    // A hand-sealed payload: `enroll` itself would refuse to write this.
    let id = uuid::Uuid::new_v4().to_string();
    let payload = good_payload(
        &a.vault,
        &id,
        RECOVERY_DEVICE_LABEL,
        &new_pubkey(),
        hours(0),
        hours(24),
    );
    let code = file_hand_sealed(&a.vault, &id, &payload);
    gitrepo::commit_all(&a.vault.path, "seed a pending request").unwrap();
    let opened = opened_for(&a.vault, &code);
    let before = commits(&a.vault);

    let approvals = [Approval::new(opened.clone())];
    let pre = a.vault.check_enrollment_labels(&approvals).unwrap_err();
    assert!(
        matches!(pre, EnrollmentError::ReservedDeviceLabel(_)),
        "{pre}"
    );
    assert_eq!(pre.kind(), ErrorKind::Usage);

    let codes = vec![zeroize::Zeroizing::new(code)];
    let err = approve_as(&a, &approvals, &codes, &FakePrompter::new()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Usage, "{err}");
    assert_eq!(commits(&a.vault), before);
    assert_eq!(a.vault.recipients().unwrap().len(), 1);
}

#[test]
fn approval_refuses_a_relabel_to_the_recovery_label() {
    let a = SyncVault::new();
    let (_m, req) = enrolled(&a, "phone");
    let opened = opened_for(&a.vault, &req.code);
    let before = commits(&a.vault);

    let approvals = [Approval::relabeled(opened, RECOVERY_DEVICE_LABEL)];
    let pre = a.vault.check_enrollment_labels(&approvals).unwrap_err();
    assert!(
        matches!(pre, EnrollmentError::ReservedDeviceLabel(_)),
        "{pre}"
    );

    let err = approve_as(&a, &approvals, &codes_of(&[&req]), &FakePrompter::new()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Usage, "{err}");
    assert_eq!(commits(&a.vault), before);
    assert_eq!(a.vault.recipients().unwrap().len(), 1);
    // Still pending: nothing was consumed.
    assert_eq!(a.vault.pending_enrollments().unwrap().len(), 1);
}
