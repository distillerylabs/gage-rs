//! Listing, resolution and pruning.

use std::time::Duration;

use uuid::Uuid;

use super::fixtures_tests::*;
use super::name::{parse_pending_file_name, pending_file_name};
use super::{EnrollmentError, MAX_ENROLLMENT_TTL, MAX_PENDING_REQUEST_BYTES};
use crate::exitcode::Code;
use crate::prompter::FakePrompter;

// The normal
// state: git tracks no empty directories.
#[test]
fn pending_enrollments_reports_zero_without_the_directory() {
    let f = vault();
    assert!(
        !f.vault.pending_dir().exists(),
        "a fresh vault already has pending/; nothing should create it speculatively"
    );
    assert!(f.vault.pending_enrollments().unwrap().is_empty());
}

// A directory anyone who can write
// the repository can fill: a file gage didn't write is skipped, never
// fatal.
#[test]
fn pending_enrollments_skips_strays() {
    let f = vault();
    let req = seal_into(&f.vault, "phone", &f.pubkey(), DAY);
    let live = hours(1);
    let well_named = || pending_file_name(&Uuid::new_v4().to_string(), live);
    write_pending(&f.vault, "README", b"not a request");
    write_pending(&f.vault, "notes.txt", b"also not a request");
    write_pending(&f.vault, &format!("{}.age", Uuid::new_v4()), b"no epoch");
    write_pending(
        &f.vault,
        &format!("not-a-uuid-{}.age", live.as_second()),
        b"no uuid",
    );
    write_pending(
        &f.vault,
        &format!("{}-tomorrow.age", Uuid::new_v4()),
        b"no epoch",
    );
    write_pending(
        &f.vault,
        &format!("{}.bak", well_named()),
        b"an editor's leftover",
    );
    // Well-named garbage: skipped by the opener, not the listing.
    write_pending(&f.vault, &well_named(), b"garbage, not ciphertext");
    std::fs::create_dir(f.vault.pending_dir().join(well_named())).unwrap();

    let pending = f.vault.pending_enrollments().unwrap();
    assert_eq!(pending.len(), 2, "{pending:?}");
    assert!(pending.iter().any(|p| p.id == req.id));
    open_all(&f.vault, &req.code).unwrap();
}

// A listing is a
// read: it filters and takes no lock, so it deletes nothing.
#[test]
fn pending_enrollments_filters_expired_without_deleting() {
    let f = vault();
    let live = seal_into(&f.vault, "phone", &f.pubkey(), DAY);
    let expired = pending_file_name(&Uuid::new_v4().to_string(), hours(-1));
    let beyond = pending_file_name(
        &Uuid::new_v4().to_string(),
        hours(MAX_ENROLLMENT_TTL.as_secs() as i64 / 3600 + 1),
    );
    write_pending(&f.vault, &expired, b"expired by its own name");
    write_pending(&f.vault, &beyond, b"an expiry gage would never write");

    let pending = f.vault.pending_enrollments().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, live.id);
    for name in [expired, beyond] {
        assert!(
            f.vault.pending_dir().join(&name).exists(),
            "listing deleted {name}"
        );
    }
}

// Proven by taking the
// repository away: a listing that consulted history in any form would
// fail without .git.
#[test]
fn pending_enrollments_reads_no_git_history() {
    let f = vault();
    let req = seal_into(&f.vault, "phone", &f.pubkey(), DAY);
    let git = f.vault.path.join(".git");
    let aside = f.vault.path.with_file_name("git-moved-aside");
    std::fs::rename(&git, &aside).unwrap();

    let pending = f.vault.pending_enrollments();
    std::fs::rename(&aside, &git).unwrap();
    let pending = pending.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, req.id);
    assert_eq!(
        pending[0].expires, req.expires,
        "the filename is the only source"
    );
}

// The two copies come from one
// value; drift would show one time and enforce another.
#[test]
fn filename_epoch_matches_the_sealed_expiry() {
    let f = vault();
    let req = seal_into(
        &f.vault,
        "phone",
        &f.pubkey(),
        Duration::from_secs(3 * 3600),
    );
    let (_, from_name) = parse_pending_file_name(&pending_file_name(&req.id, req.expires)).unwrap();
    let opened = open_all(&f.vault, &req.code).unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].expires, from_name);
}

// The query rule: exact
// beats substring, several matches come back as every candidate rather
// than a guess, and no match is an error.
#[test]
fn resolve_enrollment_follows_the_base_resolution_order() {
    let f = vault();
    let exact = "aaaaaaaa-0000-4000-8000-000000000001";
    let sibling = "aaaaaaaa-0000-4000-8000-000000000002";
    let unrelated = "bbbbbbbb-0000-4000-8000-000000000003";
    for id in [exact, sibling, unrelated] {
        write_pending(&f.vault, &pending_file_name(id, hours(1)), b"a request");
    }

    assert_eq!(f.vault.resolve_enrollment(exact).unwrap().id, exact);
    // Other spellings of the same UUID resolve exactly too.
    for spelling in [exact.to_uppercase(), format!("{{{exact}}}")] {
        assert_eq!(
            f.vault.resolve_enrollment(&spelling).unwrap().id,
            exact,
            "{spelling}"
        );
    }
    assert_eq!(f.vault.resolve_enrollment("bbbb").unwrap().id, unrelated);

    let err = assert_enrollment_error!(
        f.vault.resolve_enrollment("aaaaaaaa"),
        EnrollmentError::Ambiguous(_),
        Code::Ambiguous
    );
    let EnrollmentError::Ambiguous(candidates) = err else {
        unreachable!()
    };
    let mut ids: Vec<&str> = candidates.matches.iter().map(|m| m.id.as_str()).collect();
    ids.sort();
    assert_eq!(
        ids,
        [exact, sibling],
        "a resolver that narrowed would delete a request nobody named"
    );

    assert_enrollment_error!(
        f.vault.resolve_enrollment("cccc"),
        EnrollmentError::NoSuchRequest { .. },
        Code::NotFound
    );
    assert_enrollment_error!(
        f.vault.resolve_enrollment(""),
        EnrollmentError::NoSuchRequest { .. },
        Code::NotFound
    );

    let dead = "cccccccc-0000-4000-8000-000000000004";
    write_pending(&f.vault, &pending_file_name(dead, hours(-1)), b"expired");
    assert_enrollment_error!(
        f.vault.resolve_enrollment(dead),
        EnrollmentError::NoSuchRequest { .. },
        Code::NotFound
    );
}

// Ten digits of epoch
// are legal hex, so a short id could match one request's expiry and
// another's id; deny deletes.
#[test]
fn resolve_enrollment_matches_the_uuid_portion_only() {
    let f = vault();
    let expires = hours(2);
    let needle = &expires.as_second().to_string()[..4];

    let wanted = format!("{needle}{}", &Uuid::new_v4().to_string()[4..]);
    write_pending(
        &f.vault,
        &pending_file_name(&wanted, hours(1)),
        b"the id match",
    );
    let other = loop {
        let id = Uuid::new_v4().to_string();
        if !id.replace('-', "").contains(needle) {
            break id;
        }
    };
    write_pending(
        &f.vault,
        &pending_file_name(&other, expires),
        b"the epoch-only near miss",
    );

    assert_eq!(f.vault.resolve_enrollment(needle).unwrap().id, wanted);
}

// And what it must not
// touch: a live request, an unparseable name, an oversized live file.
#[test]
fn prune_removes_expired_and_forged_expiries_only() {
    let f = vault();
    let live = seal_into(&f.vault, "phone", &f.pubkey(), DAY);
    let expired = pending_file_name(&Uuid::new_v4().to_string(), hours(-1));
    let forged = pending_file_name(
        &Uuid::new_v4().to_string(),
        hours(MAX_ENROLLMENT_TTL.as_secs() as i64 / 3600 + 1),
    );
    let oversized = pending_file_name(&Uuid::new_v4().to_string(), hours(1));
    write_pending(&f.vault, &expired, b"expired");
    write_pending(&f.vault, &forged, b"an epoch gage would never write");
    write_pending(&f.vault, "README", b"not something gage wrote");
    write_pending(
        &f.vault,
        &oversized,
        &vec![0u8; MAX_PENDING_REQUEST_BYTES + 1],
    );

    let mut want = vec![expired, forged];
    want.sort();
    let removed = f.vault.prune_pending_enrollments().unwrap();
    assert_eq!(removed, want);
    for name in &removed {
        assert!(
            !f.vault.pending_dir().join(name).exists(),
            "{name} is still there"
        );
    }
    for name in [
        pending_file_name(&live.id, live.expires),
        "README".into(),
        oversized,
    ] {
        assert!(
            f.vault.pending_dir().join(&name).exists(),
            "pruning removed {name}"
        );
    }
}

// Pruning rides every
// recipient write, and most vaults have no pending/.
#[test]
fn prune_without_a_pending_directory_is_not_an_error() {
    let f = vault();
    assert!(f.vault.prune_pending_enrollments().unwrap().is_empty());
}

// Asserted against both the write and the prune primitive directly.
#[test]
fn pruning_leaves_live_requests_alone() {
    let f = vault();
    let live = seal_into(&f.vault, "phone-1", &new_pubkey(), DAY);
    crate::gitrepo::commit_all(&f.vault.path, "test: a live enrollment request").unwrap();

    f.vault
        .add_recipient(
            "desktop-1",
            &new_pubkey(),
            &f.identity(),
            &FakePrompter::new(),
        )
        .unwrap();
    assert!(f.vault.prune_pending_enrollments().unwrap().is_empty());

    let pending = f.vault.pending_enrollments().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, live.id);
}
