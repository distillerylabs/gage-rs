//! The seal's work factor, and sealing and opening.

use std::time::{Duration, Instant};

use uuid::Uuid;

use super::fixtures_tests::*;
use super::name::pending_file_name;
use super::{EnrollmentError, MAX_ENROLLMENT_ATTEMPTS, PendingRequest};
use crate::crypto::crypt;
use crate::error::ErrorKind;
use crate::storage::vaultconfig::UnlockMethod;

/// The `-> scrypt <salt> <logN>` stanza's logN, off a real header.
fn stanza_factor(file: &[u8]) -> u8 {
    let header = String::from_utf8_lossy(file);
    header
        .lines()
        .find_map(
            |line| match line.split_whitespace().collect::<Vec<_>>()[..] {
                ["->", "scrypt", _, n] => n.parse().ok(),
                _ => None,
            },
        )
        .expect("no scrypt stanza in the file")
}

/// Rewrites a file's scrypt stanza to claim `log_n`, leaving the rest. The
/// header MAC no longer covers it, which doesn't matter: age checks the
/// claim while unwrapping the stanza, before the MAC, so this is the file
/// a hostile git writer would commit.
fn claim_work_factor(sealed: &[u8], log_n: u8) -> Vec<u8> {
    let text = String::from_utf8_lossy(sealed);
    let at = text.find("-> scrypt ").expect("no scrypt stanza");
    let end = at + text[at..].find('\n').unwrap();
    let salt = text[at..end].split_whitespace().nth(2).unwrap();
    let mut out = sealed[..at].to_vec();
    out.extend_from_slice(format!("-> scrypt {salt} {log_n}").as_bytes());
    out.extend_from_slice(&sealed[end..]);
    out
}

// --- the child-process rows ---------------------------------------------
//
// Lowering and restoring a process-wide factor around these would race every
// other scrypt test, since tests share one process and run in parallel; a child running one test alone may move factors freely.

pub(crate) const CHILD_ENV: &str = "GAGE_ENROLL_TEST_CHILD";

/// Runs `test` alone in a child test process with `CHILD_ENV` set, and
/// returns its stderr. Panics if the child failed.
#[allow(clippy::disallowed_methods)]
pub(crate) fn run_child(test: &str) -> String {
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "the child {test} failed:\n{stderr}");
    assert!(
        stderr.contains("CHILD RAN"),
        "the child {test} never ran its body (a renamed test?):\n{stderr}"
    );
    stderr
}

/// Whether this is the child half of a child-process row. The child says
/// so on stderr, so a parent can't pass by running nothing.
#[allow(clippy::disallowed_macros)]
pub(crate) fn in_child() -> bool {
    let child = std::env::var_os(CHILD_ENV).is_some();
    if child {
        eprintln!("CHILD RAN");
    }
    child
}

#[test]
fn shipped_enrollment_factor_child() {
    if !in_child() {
        return;
    }
    let f = vault();
    crypt::set_enrollment_work_factor_for_tests(crypt::SHIPPED_ENROLLMENT_SCRYPT_WORK_FACTOR);
    let (req, sealed) = f
        .vault
        .seal_enrollment("phone", &new_pubkey(), &UnlockMethod::Passphrase, DAY)
        .unwrap();
    assert_eq!(
        stanza_factor(&sealed),
        crypt::SHIPPED_ENROLLMENT_SCRYPT_WORK_FACTOR,
        "age sealed at a factor other than the shipped one"
    );
    // And gage opens what it seals at that factor, on a real file.
    write_pending(
        &f.vault,
        &pending_file_name(req.id.as_deref().unwrap(), req.expires),
        &sealed,
    );
    let opened = open_all(&f.vault, &req.code).unwrap();
    assert_eq!(opened.len(), 1);
}

// The constant
// being right isn't the constant being wired up: dropping the
// set_work_factor call leaves every seal at age's default with every other
// row green. Read off the blob's own stanza.
#[test]
fn shipped_enrollment_work_factor_reaches_a_real_age_file() {
    run_child("enrollment::seal_tests::shipped_enrollment_factor_child");
}

#[test]
fn independent_factors_child() {
    if !in_child() {
        return;
    }
    let f = vault();
    let seal_factor = || {
        let (_, sealed) = f
            .vault
            .seal_enrollment("phone", &new_pubkey(), &UnlockMethod::Passphrase, DAY)
            .unwrap();
        stanza_factor(&sealed)
    };
    let identity_factor = || {
        stanza_factor(&crypt::encrypt(b"x", &[crypt::Recipient::passphrase("hunter2")]).unwrap())
    };

    // Moving the identity factor leaves the seal alone.
    crypt::set_enrollment_work_factor_for_tests(11);
    let before = seal_factor();
    crypt::set_scrypt_work_factor_for_tests(13);
    assert_eq!(seal_factor(), before, "sealing reads the identity factor");
    assert_eq!(
        identity_factor(),
        13,
        "the identity hook didn't take; this proves nothing"
    );

    // Moving the seal factor leaves the identity file alone.
    let before = identity_factor();
    crypt::set_enrollment_work_factor_for_tests(12);
    assert_eq!(
        identity_factor(),
        before,
        "the two factors are one variable"
    );
    assert_eq!(
        seal_factor(),
        12,
        "the enrollment hook didn't take; this proves nothing"
    );
}

// Both directions:
// collapsing them would let a suite lowering one silently lower the other,
// hiding ADR-0040's calibration behind a test hook.
#[test]
fn enrollment_and_identity_work_factors_are_independent() {
    run_child("enrollment::seal_tests::independent_factors_child");
}

// One committed file claiming
// 2^30 is enough, and without the cap the symptom is a process that seems
// to have stopped, so the open runs on a thread and the test fails on the
// timeout rather than measuring afterwards.
#[test]
fn hostile_work_factor_costs_a_bounded_wait() {
    let f = vault();
    let req = seal_into(&f.vault, "phone", &new_pubkey(), DAY);
    let path = f
        .vault
        .pending_dir()
        .join(pending_file_name(req.id.as_deref().unwrap(), req.expires));
    let sealed = std::fs::read(&path).unwrap();
    std::fs::write(&path, claim_work_factor(&sealed, 30)).unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let vault = f.vault.clone();
    let code = req.code.to_string();
    std::thread::spawn(move || tx.send(open_all(&vault, &code)).unwrap());
    let result = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("opening a request claiming 2^30 did not return within 30s: the open path is not capping the work factor");
    // Refused, not honoured: the file did not open.
    assert_enrollment_error!(result, EnrollmentError::CodeWrong, ErrorKind::LockedOrAuth);
}

// ADR-0026: a claim above the enrollment cap that the identity
// file's cap would admit is refused before the KDF runs. The rewritten
// stanza fails the header MAC either way, so the answer is the same and
// only the cost tells the cap apart: at 2^20 the KDF alone is about a
// second (and 1 GiB), the refusal is a header parse.
#[test]
fn claimed_factor_is_capped_at_the_enrollment_cap() {
    const CLAIM: u8 = 20;
    const {
        assert!(
            crypt::ENROLLMENT_SCRYPT_MAX_WORK_FACTOR < CLAIM
                && CLAIM <= crypt::SCRYPT_MAX_WORK_FACTOR
        )
    };
    let f = vault();
    let req = seal_into(&f.vault, "phone", &new_pubkey(), DAY);
    let path = f
        .vault
        .pending_dir()
        .join(pending_file_name(req.id.as_deref().unwrap(), req.expires));
    let sealed = std::fs::read(&path).unwrap();
    std::fs::write(&path, claim_work_factor(&sealed, CLAIM)).unwrap();
    let start = Instant::now();
    assert_enrollment_error!(
        open_all(&f.vault, &req.code),
        EnrollmentError::CodeWrong,
        ErrorKind::LockedOrAuth
    );
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "a claim of 2^{CLAIM} took {elapsed:?}: the open path ran the KDF, so it is not capped at \
         ENROLLMENT_SCRYPT_MAX_WORK_FACTOR"
    );
}

// --- seal and open -----------------------------------------------

// The baseline: a container. The
// rows below are what the feature rests on.
#[test]
fn seal_round_trips_through_its_own_code() {
    let f = vault();
    let joining = new_pubkey();
    let req = seal_into(&f.vault, "phone", &joining, DAY);
    let opened = open_all(&f.vault, &req.code).unwrap();
    assert_eq!(opened.len(), 1);
    let got = &opened[0];
    assert_eq!(Some(&got.id), req.id.as_ref());
    assert_eq!(got.device, "phone");
    assert_eq!(got.pubkey, joining);
    assert_eq!(got.method, "passphrase");
    assert_eq!((got.created, got.expires), (req.created, req.expires));
}

// The property the seal is for: the
// plaintext is a public key, so what a decryption proves is that the
// sealer knew a code agreed out of band.
#[test]
fn wrong_code_opens_nothing() {
    let f = vault();
    let req = seal_into(&f.vault, "phone", &new_pubkey(), DAY);
    let wrong = new_code();
    assert_ne!(wrong, *req.code, "the same code twice; this proves nothing");
    assert_enrollment_error!(
        open_all(&f.vault, &wrong),
        EnrollmentError::CodeWrong,
        ErrorKind::LockedOrAuth
    );
}

// End to end: what matters to a
// human retyping a code off paper is that the request opens. The code has
// a 1 and a 0 so the substitutions have something to substitute for.
#[test]
fn retyped_codes_open_the_same_request() {
    let f = vault();
    const CODE: &str = "GAGE-10AB-CDEF-GHJK-MNPQ";
    let id = Uuid::new_v4().to_string();
    let p = good_payload(&f.vault, &id, "phone", &new_pubkey(), hours(0), hours(24));
    write_pending(
        &f.vault,
        &pending_file_name(&id, hours(24)),
        &hand_seal(&p, CODE),
    );
    for (name, typed) in [
        ("as displayed", CODE),
        ("lowercase", "gage-10ab-cdef-ghjk-mnpq"),
        ("spaces for hyphens", "GAGE 10AB CDEF GHJK MNPQ"),
        ("no prefix", "10AB-CDEF-GHJK-MNPQ"),
        ("I typed for 1", "GAGE-I0AB-CDEF-GHJK-MNPQ"),
        ("L typed for 1", "GAGE-L0AB-CDEF-GHJK-MNPQ"),
        ("O typed for 0", "GAGE-1OAB-CDEF-GHJK-MNPQ"),
        ("lowercase l and o", "gage-lOab-cdef-ghjk-mnpq"),
    ] {
        let opened = open_all(&f.vault, typed).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(opened.len(), 1, "{name}");
        assert_eq!(opened[0].id, id, "{name}");
    }
}

// The answer is the
// same either way, deliberately (the retry loop keys on it), so cost is
// the only thing to assert; the well-formed wrong code paying is what
// makes the zero meaningful.
#[test]
fn malformed_code_is_rejected_before_any_decryption() {
    let f = vault();
    let joining = new_pubkey();
    for _ in 0..5 {
        seal_into(&f.vault, "phone", &joining, DAY);
    }
    for typed in ["", "GAGE-", "nope", "7K4M9QX2P3RH8WVU", "7K4M-9QX2-P3RH"] {
        assert_enrollment_error!(
            open_all(&f.vault, typed),
            EnrollmentError::CodeWrong,
            ErrorKind::LockedOrAuth
        );
    }
    assert_eq!(
        f.vault.enrollment_decrypt_count(),
        0,
        "a malformed code paid"
    );
    assert!(open_all(&f.vault, &new_code()).is_err());
    assert_eq!(f.vault.enrollment_decrypt_count(), 5);
}

// The AEAD property: without it the seal
// would prove a code-holder once wrote a blob, not that these bytes are
// theirs, and the pubkey is the field an attacker would change. The flip
// is in the payload, so the failure is authentication, not parsing.
#[test]
fn tampered_seal_does_not_open() {
    let f = vault();
    let req = seal_into(&f.vault, "phone", &new_pubkey(), DAY);
    let path = f
        .vault
        .pending_dir()
        .join(pending_file_name(req.id.as_deref().unwrap(), req.expires));
    let mut sealed = std::fs::read(&path).unwrap();
    *sealed.last_mut().unwrap() ^= 0x01;
    std::fs::write(&path, sealed).unwrap();
    let result = open_all(&f.vault, &req.code);
    assert!(result.is_err(), "a tampered blob opened: {result:?}");
}

// The filename is unauthenticated,
// the sealed id is not, so a blob can't be renamed onto another slot. The
// epoch is deliberately not compared, and that is asserted too.
#[test]
fn sealed_id_must_match_the_filename() {
    let joining = new_pubkey();
    let code = new_code();
    let sealed_id = Uuid::new_v4().to_string();
    let expires = hours(24);

    let f = vault();
    let p = good_payload(&f.vault, &sealed_id, "phone", &joining, hours(0), expires);
    let sealed = hand_seal(&p, &code);
    let other = Uuid::new_v4().to_string();
    write_pending(&f.vault, &pending_file_name(&other, expires), &sealed);
    assert_enrollment_error!(
        open_all(&f.vault, &code),
        EnrollmentError::IdMismatch { .. },
        ErrorKind::Conflict
    );

    // The same request under an epoch two hours from the sealed one: still
    // live, so only a comparison that shouldn't happen could refuse it.
    let f = vault();
    let p = good_payload(&f.vault, &sealed_id, "phone", &joining, hours(0), expires);
    write_pending(
        &f.vault,
        &pending_file_name(&sealed_id, hours(22)),
        &hand_seal(&p, &code),
    );
    let opened = open_all(&f.vault, &code).unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].expires, expires, "expiry comes from the seal");
}

/// `n` well-named, live, unopenable files.
fn stuff(f: &crate::trust::recipient::recipient_tests::Fixture, n: usize) {
    for _ in 0..n {
        let id = Uuid::new_v4().to_string();
        write_pending(&f.vault, &pending_file_name(&id, hours(1)), b"a request");
    }
}

// A directory any git
// writer can fill multiplies every run's work; the refusal comes before
// the work and names the way through.
#[test]
fn too_many_pending_is_refused_before_any_decryption() {
    let f = vault();
    stuff(&f, MAX_ENROLLMENT_ATTEMPTS + 1);
    let err = assert_enrollment_error!(
        open_all(&f.vault, &new_code()),
        EnrollmentError::TooManyPending { .. },
        ErrorKind::Conflict
    );
    assert_eq!(f.vault.enrollment_decrypt_count(), 0);
    assert!(err.to_string().contains("by id"), "{err}");
}

// Ordinary neglect never looks
// like an attack, with no write having pruned first; and opening deletes
// nothing.
#[test]
fn the_bound_counts_live_requests_only() {
    let f = vault();
    for i in 0..40 {
        let id = Uuid::new_v4().to_string();
        write_pending(
            &f.vault,
            &pending_file_name(&id, hours(-(i + 1))),
            b"expired",
        );
    }
    let req = seal_into(&f.vault, "phone", &new_pubkey(), DAY);
    stuff(&f, 3);

    let pending = f.vault.pending_enrollments().unwrap();
    assert!(
        pending.len() <= MAX_ENROLLMENT_ATTEMPTS,
        "{}",
        pending.len()
    );
    let opened = f
        .vault
        .open_enrollment(&pending, &codes(&req.code))
        .unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(Some(&opened[0].id), req.id.as_ref());
    let left = std::fs::read_dir(f.vault.pending_dir()).unwrap().count();
    assert_eq!(left, 44, "a read-only run deleted files");
}

// No flag: the broad and
// the scoped run are one path over different inputs, so neither can opt
// out of the bound.
#[test]
fn id_scoped_run_passes_the_bound_by_construction() {
    let f = vault();
    let req = seal_into(&f.vault, "phone", &new_pubkey(), DAY);
    stuff(&f, MAX_ENROLLMENT_ATTEMPTS * 2);
    assert!(matches!(
        open_all(&f.vault, &req.code),
        Err(EnrollmentError::TooManyPending { .. })
    ));

    let scoped = f
        .vault
        .resolve_enrollment(req.id.as_deref().unwrap())
        .unwrap();
    let opened = f
        .vault
        .open_enrollment(&[scoped], &codes(&req.code))
        .unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(Some(&opened[0].id), req.id.as_ref());
    assert_eq!(
        f.vault.enrollment_decrypt_count(),
        1,
        "one attempt per code"
    );
}

// An open that read the
// directory and treated its argument as advice would pass every row above
// and reintroduce the unbounded path.
#[test]
fn scope_is_honoured_not_treated_as_a_hint() {
    let f = vault();
    let joining = new_pubkey();
    let wanted = seal_into(&f.vault, "phone", &joining, DAY);
    let other = seal_into(&f.vault, "tablet", &joining, DAY);
    let scoped: PendingRequest = f
        .vault
        .resolve_enrollment(other.id.as_deref().unwrap())
        .unwrap();
    assert_enrollment_error!(
        f.vault.open_enrollment(&[scoped], &codes(&wanted.code)),
        EnrollmentError::CodeWrong,
        ErrorKind::LockedOrAuth
    );
}
