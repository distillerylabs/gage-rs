//! `Vault::verify_recovery_key`.

use super::*;
use crate::exitcode::{Code, HasCode};
use crate::recipient_tests::dirs_in;
use crate::vault::{CreateSpec, LabelledRecipient, Vault};
use crate::vaultconfig;

struct Fixture {
    _root: tempfile::TempDir,
    vault: Vault,
    device_secret: String,
    recovery: RecoveryKey,
}

fn fixture() -> Fixture {
    use age::secrecy::ExposeSecret;
    let root = tempfile::tempdir().unwrap();
    let device = age::x25519::Identity::generate();
    let recovery = new_recovery_key().unwrap();
    let vault = Vault::create(
        &dirs_in(root.path()),
        &CreateSpec {
            name: "personal".into(),
            id: vaultconfig::VaultId::new(),
            path: root.path().join("vault"),
            vault_type: crate::vaultconfig::VaultType::Git,
            method: "passphrase".into(),
            device: "laptop-1".into(),
            recipients: vec![device.to_public().to_string()],
            extra_recipients: vec![LabelledRecipient {
                device: RECOVERY_DEVICE_LABEL.into(),
                pubkey: recovery.pubkey.clone(),
            }],
            remote: None,
        },
    )
    .unwrap();
    Fixture {
        _root: root,
        vault,
        device_secret: device.to_string().expose_secret().to_string(),
        recovery,
    }
}

#[test]
fn verify_recovery_key() {
    let f = fixture();

    // The right key.
    assert_eq!(
        f.vault.verify_recovery_key(&f.recovery.secret).unwrap(),
        RECOVERY_DEVICE_LABEL
    );

    // A device's key returns that device's label: the caller decides
    // whether that is what it wanted.
    assert_eq!(
        f.vault.verify_recovery_key(&f.device_secret).unwrap(),
        "laptop-1"
    );

    // Surrounding whitespace, as a paste leaves it.
    let padded = format!("  \n{}\r\n\t", f.recovery.secret.as_str());
    assert_eq!(
        f.vault.verify_recovery_key(&padded).unwrap(),
        RECOVERY_DEVICE_LABEL
    );

    // A valid stranger.
    let stranger = new_recovery_key().unwrap();
    let err = f.vault.verify_recovery_key(&stranger.secret).unwrap_err();
    assert!(
        matches!(&err, RecoveryError::NotARecipient { pubkey, vault }
        if *pubkey == stranger.pubkey && vault == "personal")
    );
    assert_eq!(err.code(), Code::LockedOrAuth);
    assert!(!err.to_string().contains(stranger.secret.as_str()));

    // Garbage, and a near miss: a real key with its last character cut off.
    let truncated = &f.recovery.secret[..f.recovery.secret.len() - 1];
    for garbage in ["", "hello", "age1notasecret", truncated] {
        let err = f.vault.verify_recovery_key(garbage).unwrap_err();
        assert!(matches!(err, RecoveryError::MalformedRecoveryKey), "{err}");
        assert_eq!(err.code(), Code::Usage);
        assert!(
            garbage.is_empty() || !err.to_string().contains(garbage),
            "Display echoed the input: {err}"
        );
    }
}
