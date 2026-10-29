//! The expiry half. Expiry is
//! enforced from the sealed copy; the filename's epoch is a housekeeping
//! hint for display and pruning, so the two can disagree and both
//! directions are safe.

use jiff::Timestamp;
use uuid::Uuid;

use super::EnrollmentError;
use super::fixtures_tests::*;
use super::name::pending_file_name;
use crate::error::ErrorKind;
use crate::recipient_tests::Fixture;

/// Files a good payload for `vault` with the given sealed times, under a
/// filename epoch of `filed`, and returns `(id, code)`.
fn file_with(
    f: &Fixture,
    created: Timestamp,
    expires: Timestamp,
    filed: Timestamp,
) -> (String, String) {
    let id = Uuid::new_v4().to_string();
    let p = good_payload(&f.vault, &id, "phone", &new_pubkey(), created, expires);
    let code = new_code();
    write_pending(
        &f.vault,
        &pending_file_name(&id, filed),
        &hand_seal(&p, &code),
    );
    (id, code)
}

// A far-future filename on
// an expired seal must not add a second: the listing shows it, and only
// the seal can refuse it.
#[test]
fn expiry_comes_from_the_seal_not_the_filename() {
    let f = vault();
    let (_, code) = file_with(&f, hours(-48), hours(-24), hours(6 * 24));
    assert_eq!(f.vault.pending_enrollments().unwrap().len(), 1);
    assert_enrollment_error!(
        open_all(&f.vault, &code),
        EnrollmentError::Expired { .. },
        ErrorKind::Conflict
    );
}

// The converse is
// safe, not merely tolerable: the request is invisible to a broad run, and
// pruning removes it (a denial of service anyone who can rename the file
// already has).
#[test]
fn filename_claiming_expiry_on_a_live_seal_grants_nothing() {
    let f = vault();
    let filed = hours(-1);
    let (id, code) = file_with(&f, hours(0), hours(24), filed);
    assert_enrollment_error!(
        open_all(&f.vault, &code),
        EnrollmentError::CodeWrong,
        ErrorKind::LockedOrAuth
    );
    assert_eq!(
        f.vault.prune_pending_enrollments().unwrap(),
        [pending_file_name(&id, filed)]
    );
}

// Its epoch never arrives,
// so without the clamp nothing would ever remove it. Includes an epoch
// past what a timestamp can hold.
#[test]
fn forged_filename_epoch_is_pruned_as_expired() {
    let f = vault();
    let live = seal_into(&f.vault, "phone", &f.pubkey(), DAY);
    let forged = format!("{}-99999999999.age", Uuid::new_v4());
    let unrepresentable = format!("{}-9999999999999999.age", Uuid::new_v4());
    write_pending(&f.vault, &forged, b"parked forever");
    write_pending(&f.vault, &unrepresentable, b"parked past the calendar");

    let pending = f.vault.pending_enrollments().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(Some(&pending[0].id), live.id.as_ref());

    let mut want = vec![forged, unrepresentable];
    want.sort();
    assert_eq!(f.vault.prune_pending_enrollments().unwrap(), want);
    assert!(
        f.vault
            .pending_dir()
            .join(pending_file_name(live.id.as_deref().unwrap(), live.expires))
            .exists(),
        "pruning removed the live request"
    );
}

// The clamp on the copy
// approval enforces, under an in-range filename. Expired, not malformed:
// nothing is ill-formed, the claim is outside what gage honours.
#[test]
fn sealed_expiry_beyond_the_ceiling_is_expired() {
    let f = vault();
    let (_, code) = file_with(&f, hours(0), hours(10 * 365 * 24), hours(1));
    assert_enrollment_error!(
        open_all(&f.vault, &code),
        EnrollmentError::Expired { .. },
        ErrorKind::Conflict
    );
}

// Both timestamps are the
// sealer's, so hanging the check on one of them checks nothing.
#[test]
fn expiry_is_compared_against_now_not_created() {
    const YEAR: i64 = 365 * 24;
    for (name, created, expires) in [
        ("a forged past", hours(-10 * YEAR), hours(-9 * YEAR)),
        ("a forged future", hours(-10 * YEAR), hours(10 * YEAR)),
    ] {
        let f = vault();
        let (_, code) = file_with(&f, created, expires, hours(1));
        let result = open_all(&f.vault, &code);
        assert!(
            matches!(result, Err(EnrollmentError::Expired { .. })),
            "{name}: {result:?}"
        );
    }
}

// Fix that machine's clock, versus ask for
// a fresh request.
#[test]
fn clock_skew_is_its_own_error() {
    let f = vault();
    let (_, code) = file_with(&f, hours(0), hours(-24), hours(1));
    assert_enrollment_error!(
        open_all(&f.vault, &code),
        EnrollmentError::ClockSkew { .. },
        ErrorKind::Conflict
    );

    let f = vault();
    let (_, code) = file_with(&f, hours(-48), hours(-24), hours(1));
    assert_enrollment_error!(
        open_all(&f.vault, &code),
        EnrollmentError::Expired { .. },
        ErrorKind::Conflict
    );
}
