//! `recovery-paper-key` is reserved for the recovery key. The approval
//! paths are in `enrollment/reserved_tests.rs`, which has the fixtures.

use std::time::Duration;

use super::*;
use crate::exitcode::{Code, HasCode};
use crate::identity_add::IdentityAddError;
use crate::prompter::FakePrompter;
use crate::recipient::{RecipientError, RecipientWrite};
use crate::recipient_tests::{Fixture, dirs_in};
use crate::vault::{CreateError, CreateSpec, LabelledRecipient, Vault};
use crate::vaultconfig::{self, Recipient};

fn key() -> String {
    age::x25519::Identity::generate().to_public().to_string()
}

fn spec_for(root: &tempfile::TempDir, device: &str) -> CreateSpec {
    CreateSpec {
        name: "personal".into(),
        id: vaultconfig::new_id(),
        path: root.path().join("vault"),
        vault_type: "git".into(),
        method: "passphrase".into(),
        device: device.into(),
        recipients: vec![key()],
        extra_recipients: Vec::new(),
        remote: None,
    }
}

#[test]
fn the_recovery_label_is_reserved_for_the_recovery_key() {
    // create refuses it as a device name even with no recovery key, and
    // leaves no directory.
    let root = tempfile::tempdir().unwrap();
    let spec = spec_for(&root, RECOVERY_DEVICE_LABEL);
    let err = Vault::create(&dirs_in(root.path()), &spec).unwrap_err();
    assert!(matches!(err, CreateError::ReservedDeviceLabel(_)), "{err}");
    assert_eq!(err.code(), Code::Usage);
    assert!(!spec.path.exists());
    // Also from the check `init` runs before it asks for a passphrase.
    assert_eq!(
        spec.validate_without_device_key().unwrap_err().code(),
        Code::Usage
    );

    let f = Fixture::new();
    let commits = f.commits();

    // add_identity refuses it.
    let err = f
        .vault
        .add_identity(RECOVERY_DEVICE_LABEL, &FakePrompter::new())
        .unwrap_err();
    assert!(
        matches!(err, IdentityAddError::ReservedDeviceLabel(_)),
        "{err}"
    );
    assert_eq!(err.code(), Code::Usage);

    // add_recipient refuses it.
    let err = f
        .vault
        .add_recipient(
            RECOVERY_DEVICE_LABEL,
            &key(),
            &f.identity(),
            &FakePrompter::new(),
        )
        .unwrap_err();
    assert!(
        matches!(err, RecipientError::ReservedDeviceLabel(_)),
        "{err}"
    );
    assert_eq!(err.code(), Code::Usage);
    assert_eq!(f.commits(), commits);
    assert!(f.is_clean());

    // The recovery recipient itself is still written under the label, by
    // create's extras.
    let root = tempfile::tempdir().unwrap();
    let mut spec = spec_for(&root, "laptop-1");
    spec.extra_recipients = vec![LabelledRecipient {
        device: RECOVERY_DEVICE_LABEL.into(),
        pubkey: key(),
    }];
    let vault = Vault::create(&dirs_in(root.path()), &spec).unwrap();
    assert_eq!(vault.recipients().unwrap()[1].device, RECOVERY_DEVICE_LABEL);
}

#[test]
fn the_batch_form_checks_every_member() {
    // The approval path hands `add_recipient_locked` a batch; a reserved
    // label anywhere in it refuses the whole batch before a byte is
    // written.
    let f = Fixture::new();
    let commits = f.commits();
    let batch = [
        Recipient {
            device: "phone".into(),
            pubkey: key(),
        },
        Recipient {
            device: RECOVERY_DEVICE_LABEL.into(),
            pubkey: key(),
        },
    ];
    let err = f
        .vault
        .add_recipient_locked(
            &batch,
            &RecipientWrite::message("test"),
            &f.identity(),
            &FakePrompter::new(),
        )
        .unwrap_err();
    assert!(
        matches!(err, RecipientError::ReservedDeviceLabel(_)),
        "{err}"
    );
    assert_eq!(f.commits(), commits);
    assert!(f.is_clean());
    assert_eq!(f.vault.recipients().unwrap().len(), 1);
}

#[test]
fn enroll_refuses_the_recovery_label() {
    let f = Fixture::new();
    let commits = f.commits();
    let err = f
        .vault
        .enroll(
            RECOVERY_DEVICE_LABEL,
            Duration::from_secs(3600),
            &FakePrompter::new(),
        )
        .unwrap_err();
    assert!(
        matches!(
            err,
            crate::enrollment::EnrollmentError::ReservedDeviceLabel(_)
        ),
        "{err}"
    );
    assert_eq!(err.code(), Code::Usage);
    assert_eq!(f.commits(), commits);
}
