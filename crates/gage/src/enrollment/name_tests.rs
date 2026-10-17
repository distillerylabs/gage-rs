//! E2 task 5, the filename and TTL half (Go's `enrollname_test.go`).

use std::time::Duration;

use uuid::Uuid;

use super::fixtures_tests::*;
use super::name::{parse_pending_file_name, pending_file_name};
use super::{DEFAULT_ENROLLMENT_TTL, EnrollmentError, MAX_ENROLLMENT_TTL};
use crate::exitcode::Code;
use crate::vault::METHOD_PASSPHRASE;

// Go: TestPendingFileNameRoundTrips. <request-id>-<expires-epoch>.age,
// to the second, which is the resolution the name carries.
#[test]
fn pending_file_name_round_trips() {
    let id = Uuid::new_v4().to_string();
    let expires = hours(3);
    let name = pending_file_name(&id, expires);
    assert!(name.ends_with(".age"), "{name}");
    assert!(name.starts_with(&format!("{id}-")), "{name}");
    assert_eq!(parse_pending_file_name(&name), Some((id, expires)));
}

// Go: TestPendingFileNameParserSkipsStrays. A name gage didn't write is
// skipped, and each half is validated on its own.
#[test]
fn pending_file_name_parser_skips_strays() {
    let id = Uuid::new_v4().to_string();
    let upper = id.to_uppercase();
    let undashed = id.replace('-', "");
    for (name, file) in [
        ("no extension", format!("{id}-1788862320")),
        ("wrong extension", format!("{id}-1788862320.txt")),
        ("no epoch", format!("{id}.age")),
        ("empty epoch", format!("{id}-.age")),
        ("non-numeric epoch", format!("{id}-tomorrow.age")),
        ("negative epoch", format!("{id}--1788862320.age")),
        ("signed epoch", format!("{id}-+1788862320.age")),
        ("epoch with padding", format!("{id}- 1788862320.age")),
        (
            "an epoch past i64",
            format!("{id}-99999999999999999999.age"),
        ),
        ("not a uuid", "not-a-uuid-1788862320.age".into()),
        ("uppercase uuid", format!("{upper}-1788862320.age")),
        ("uuid without hyphens", format!("{undashed}-1788862320.age")),
        (
            "nil uuid",
            "00000000-0000-0000-0000-000000000000-1788862320.age".into(),
        ),
        ("empty", String::new()),
        ("extension only", ".age".into()),
        ("a directory marker", "README".into()),
    ] {
        assert_eq!(parse_pending_file_name(&file), None, "{name}: {file:?}");
    }
}

// Go: TestDefaultTTLIsWithinTheCeiling. A default gage couldn't seal would
// be found by every user and no test.
#[test]
fn default_ttl_is_within_the_ceiling() {
    assert!(!DEFAULT_ENROLLMENT_TTL.is_zero() && DEFAULT_ENROLLMENT_TTL <= MAX_ENROLLMENT_TTL);
    assert_eq!(DEFAULT_ENROLLMENT_TTL, Duration::from_secs(24 * 3600));
    assert_eq!(MAX_ENROLLMENT_TTL, Duration::from_secs(7 * 24 * 3600));
}

// Go: TestSealRejectsTTLsOutsideTheCeiling. At Usage, a rejection of what
// was asked for, and before anything is generated. Go's negative case
// can't be built as a std Duration; E3 refuses a negative --ttl when it
// parses one.
#[test]
fn seal_rejects_ttls_outside_the_ceiling() {
    let f = vault();
    let joining = new_pubkey();
    for ttl in [
        Duration::ZERO,
        MAX_ENROLLMENT_TTL + Duration::from_secs(1),
        Duration::from_secs(365 * 24 * 3600),
    ] {
        assert_enrollment_error!(
            f.vault
                .seal_enrollment("phone", &joining, METHOD_PASSPHRASE, ttl),
            EnrollmentError::InvalidTtl { .. },
            Code::Usage
        );
    }
    // The boundary itself is accepted: a ceiling, not one below it.
    f.vault
        .seal_enrollment("phone", &joining, METHOD_PASSPHRASE, MAX_ENROLLMENT_TTL)
        .unwrap();
    assert!(!f.vault.pending_dir().exists(), "sealing wrote something");
}
