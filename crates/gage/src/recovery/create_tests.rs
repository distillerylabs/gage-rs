//! Labelled extra recipients at `create`.

use super::*;
use crate::entry::NewEntry;
use crate::error::{ErrorKind, HasKind};
use crate::gitrepo;
use crate::identity::Identity;
use crate::prompter::FakePrompter;
use crate::recipient_tests::dirs_in;
use crate::recipients;
use crate::testsupport::fast_scrypt;
use crate::vault::{CreateError, CreateSpec, LabelledRecipient, Vault};
use crate::vaultconfig;

const DEVICE: &str = "laptop-1";

fn key() -> String {
    age::x25519::Identity::generate().to_public().to_string()
}

fn spec(root: &tempfile::TempDir, recipients: Vec<String>) -> CreateSpec {
    CreateSpec {
        name: "personal".into(),
        id: vaultconfig::VaultId::new(),
        path: root.path().join("vault"),
        vault_type: crate::vaultconfig::VaultType::Git,
        method: "passphrase".into(),
        device: DEVICE.into(),
        recipients,
        extra_recipients: Vec::new(),
        remote: None,
    }
}

fn extra(device: &str, pubkey: &str) -> LabelledRecipient {
    LabelledRecipient {
        device: device.into(),
        pubkey: pubkey.into(),
    }
}

fn labels(vault: &Vault) -> Vec<String> {
    vault
        .recipients()
        .unwrap()
        .into_iter()
        .map(|r| r.device)
        .collect()
}

#[test]
fn create_writes_labelled_extra_recipient_to_both_files_in_one_commit() {
    let root = tempfile::tempdir().unwrap();
    let (device_key, recovery) = (key(), key());
    let mut s = spec(&root, vec![device_key.clone()]);
    s.extra_recipients = vec![extra(RECOVERY_DEVICE_LABEL, &recovery)];

    let vault = Vault::create(&dirs_in(root.path()), &s).unwrap();

    assert_eq!(gitrepo::commit_count(&vault.path).unwrap(), 1);
    assert!(gitrepo::is_clean(&vault.path).unwrap());
    assert_eq!(labels(&vault), [DEVICE, RECOVERY_DEVICE_LABEL]);
    let in_file = recipients::load(&vault.path.join(".age-recipients")).unwrap();
    assert_eq!(in_file, [device_key, recovery]);
}

#[test]
fn create_orders_device_then_labelled_extras_then_generic_recipients() {
    let root = tempfile::tempdir().unwrap();
    let (device_key, recovery, flag) = (key(), key(), key());
    let mut s = spec(&root, vec![device_key.clone(), flag.clone()]);
    s.extra_recipients = vec![extra(RECOVERY_DEVICE_LABEL, &recovery)];

    let vault = Vault::create(&dirs_in(root.path()), &s).unwrap();

    // N is the position in `recipients`, with the device key at index 0:
    // an extra does not shift the numbering.
    assert_eq!(
        labels(&vault),
        [DEVICE, RECOVERY_DEVICE_LABEL, "recipient-2"]
    );
    let expected = [device_key, recovery, flag];
    let config_keys: Vec<String> = vault
        .recipients()
        .unwrap()
        .into_iter()
        .map(|r| r.pubkey)
        .collect();
    assert_eq!(config_keys, expected);
    assert_eq!(
        recipients::load(&vault.path.join(".age-recipients")).unwrap(),
        expected
    );
}

#[test]
fn create_rejects_duplicate_or_invalid_extra_recipients() {
    let (device_key, flag, recovery) = (key(), key(), key());
    type Mutate = Box<dyn Fn(&mut CreateSpec)>;
    let cases: Vec<(&str, Mutate, ErrorKind)> = vec![
        (
            "extra equals the device's key",
            Box::new({
                let (d, r) = (device_key.clone(), recovery.clone());
                move |s| {
                    s.recipients = vec![d.clone()];
                    s.extra_recipients = vec![extra("recovery-a", &d), extra("recovery-b", &r)];
                }
            }),
            ErrorKind::Conflict,
        ),
        (
            "extra equals a --recipient key",
            Box::new({
                let (d, f) = (device_key.clone(), flag.clone());
                move |s| {
                    s.recipients = vec![d.clone(), f.clone()];
                    s.extra_recipients = vec![extra("recovery-a", &f)];
                }
            }),
            ErrorKind::Conflict,
        ),
        (
            "two extras share a key",
            Box::new({
                let (d, r) = (device_key.clone(), recovery.clone());
                move |s| {
                    s.recipients = vec![d.clone()];
                    s.extra_recipients = vec![extra("recovery-a", &r), extra("recovery-b", &r)];
                }
            }),
            ErrorKind::Conflict,
        ),
        (
            "two extras share a label",
            Box::new({
                let d = device_key.clone();
                move |s| {
                    s.recipients = vec![d.clone()];
                    s.extra_recipients = vec![extra("same", &key()), extra("same", &key())];
                }
            }),
            ErrorKind::Conflict,
        ),
        (
            "extra label equals the device's",
            Box::new({
                let d = device_key.clone();
                move |s| {
                    s.recipients = vec![d.clone()];
                    s.extra_recipients = vec![extra(DEVICE, &key())];
                }
            }),
            ErrorKind::Conflict,
        ),
        (
            "extra label equals a generated recipient-N label",
            Box::new({
                let (d, f) = (device_key.clone(), flag.clone());
                move |s| {
                    s.recipients = vec![d.clone(), f.clone()];
                    s.extra_recipients = vec![extra("recipient-2", &key())];
                }
            }),
            ErrorKind::Conflict,
        ),
        (
            "extra key is not an age recipient",
            Box::new({
                let d = device_key.clone();
                move |s| {
                    s.recipients = vec![d.clone()];
                    s.extra_recipients = vec![extra("recovery-a", "nope")];
                }
            }),
            ErrorKind::Usage,
        ),
        (
            "extra label is not a valid device name",
            Box::new({
                let d = device_key.clone();
                move |s| {
                    s.recipients = vec![d.clone()];
                    s.extra_recipients = vec![extra("../x", &key())];
                }
            }),
            ErrorKind::Usage,
        ),
    ];
    for (name, mutate, code) in cases {
        let root = tempfile::tempdir().unwrap();
        let mut s = spec(&root, vec![]);
        mutate(&mut s);
        let err = Vault::create(&dirs_in(root.path()), &s).unwrap_err();
        assert_eq!(err.kind(), code, "{name}: {err}");
        assert!(!s.path.exists(), "{name}: left a directory behind");
    }
}

#[test]
fn a_duplicate_is_recipient_exists_shaped() {
    let root = tempfile::tempdir().unwrap();
    let (d, r) = (key(), key());
    let mut s = spec(&root, vec![d]);
    s.extra_recipients = vec![extra("recovery-a", &r), extra("recovery-b", &r)];
    let err = Vault::create(&dirs_in(root.path()), &s).unwrap_err();
    assert!(
        matches!(&err, CreateError::RecipientExists { pubkey: Some(p), .. } if *p == r),
        "{err:?}"
    );
}

#[test]
fn collisions_knowable_from_the_flags_fail_before_the_device_key_exists() {
    // `init` validates before it generates the device key, so the list it
    // checks holds the --recipient keys only.
    let root = tempfile::tempdir().unwrap();
    let (flag, other) = (key(), key());
    let mut s = spec(&root, vec![flag.clone()]);
    s.extra_recipients = vec![extra(RECOVERY_DEVICE_LABEL, &flag)];
    assert_eq!(
        s.validate_without_device_key().unwrap_err().kind(),
        ErrorKind::Conflict
    );
    s.extra_recipients = vec![extra(RECOVERY_DEVICE_LABEL, &other)];
    s.validate_without_device_key().unwrap();
}

#[test]
fn create_without_extra_recipients_still_allows_repeated_recipient_keys() {
    let root = tempfile::tempdir().unwrap();
    let (a, b) = (key(), key());
    let s = spec(&root, vec![a.clone(), b.clone(), a.clone()]);
    let vault = Vault::create(&dirs_in(root.path()), &s).unwrap();
    assert_eq!(
        recipients::load(&vault.path.join(".age-recipients")).unwrap(),
        [a.clone(), b, a]
    );
}

#[test]
fn a_repeated_recipient_stays_legal_when_no_extra_is_involved() {
    let root = tempfile::tempdir().unwrap();
    let (a, b, recovery) = (key(), key(), key());
    let mut s = spec(&root, vec![a.clone(), b.clone(), b.clone()]);
    s.extra_recipients = vec![extra(RECOVERY_DEVICE_LABEL, &recovery)];
    let vault = Vault::create(&dirs_in(root.path()), &s).unwrap();
    assert_eq!(
        labels(&vault),
        [DEVICE, RECOVERY_DEVICE_LABEL, "recipient-2", "recipient-3"]
    );
}

#[test]
fn entry_encrypted_to_the_vault_decrypts_with_the_recovery_secret_alone() {
    fast_scrypt();
    let root = tempfile::tempdir().unwrap();
    let device = age::x25519::Identity::generate();
    let recovery = new_recovery_key().unwrap();
    let mut s = spec(&root, vec![device.to_public().to_string()]);
    s.extra_recipients = vec![extra(RECOVERY_DEVICE_LABEL, &recovery.pubkey)];
    let vault = Vault::create(&dirs_in(root.path()), &s).unwrap();
    let id = vault
        .insert(
            &Identity::for_tests(&device),
            NewEntry {
                title: "t".into(),
                value: "the-secret".into(),
                ..NewEntry::default()
            },
            true,
            &FakePrompter::new(),
        )
        .unwrap();

    // Only the pasted secret: no device identity, no unlock.
    let parsed: age::x25519::Identity = recovery.secret.parse().unwrap();
    let entry = vault.read_entry(id, &Identity::for_tests(&parsed)).unwrap();
    assert_eq!(entry.value, "the-secret");
}

#[test]
fn a_recipient_n_label_collision_fails_before_the_device_key_exists() {
    // Before the device key is prepended, one --recipient is about to be
    // labelled `recipient-2`.
    let root = tempfile::tempdir().unwrap();
    let mut s = spec(&root, vec![key()]);
    s.extra_recipients = vec![extra("recipient-2", &key())];
    assert_eq!(
        s.validate_without_device_key().unwrap_err().kind(),
        ErrorKind::Conflict
    );
}

#[test]
fn a_key_collision_names_the_label_that_holds_the_key() {
    let root = tempfile::tempdir().unwrap();
    let (d, f) = (key(), key());
    let mut s = spec(&root, vec![d, f.clone()]);
    s.extra_recipients = vec![extra("recovery-a", &f)];
    match Vault::create(&dirs_in(root.path()), &s).unwrap_err() {
        CreateError::RecipientExists { device, .. } => assert_eq!(device, "recipient-2"),
        other => panic!("{other:?}"),
    }
}
