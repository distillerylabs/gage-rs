//! E2 task 4: the sealed payload is untrusted input (Go's
//! `enrollpayload_test.go`, and the coverage rows at the end of
//! `enrollseal_test.go`). Authenticated is not well-formed: the seal proves
//! the bytes are unaltered, not that the fields mean anything, and a git
//! writer or any code-holder can seal a bad one. So each payload here is
//! hand-sealed with one field spoiled, and must be refused as malformed,
//! never as a wrong code: the code worked.

use std::path::PathBuf;

use uuid::Uuid;

use super::fixtures_tests::*;
use super::name::pending_file_name;
use super::seal::{Attempt, Payload};
use super::{EnrollmentError, MAX_PENDING_REQUEST_BYTES, PendingRequest};
use crate::exitcode::Code;
use crate::vault::METHOD_PASSPHRASE;

// Go: TestMalformedPayloadsAreRefusedAsMalformed. Each is MalformedRequest
// at Conflict, and never CodeWrong, which a retry loop would sit
// re-prompting on against a file no code can make valid.
#[test]
fn malformed_payloads_are_refused_as_malformed() {
    type Spoil = fn(&mut Payload);
    let cases: [(&str, Spoil); 13] = [
        // It would be a path component, and a name in config.toml, before
        // anyone reviewed it.
        ("a device outside the allowlist", |p| {
            p.device = "../../../../etc/cron.d/x".into()
        }),
        // A name that moves the cursor can rewrite the question.
        ("a device carrying ANSI escapes", |p| {
            p.device = "phone\x1b[2K\x1b[1GApprove? [Y/n] ".into()
        }),
        // Printed directly above the prompt granting vault-wide access.
        ("a device carrying a newline", |p| {
            p.device = "phone\nthis is a second line".into()
        }),
        ("an empty device", |p| p.device = String::new()),
        ("a pubkey that is not an age recipient", |p| {
            p.pubkey = "not-a-key".into()
        }),
        ("a pubkey that is an age private key", |p| {
            p.pubkey =
                "AGE-SECRET-KEY-1QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQ"
                    .into()
        }),
        ("a method outside the allowlist", |p| {
            p.method = "yubikey".into()
        }),
        ("an empty method", |p| p.method = String::new()),
        ("a request id that is not a uuid", |p| {
            p.request_id = "request-1".into()
        }),
        ("a created that is not RFC 3339", |p| {
            p.created = "yesterday".into()
        }),
        ("an expires that is not RFC 3339", |p| {
            p.expires = "soon".into()
        }),
        ("an absent vault id", |p| p.vault_id = String::new()),
        ("a vault id that is not a uuid", |p| {
            p.vault_id = "personal".into()
        }),
    ];
    for (name, spoil) in cases {
        let f = vault();
        let id = Uuid::new_v4().to_string();
        let mut p = good_payload(&f.vault, &id, "phone", &new_pubkey(), hours(0), hours(24));
        spoil(&mut p);
        let code = file_hand_sealed(&f.vault, &id, &p);
        let result = open_all(&f.vault, &code);
        assert!(
            matches!(result, Err(EnrollmentError::MalformedRequest { .. })),
            "{name}: {result:?}"
        );
        assert_eq!(
            crate::exitcode::HasCode::code(&result.unwrap_err()),
            Code::Conflict,
            "{name}"
        );
    }
}

// The same refusal for a field that is missing from the TOML altogether,
// not just empty: an absent field is not a default.
#[test]
fn absent_payload_fields_are_malformed_not_unparseable() {
    let f = vault();
    let id = Uuid::new_v4().to_string();
    let p = good_payload(&f.vault, &id, "phone", &new_pubkey(), hours(0), hours(24));
    let request = PendingRequest {
        id: id.clone(),
        path: PathBuf::from("unused"),
        expires: hours(24),
    };
    let full = toml::to_string(&p).unwrap();
    let without_vault = full
        .lines()
        .filter(|l| !l.starts_with("vault_id"))
        .collect::<Vec<_>>()
        .join("\n");
    let err = f
        .vault
        .validate_payload(&request, without_vault.as_bytes(), hours(0))
        .unwrap_err();
    assert!(
        matches!(&err, EnrollmentError::MalformedRequest { reason } if reason.contains("vault id")),
        "{err}"
    );
}

// Go: TestMalformedPayloadDoesNotPoisonTheRun. One bad file anyone can
// commit must not take approval down with it.
#[test]
fn malformed_payload_does_not_poison_the_run() {
    let f = vault();
    let joining = new_pubkey();
    let bad = Uuid::new_v4().to_string();
    let mut p = good_payload(&f.vault, &bad, "phone", &joining, hours(0), hours(24));
    p.device = "\x1b[2Kmalicious".into();
    let bad_code = file_hand_sealed(&f.vault, &bad, &p);
    let good = seal_into(&f.vault, "tablet", &joining, DAY);

    let opened = open_all(&f.vault, &good.code).unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].id, good.id);
    // And the bad one is still refused on its own terms: skipped, not
    // quietly accepted.
    assert!(matches!(
        open_all(&f.vault, &bad_code),
        Err(EnrollmentError::MalformedRequest { .. })
    ));
}

// Beyond Go's row, which can't see this: there the bad file never opens
// with the good code, so nothing is refused. With two codes in one run,
// one opening a malformed request and one a good request, the refusal must
// not displace what opened (found by E2's mutation review).
#[test]
fn a_refusal_does_not_displace_a_request_that_opened() {
    let f = vault();
    let joining = new_pubkey();
    let bad = Uuid::new_v4().to_string();
    let mut p = good_payload(&f.vault, &bad, "phone", &joining, hours(0), hours(24));
    p.device = "\x1b[2Kmalicious".into();
    let bad_code = file_hand_sealed(&f.vault, &bad, &p);
    let good = seal_into(&f.vault, "tablet", &joining, DAY);

    let pending = f.vault.pending_enrollments().unwrap();
    let both = [
        zeroize::Zeroizing::new(bad_code),
        zeroize::Zeroizing::new(good.code.to_string()),
    ];
    let opened = f.vault.open_enrollment(&pending, &both).unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].id, good.id);
}

// Go: TestValidateSealedPayloadRejectsNonTOML. Authenticated bytes that
// aren't TOML at all.
#[test]
fn validate_sealed_payload_rejects_non_toml() {
    let f = vault();
    let request = PendingRequest {
        id: Uuid::new_v4().to_string(),
        path: PathBuf::from("unused"),
        expires: hours(1),
    };
    for plaintext in [&b"not { valid toml"[..], &[0xff, 0xfe, 0x00][..]] {
        let err = assert_enrollment_error!(
            f.vault.validate_payload(&request, plaintext, hours(0)),
            EnrollmentError::MalformedRequest { .. },
            Code::Conflict
        );
        assert!(err.to_string().contains("not valid TOML"), "{err}");
    }
}

// Go: TestSealEnrollmentRejectsInvalidInput. Sealing's own checks, each at
// its exit code.
#[test]
fn seal_enrollment_rejects_invalid_input() {
    let f = vault();
    let joining = new_pubkey();
    let seal = |device: &str, pubkey: &str, method: &str| {
        f.vault.seal_enrollment(device, pubkey, method, DAY)
    };

    let err = assert_enrollment_error!(
        seal("not a valid name!", &joining, METHOD_PASSPHRASE),
        EnrollmentError::InvalidDevice { .. },
        Code::Usage
    );
    assert!(err.to_string().contains("device name"), "{err}");
    assert_enrollment_error!(
        seal("phone", "not-an-age-key", METHOD_PASSPHRASE),
        EnrollmentError::InvalidPubkey(_),
        Code::Usage
    );
    let err = assert_enrollment_error!(
        seal("phone", &joining, "yubikey"),
        EnrollmentError::UnknownMethod { .. },
        Code::Usage
    );
    assert!(err.to_string().contains(METHOD_PASSPHRASE), "{err}");

    let mut broken = f.vault.clone();
    broken.id = "not-a-uuid".into();
    assert_enrollment_error!(
        broken.seal_enrollment("phone", &joining, METHOD_PASSPHRASE, DAY),
        EnrollmentError::NoVaultId,
        Code::Internal
    );
}

// Go: TestRequestSealedForAnotherVaultIsRefused. Built the way it actually
// happens, as a file copy; refused as neither of its neighbours, and named
// by the vault it is for, which is the whole content of the mistake.
#[test]
fn request_sealed_for_another_vault_is_refused() {
    let work = vault();
    let personal = vault();
    assert_ne!(work.vault.id, personal.vault.id, "this proves nothing");
    // The approver's machine has both registered: the situation the
    // refusal is for, and what lets it name the vault.
    register_also(&personal.vault, "work", &work.vault.id);

    let req = seal_into(&work.vault, "phone", &work.pubkey(), DAY);
    let name = pending_file_name(&req.id, req.expires);
    let sealed = std::fs::read(work.vault.pending_dir().join(&name)).unwrap();
    write_pending(&personal.vault, &name, &sealed);

    let err = assert_enrollment_error!(
        open_all(&personal.vault, &req.code),
        EnrollmentError::WrongVault { .. },
        Code::Conflict
    );
    assert!(err.to_string().contains("\"work\""), "{err}");

    // And it still opens where it belongs.
    let opened = open_all(&work.vault, &req.code).unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].id, req.id);

    // With nothing registered, the message falls back to the id.
    let stranger = vault();
    write_pending(&stranger.vault, &name, &sealed);
    let err = open_all(&stranger.vault, &req.code).unwrap_err();
    assert!(err.to_string().contains(&work.vault.id), "{err}");
}

// Go: TestOversizedPendingFileIsNeverOpened. "Never opened", not "opened
// and rejected", so it counts attempts; and skipped, never deleted.
#[test]
fn oversized_pending_file_is_never_opened() {
    let f = vault();
    let req = seal_into(&f.vault, "phone", &new_pubkey(), DAY);
    let huge = pending_file_name(&Uuid::new_v4().to_string(), hours(1));
    write_pending(&f.vault, &huge, &vec![0u8; 20 << 20]);

    let pending = f.vault.pending_enrollments().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, req.id);
    let opened = f
        .vault
        .open_enrollment(&pending, &codes(&req.code))
        .unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(
        f.vault.enrollment_decrypt_count(),
        1,
        "the oversized file was opened"
    );
    assert!(f.vault.pending_dir().join(&huge).exists(), "it was deleted");
}

// Go: TestOversizedFileIsSkippedEvenWhenHandedInDirectly. The open path is
// handed a scope, so the bound lives there too. age ignores trailing
// bytes, so without the check this padded real request would open.
#[test]
fn oversized_file_is_skipped_even_when_handed_in_directly() {
    let f = vault();
    let (req, mut sealed) = f
        .vault
        .seal_enrollment("phone", &new_pubkey(), METHOD_PASSPHRASE, DAY)
        .unwrap();
    sealed.extend(std::iter::repeat_n(0u8, MAX_PENDING_REQUEST_BYTES));
    let path = write_pending(&f.vault, &pending_file_name(&req.id, req.expires), &sealed);
    let direct = PendingRequest {
        id: req.id.clone(),
        path,
        expires: req.expires,
    };
    assert_enrollment_error!(
        f.vault.open_enrollment(&[direct], &codes(&req.code)),
        EnrollmentError::CodeWrong,
        Code::LockedOrAuth
    );
    assert_eq!(f.vault.enrollment_decrypt_count(), 0);
}

// Go: TestDecryptedPayloadIsBoundedToo. Against the function that owns the
// bound, since today the file-size check would catch this first: the
// bound stays real rather than incidental to age's framing.
#[test]
fn decrypted_payload_is_bounded_too() {
    let f = vault();
    let code = new_code();
    let id = Uuid::new_v4().to_string();
    let joining = new_pubkey();
    let mut p = good_payload(&f.vault, &id, "phone", &joining, hours(0), hours(24));
    p.pubkey = joining + &"a".repeat(4 << 20);
    let sealed = hand_seal(&p, &code);
    assert!(sealed.len() > MAX_PENDING_REQUEST_BYTES);

    let normalized = super::code::normalize_code(&code).unwrap();
    let request = PendingRequest {
        id,
        path: PathBuf::from("unused"),
        expires: hours(24),
    };
    match f
        .vault
        .open_sealed(&request, &sealed, &normalized, hours(0))
    {
        Attempt::Refused(err @ EnrollmentError::MalformedRequest { .. }) => {
            assert!(err.to_string().contains("larger than"), "{err}")
        }
        other => panic!("want a refusal on the limit, got {other:?}"),
    }
}

// Go: TestGeneratedCodeIsReturnedAndNowhereElse. Scoped to the generating
// side, as Go's is: the code is in the return value and in no file under
// the vault or this device's config, data and state roots
// (Q-ENROLLMENT-CODE-HISTORY covers the approving side).
#[test]
fn generated_code_is_returned_and_nowhere_else() {
    let f = vault();
    let req = seal_into(&f.vault, "phone", &new_pubkey(), DAY);
    assert!(!req.code.is_empty());
    let normalized = super::code::normalize_code(&req.code).expect("a code that validates");

    let mut stack = vec![f._root.path().to_path_buf()];
    let mut files = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            files += 1;
            let content = std::fs::read(&path).unwrap();
            for needle in [req.code.as_bytes(), normalized.as_bytes()] {
                assert!(
                    !content.windows(needle.len()).any(|w| w == needle),
                    "the code appears in {}",
                    path.display()
                );
            }
        }
    }
    assert!(
        files > 5,
        "the walk saw only {files} files; it isn't looking at the vault"
    );
}
