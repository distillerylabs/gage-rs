//! E2 task 7: the load-bearing invariant, `pending/` is inert (Go's
//! `enrollinert_test.go`, and the wall-clock row from
//! `enrollseal_test.go`). Nothing in `pending/` is consulted at encryption
//! time or counts as a recipient; only a human approving with a code turns
//! a proposal into a grant. These rows must never be weakened.

use std::time::{Duration, Instant};

use uuid::Uuid;

use super::fixtures_tests::*;
use super::name::pending_file_name;
use super::seal_tests::{in_child, run_child};
use super::{MAX_ENROLLMENT_ATTEMPTS, PendingRequest};
use crate::crypt;
use crate::entry::NewEntry;
use crate::prompter::FakePrompter;

// Go: TestPendingRequestGrantsNoDecryption. The invariant as ciphertext,
// with the entry written *after* the request is pending: the ordering
// that fails if any encrypt path consulted the directory.
#[test]
fn pending_request_grants_no_decryption() {
    let f = vault();
    let outsider = age::x25519::Identity::generate();
    let outsider_pub = outsider.to_public().to_string();
    let req = seal_into(&f.vault, "phone", &outsider_pub, DAY);

    // It really is a valid request, so this is about inertness rather
    // than a broken fixture.
    let opened = open_all(&f.vault, &req.code).unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].pubkey, outsider_pub);

    let id = f
        .vault
        .insert(
            &f.identity(),
            NewEntry {
                title: "written while a request is pending".into(),
                value: "secret".into(),
                ..NewEntry::default()
            },
            true,
            &FakePrompter::new(),
        )
        .unwrap();
    let as_outsider = f
        .vault
        .read_entry(id, &crate::identity::Identity::for_tests(&outsider));
    assert!(
        matches!(
            as_outsider,
            Err(crate::entry::Error::Crypt(crypt::Error::NotARecipient))
        ),
        "the pending key could read an entry: {as_outsider:?}"
    );
    f.vault.read_entry(id, &f.identity()).unwrap();
}

// Go: TestPendingRequestsAreNotRecipients. From the control plane: the
// list is unchanged, and verify is in sync with ten pending.
#[test]
fn pending_requests_are_not_recipients() {
    let f = vault();
    let before = f.vault.recipients().unwrap();
    let joining = new_pubkey();
    for _ in 0..10 {
        seal_into(&f.vault, "phone", &joining, DAY);
    }
    let after = f.vault.recipients().unwrap();
    assert_eq!(after, before, "a request is a proposal, not a grant");
    assert!(after.iter().all(|r| r.pubkey != joining));
    let verification = f.vault.verify_recipients().unwrap();
    assert!(verification.in_sync, "{verification:?}");
}

// Go: TestPendingDirectoryNeedsNoIdentity. The signature E4's late unlock
// rests on, structurally: no identity is in scope here, and every call
// still works.
#[test]
fn pending_directory_needs_no_identity() {
    let f = vault();
    let req = seal_into(&f.vault, "phone", &new_pubkey(), DAY);
    let pending = f.vault.pending_enrollments().unwrap();
    assert_eq!(pending.len(), 1);
    let resolved: PendingRequest = f.vault.resolve_enrollment(&req.id).unwrap();
    assert_eq!(resolved.id, req.id);
    let opened = f
        .vault
        .open_enrollment(&pending, &codes(&req.code))
        .unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].id, req.id);
}

// Go: TestGarbageInPendingIsNeverFatal. Degrades to "skipped", never to an
// error that takes a read or an approval down.
#[test]
fn garbage_in_pending_is_never_fatal() {
    let f = vault();
    let req = seal_into(&f.vault, "phone", &f.pubkey(), DAY);
    let live = || pending_file_name(&Uuid::new_v4().to_string(), hours(1));
    write_pending(&f.vault, &live(), b"");
    write_pending(&f.vault, &live(), b"age-encryption.org/v1\ntruncated");
    write_pending(&f.vault, &live(), &[0x00, 0xff, 0x00, 0xff]);
    write_pending(&f.vault, "a-directory-listing", b"nothing to do with gage");

    f.vault.pending_enrollments().unwrap();
    let opened = open_all(&f.vault, &req.code).unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].id, req.id);
}

// Beyond Go (E2's review): a committed symlink is not a request, at
// listing or when handed in directly, so one pointing at a FIFO can't hang
// the open and one pointing at a real sealed file can't stand in for it.
#[cfg(unix)]
#[test]
fn symlinks_in_pending_are_never_followed() {
    let f = vault();
    let req = seal_into(&f.vault, "phone", &f.pubkey(), DAY);
    let real = f
        .vault
        .pending_dir()
        .join(pending_file_name(&req.id, req.expires));
    let elsewhere = f.vault.path.with_file_name("sealed-elsewhere.age");
    std::fs::rename(&real, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &real).unwrap();
    let fifo = f
        .vault
        .pending_dir()
        .join(pending_file_name(&Uuid::new_v4().to_string(), hours(1)));
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    assert!(made.is_ok_and(|s| s.success()), "mkfifo failed");

    assert!(f.vault.pending_enrollments().unwrap().is_empty());
    let link = PendingRequest {
        id: req.id.clone(),
        path: real,
        expires: req.expires,
    };
    assert!(matches!(
        f.vault.open_enrollment(&[link], &codes(&req.code)),
        Err(super::EnrollmentError::CodeWrong)
    ));
    assert_eq!(f.vault.enrollment_decrypt_count(), 0);

    // On a thread, so a regression fails here rather than hanging the run.
    let fifo = PendingRequest {
        id: Uuid::new_v4().to_string(),
        path: fifo,
        expires: hours(1),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let vault = f.vault.clone();
    let code = req.code.to_string();
    std::thread::spawn(move || {
        tx.send(vault.open_enrollment(&[fifo], &codes(&code)))
            .unwrap()
    });
    let result = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("opening a FIFO in pending/ blocked: the open path follows non-regular files");
    assert!(matches!(result, Err(super::EnrollmentError::CodeWrong)));
}

/// The generous ceiling of the one wall-clock row: it exists to catch a
/// ~32× regression (the seal "hardened" back to 19), not to measure.
const FULL_DIRECTORY_CEILING: Duration = Duration::from_secs(30);

#[test]
fn full_directory_child() {
    if !in_child() {
        return;
    }
    let f = vault();
    crypt::set_enrollment_work_factor_for_tests(crypt::SHIPPED_ENROLLMENT_SCRYPT_WORK_FACTOR);
    let joining = new_pubkey();
    for _ in 0..MAX_ENROLLMENT_ATTEMPTS {
        seal_into(&f.vault, "phone", &joining, DAY);
    }
    let pending = f.vault.pending_enrollments().unwrap();
    assert_eq!(pending.len(), MAX_ENROLLMENT_ATTEMPTS);

    let start = Instant::now();
    assert!(
        f.vault
            .open_enrollment(&pending, &codes(&new_code()))
            .is_err(),
        "a random code opened a request"
    );
    let elapsed = start.elapsed();
    assert_eq!(f.vault.enrollment_decrypt_count(), MAX_ENROLLMENT_ATTEMPTS);
    assert!(
        elapsed < FULL_DIRECTORY_CEILING,
        "a wrong code against {MAX_ENROLLMENT_ATTEMPTS} requests took {elapsed:?}: the seal \
         factor has been raised back toward the identity file's"
    );
}

// Go: TestWrongCodeAgainstAFullDirectoryIsFast. The case factor 14 was
// chosen for: a typo has to try every request before it can say "wrong",
// and a frontend retries on exactly that. At the shipped factor, in a
// child, since the rest of the binary runs at the test factor.
#[test]
fn wrong_code_against_a_full_directory_is_fast() {
    run_child("enrollment::inert_tests::full_directory_child");
}
