//! RK1 task 1: the recovery keypair. Go: `recovery_key_test.go`.

use age::secrecy::ExposeSecret;
use zeroize::Zeroize;

use super::*;
use crate::agekey;

#[test]
fn new_recovery_key_returns_a_valid_recipient() {
    let key = new_recovery_key().unwrap();
    agekey::validate_recipient(&key.pubkey).unwrap();
    assert!(key.pubkey.starts_with("age1"));
}

#[test]
fn new_recovery_key_secret_parses_back_to_the_same_pubkey() {
    let key = new_recovery_key().unwrap();
    assert!(key.secret.starts_with("AGE-SECRET-KEY-1"));
    let parsed: age::x25519::Identity = key.secret.parse().unwrap();
    assert_eq!(parsed.to_public().to_string(), key.pubkey);
}

#[test]
fn new_recovery_key_is_different_each_call() {
    let (a, b) = (new_recovery_key().unwrap(), new_recovery_key().unwrap());
    assert_ne!(a.pubkey, b.pubkey);
    assert_ne!(a.secret.as_str(), b.secret.as_str());
}

#[test]
fn zeroing_recovery_secret_leaves_pubkey_intact() {
    let mut key = new_recovery_key().unwrap();
    let pubkey = key.pubkey.clone();
    key.secret.zeroize();
    assert!(key.secret.is_empty());
    assert_eq!(key.pubkey, pubkey);
}

#[test]
fn recovery_key_debug_redacts_the_secret() {
    let key = new_recovery_key().unwrap();
    let shown = format!("{key:?}");
    assert!(!shown.contains(key.secret.as_str()));
    assert!(!shown.contains("AGE-SECRET-KEY"));
    assert!(shown.contains(&key.pubkey));
    // Keep the import honest: the secret is what age itself would print.
    let parsed: age::x25519::Identity = key.secret.parse().unwrap();
    assert_eq!(parsed.to_string().expose_secret(), key.secret.as_str());
}

#[test]
fn the_recovery_label_is_the_documented_one() {
    assert_eq!(RECOVERY_DEVICE_LABEL, "recovery-paper-key");
}
