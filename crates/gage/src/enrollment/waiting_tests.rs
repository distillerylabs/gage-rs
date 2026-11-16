//! `Vault::check_approved`: the joining device asking whether it has been
//! approved, against a real bare origin, with no passphrase (#231). Device
//! A is the owner; device B a [`Machine`] that enrolled and recorded its key
//! the way a frontend does after a published request.

use super::approve_tests::{approve_one, authorize, enrolled};
use super::*;
use crate::error::{ErrorKind, HasKind, Remedy};
use crate::prompter::FakePrompter;
use crate::sync::remotesync::tests::{FakeSyncer, SyncVault};
use crate::trust::trustcache_tests::new_key;
use crate::vault::twodevice_tests::{B_DEVICE, Machine};

/// Records `req`'s key as B's own in its registration, as the CLI and the
/// bindings do once a request is published.
fn record_key(m: &Machine, req: &EnrollmentRequest) {
    let path = paths::global_config_file(&m.dirs);
    let mut global = config::load(&path).unwrap();
    let entry = global.vaults.get_mut(&m.vault.name).unwrap();
    entry.device = req.device.clone();
    entry.pubkey = Some(req.pubkey.clone());
    config::save_creating_dir(&path, &global).unwrap();
}

/// A device that has asked to join and is waiting.
fn waiting() -> (SyncVault, Machine, EnrollmentRequest) {
    let a = SyncVault::new();
    let (b, req) = enrolled(&a, B_DEVICE);
    record_key(&b, &req);
    (a, b, req)
}

#[test]
fn a_device_waiting_for_approval_is_not_approved() {
    let (_a, b, _req) = waiting();
    let p = FakePrompter::new();

    assert!(!b.vault.check_approved(&p).unwrap());
    assert!(p.unlock_requests().is_empty(), "no passphrase is asked for");
}

#[test]
fn an_approved_device_is_approved_once_it_has_fetched_the_approval() {
    let (a, b, req) = waiting();
    approve_one(&a, &req).unwrap();
    // B hasn't pulled: the answer has to come from the remote, not from
    // what the clone held before the call.
    assert!(
        !b.vault
            .recipients()
            .unwrap()
            .iter()
            .any(|r| r.pubkey == req.pubkey)
    );
    let p = FakePrompter::new();

    assert!(b.vault.check_approved(&p).unwrap());
    assert!(p.unlock_requests().is_empty(), "no passphrase is asked for");
}

#[test]
fn approval_is_by_key_not_by_device_name() {
    let (a, b, _req) = waiting();
    // Someone else's key listed under B's name: B is still waiting.
    authorize(&a, B_DEVICE, &new_key());

    assert!(!b.vault.check_approved(&FakePrompter::new()).unwrap());
}

#[test]
fn checking_while_waiting_saves_no_trust_record_and_the_approval_does_not_warn() {
    let (a, b, req) = waiting();
    assert!(!b.vault.check_approved(&FakePrompter::new()).unwrap());
    assert!(
        b.vault.load_trust_cache().unwrap().is_none(),
        "a pending device adopted a list"
    );

    approve_one(&a, &req).unwrap();
    let p = FakePrompter::new();
    assert!(b.vault.check_approved(&p).unwrap());
    assert!(p.warnings().is_empty(), "{:?}", p.warnings());
    let p = FakePrompter::new();
    assert!(b.vault.check_approved(&p).unwrap());
    assert!(p.warnings().is_empty(), "{:?}", p.warnings());
}

#[test]
fn a_device_that_never_asked_to_join_is_told_to_enroll() {
    let a = SyncVault::new();
    let b = Machine::cloned_from(&a, B_DEVICE);

    let err = b.vault.check_approved(&FakePrompter::new()).unwrap_err();

    assert!(
        matches!(err, ApprovalCheckError::NotEnrolled { .. }),
        "{err}"
    );
    assert_eq!(err.kind(), ErrorKind::Usage);
    assert_eq!(err.remedy(), Some(Remedy::EnrollOrRecover));
}

#[test]
fn an_unreachable_remote_is_unreachable_not_waiting() {
    let (_a, b, _req) = waiting();
    let offline = b.vault.clone().with_remote_syncer(FakeSyncer::offline());

    let err = offline.check_approved(&FakePrompter::new()).unwrap_err();

    assert!(matches!(err, ApprovalCheckError::Sync(_)), "{err}");
    assert_eq!(err.kind(), ErrorKind::Unreachable);
}

#[test]
fn an_unregistered_vault_is_not_found() {
    let (_a, b, _req) = waiting();
    let path = paths::global_config_file(&b.dirs);
    let mut global = config::load(&path).unwrap();
    global.vaults.clear();
    config::save_creating_dir(&path, &global).unwrap();

    let err = b.vault.check_approved(&FakePrompter::new()).unwrap_err();

    assert_eq!(err.kind(), ErrorKind::NotFound, "{err}");
}
