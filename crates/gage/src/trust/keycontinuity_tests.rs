//! Key continuity (ADR-0067), as library tests: a device refuses a key file
//! that doesn't continue the key it last saw, and every legitimate history
//! keeps reading.
//!
//! The forger here does what issue #207's does: seals a key of their own to
//! the public list and replaces every entry with one tagged under it, so
//! each entry authenticates against the file beside it.

use uuid::Uuid;

use crate::crypto::crypt;
use crate::crypto::identity::Identity;
use crate::crypto::vaultkey::{self, VAULT_KEY_FILE, VaultKey};
use crate::entry;
use crate::error::{ErrorKind, HasKind, Remedy};
use crate::sync::gitrepo;
use crate::sync::remotesync::tests::SyncVault;
use crate::trust::trustcache;
use crate::trust::trustcache_tests::{
    TrustVault, commit_routine_change, new_key, trusting, unlocking,
};
use crate::vault::Vault;

const RECORD: &str = "key-continuity.toml";

/// What the record holds, read by hand: "the device recorded this" is a
/// claim about a file, not about the code that wrote it.
fn recorded(dirs: &crate::storage::xdgpaths::Dirs, vault: &Vault) -> Option<String> {
    let text = std::fs::read_to_string(dirs.state.join(&vault.id).join(RECORD)).ok()?;
    let table: toml::Table = text.parse().unwrap();
    Some(table["fingerprint"].as_str().unwrap().to_string())
}

fn key_on_disk(vault: &Vault, identity: &Identity) -> VaultKey {
    let sealed = std::fs::read(vault.path.join(VAULT_KEY_FILE)).unwrap();
    VaultKey::open(&sealed, identity).ok().unwrap()
}

/// A rotation as another member's commit arrives: the tree and HEAD move.
fn rotate_and_commit(vault: &Vault, identity: &Identity) {
    let to = vault.encrypt_recipients().unwrap();
    key_on_disk(vault, identity)
        .rotated()
        .save(&vault.path, &to)
        .unwrap();
    gitrepo::commit_all(&vault.path, "gage: rotate").unwrap();
}

/// Replaces the key file and every entry with the forger's, each entry
/// keeping the title it had and reading `forged`.
fn forge(vault: &Vault, identity: &Identity) {
    let to = vault.encrypt_recipients().unwrap();
    let key = VaultKey::generate();
    for id in vault.entry_ids().unwrap() {
        let mut e = vault.read_entry(id, identity).unwrap();
        e.value = "forged".into();
        vault.write_entry_to(id, &e, &to, &key).unwrap();
    }
    key.save(&vault.path, &to).unwrap();
    gitrepo::commit_all(&vault.path, "replace everything").unwrap();
}

fn refused(err: &vaultkey::Error) -> bool {
    matches!(err, vaultkey::Error::Discontinuous { .. })
}

fn assert_refused(result: Result<VaultKey, entry::Error>) {
    match result {
        Err(entry::Error::VaultKey(e)) => assert!(refused(&e), "{e}"),
        other => panic!("want a refusal, got {other:?}"),
    }
}

/// A vault this device has created, with one genuine entry.
fn with_entry() -> (TrustVault, Identity, Uuid) {
    let t = TrustVault::new();
    let identity = t.unlock(&unlocking());
    let id = t.insert(&identity, "Pw", &unlocking());
    (t, identity, id)
}

#[test]
fn init_records_the_key_it_minted() {
    let t = TrustVault::new();
    let identity = t.unlock(&unlocking());
    assert_eq!(
        recorded(&t.dirs, &t.vault).unwrap(),
        key_on_disk(&t.vault, &identity).fingerprint()
    );
}

#[test]
fn the_record_holds_a_fingerprint_and_never_the_key() {
    let t = TrustVault::new();
    let text = std::fs::read_to_string(t.dirs.state.join(&t.vault.id).join(RECORD)).unwrap();
    let table: toml::Table = text.parse().unwrap();
    assert_eq!(table.len(), 1, "{text}");
    assert_eq!(table["fingerprint"].as_str().unwrap().len(), 64);
}

#[test]
fn a_replaced_key_and_every_entry_are_refused_by_every_read() {
    let (t, identity, id) = with_entry();
    forge(&t.vault, &identity);

    let err = t.vault.read_entry(id, &identity).unwrap_err();
    assert!(
        matches!(&err, entry::Error::VaultKey(e) if refused(e)),
        "{err}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(err.remedy(), Some(Remedy::KeyContinuity));
    assert!(t.vault.list(&identity).is_err());
    assert!(t.vault.resolve(&identity, "Pw").is_err());
    assert!(t.vault.history(id, &identity).is_err());
}

#[test]
fn the_refusal_holds_when_the_attacker_also_adds_a_recipient() {
    let (t, identity, id) = with_entry();
    commit_routine_change(&t.vault, "mallory", &new_key());
    forge(&t.vault, &identity);
    let err = t.vault.read_entry(id, &identity).unwrap_err();
    assert!(
        matches!(&err, entry::Error::VaultKey(e) if refused(e)),
        "{err}"
    );
}

#[test]
fn yes_does_not_buy_a_write_over_a_refused_key() {
    let (t, identity, _) = with_entry();
    forge(&t.vault, &identity);
    let before = t.head();
    let err = t
        .try_insert(&identity, "After", &trusting(&[true]))
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict, "{err}");
    assert_eq!(t.head(), before);
}

#[test]
fn a_refusal_leaves_the_record_where_it_was() {
    let (t, identity, _) = with_entry();
    let before = recorded(&t.dirs, &t.vault);
    forge(&t.vault, &identity);
    assert_refused(t.vault.vault_key(&identity));
    assert_eq!(recorded(&t.dirs, &t.vault), before);
}

#[test]
fn restoring_the_good_key_file_reads_again() {
    let (t, identity, id) = with_entry();
    let key = t.file(VAULT_KEY_FILE);
    let entry = t.file(&format!("entries/{id}.age"));
    forge(&t.vault, &identity);
    assert_refused(t.vault.vault_key(&identity));

    std::fs::write(t.vault.path.join(VAULT_KEY_FILE), key).unwrap();
    std::fs::write(t.vault.path.join(format!("entries/{id}.age")), entry).unwrap();
    gitrepo::commit_all(&t.vault.path, "restore").unwrap();
    assert_eq!(t.vault.read_entry(id, &identity).unwrap().value, "hunter2");
}

#[test]
fn a_reseal_keeps_reading_and_records_nothing_new() {
    let (t, identity, _) = with_entry();
    let before = recorded(&t.dirs, &t.vault);
    let to = t.vault.encrypt_recipients().unwrap();
    key_on_disk(&t.vault, &identity)
        .save(&t.vault.path, &to)
        .unwrap();
    gitrepo::commit_all(&t.vault.path, "gage: reseal").unwrap();
    t.vault.vault_key(&identity).unwrap();
    assert_eq!(recorded(&t.dirs, &t.vault), before);
}

#[test]
fn several_rotations_seen_at_once_read_and_advance_to_the_newest() {
    let (t, identity, _) = with_entry();
    for _ in 0..3 {
        rotate_and_commit(&t.vault, &identity);
    }
    let key = t.vault.vault_key(&identity).unwrap();
    assert_eq!(recorded(&t.dirs, &t.vault).unwrap(), key.fingerprint());
}

#[test]
fn a_rollback_across_a_rotation_this_device_saw_is_refused() {
    let (t, identity, _) = with_entry();
    let old = t.file(VAULT_KEY_FILE);
    rotate_and_commit(&t.vault, &identity);
    t.vault.vault_key(&identity).unwrap();

    std::fs::write(t.vault.path.join(VAULT_KEY_FILE), old).unwrap();
    gitrepo::commit_all(&t.vault.path, "roll back").unwrap();
    assert_refused(t.vault.vault_key(&identity));
}

// A reader during a rotation, or after a crash in one, sees a key that the
// next write's reset will discard. Recording it would refuse the vault for
// good once the old key comes back.
#[test]
fn an_uncommitted_rotation_reads_but_is_not_recorded() {
    let (t, identity, _) = with_entry();
    let before = recorded(&t.dirs, &t.vault);
    let to = t.vault.encrypt_recipients().unwrap();
    key_on_disk(&t.vault, &identity)
        .rotated()
        .save(&t.vault.path, &to)
        .unwrap();

    t.vault.vault_key(&identity).unwrap();
    assert_eq!(recorded(&t.dirs, &t.vault), before);

    gitrepo::reset_hard(&t.vault.path).unwrap();
    t.vault.vault_key(&identity).unwrap();
}

#[test]
fn a_device_with_no_record_adopts_the_key_then_holds_it() {
    let (t, identity, _) = with_entry();
    std::fs::remove_file(t.dirs.state.join(&t.vault.id).join(RECORD)).unwrap();

    let key = t.vault.vault_key(&identity).unwrap();
    assert_eq!(recorded(&t.dirs, &t.vault).unwrap(), key.fingerprint());
    forge(&t.vault, &identity);
    assert_refused(t.vault.vault_key(&identity));
}

#[test]
fn an_unreadable_record_fails_closed() {
    let (t, identity, _) = with_entry();
    let path = t.dirs.state.join(&t.vault.id).join(RECORD);
    for bad in [
        "not toml =",
        "other = 1\n",
        "fingerprint = \"short\"\n",
        &format!("fingerprint = \"{}\"\n", "G".repeat(64)),
    ] {
        std::fs::write(&path, bad).unwrap();
        match t.vault.vault_key(&identity) {
            Err(entry::Error::VaultKey(e)) => {
                assert!(
                    matches!(*e, vaultkey::Error::BadRecord { .. }),
                    "{bad}: {e}"
                );
                assert_eq!(e.kind(), ErrorKind::Internal);
            }
            other => panic!("{bad}: {other:?}"),
        }
    }
}

// Continuity already passed, and the record stays at an ancestor of the
// key, which the next read can still advance from.
#[cfg(unix)]
#[test]
fn a_record_that_cannot_be_advanced_does_not_fail_the_read() {
    use std::os::unix::fs::PermissionsExt;
    let (t, identity, _) = with_entry();
    rotate_and_commit(&t.vault, &identity);
    let dir = t.dirs.state.join(&t.vault.id);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = t.vault.vault_key(&identity);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    result.unwrap();
}

// Removed: the key is rotated to one this device can't open, so reads fail
// as not-a-recipient and the record stays put. Re-added: the file now in
// place carries the old key among its retired ones.
#[test]
fn a_device_removed_then_re_added_reads_again_with_no_ceremony() {
    let (t, identity, _) = with_entry();
    let before = recorded(&t.dirs, &t.vault);
    let member = age::x25519::Identity::generate();
    let others = [crypt::Recipient::parse(&member.to_public().to_string()).unwrap()];
    let rotated = key_on_disk(&t.vault, &identity).rotated();
    rotated.save(&t.vault.path, &others).unwrap();
    gitrepo::commit_all(&t.vault.path, "gage: recipient remove").unwrap();

    match t.vault.vault_key(&identity) {
        Err(entry::Error::VaultKey(e)) => assert_eq!(e.kind(), ErrorKind::LockedOrAuth, "{e}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(recorded(&t.dirs, &t.vault), before);

    let to = t.vault.encrypt_recipients().unwrap();
    let again = rotated.rotated();
    again.save(&t.vault.path, &to).unwrap();
    gitrepo::commit_all(&t.vault.path, "gage: recipient add").unwrap();
    let key = t.vault.vault_key(&identity).unwrap();
    assert_eq!(recorded(&t.dirs, &t.vault).unwrap(), key.fingerprint());
    assert_eq!(key.fingerprint(), again.fingerprint());
}

// `vault remove` is the deliberate way to trust a vault afresh.
#[test]
fn removing_the_trust_cache_removes_the_record() {
    let (t, identity, _) = with_entry();
    forge(&t.vault, &identity);
    assert_refused(t.vault.vault_key(&identity));
    trustcache::remove(&t.dirs, &t.vault.id).unwrap();
    assert_eq!(recorded(&t.dirs, &t.vault), None);
    t.vault.vault_key(&identity).unwrap();
}

// --- through a real origin ----------------------------------------------

fn seen_by(s: &SyncVault) -> Option<String> {
    recorded(s.vault.dirs(), &s.vault)
}

#[test]
fn a_local_rotation_is_recorded_only_once_it_is_published() {
    let s = SyncVault::new();
    let identity = s.unlock();
    s.vault.vault_key(&identity).unwrap();
    let before = seen_by(&s);
    rotate_and_commit(&s.vault, &identity);

    s.vault.vault_key(&identity).unwrap();
    assert_eq!(seen_by(&s), before, "not on origin yet");

    gitrepo::push(&s.vault.path, &crate::gittest::remote_options()).unwrap();
    let key = s.vault.vault_key(&identity).unwrap();
    assert_eq!(seen_by(&s).unwrap(), key.fingerprint());
}

// Two devices rotate from one key and this one loses. Its key never reached
// origin, so what it adopts instead still continues what it recorded.
#[test]
fn a_rotation_that_lost_the_race_leaves_the_winner_readable() {
    let s = SyncVault::new();
    let identity = s.unlock();
    let to = s.vault.encrypt_recipients().unwrap();
    let winner = key_on_disk(&s.vault, &identity).rotated();
    rotate_and_commit(&s.vault, &identity);
    s.vault.vault_key(&identity).unwrap();

    let other = s.other();
    other.write_bytes(VAULT_KEY_FILE, &winner.seal(&to).unwrap());
    other.commit("gage: rotate");
    other.push();
    gitrepo::fetch(&s.vault.path, &crate::gittest::remote_options()).unwrap();
    let repo = git2::Repository::open(&s.vault.path).unwrap();
    let theirs = repo
        .find_reference("refs/remotes/origin/main")
        .unwrap()
        .peel_to_commit()
        .unwrap();
    repo.reset(theirs.as_object(), git2::ResetType::Hard, None)
        .unwrap();

    let key = s.vault.vault_key(&identity).unwrap();
    assert_eq!(key.fingerprint(), winner.fingerprint());
}

#[test]
fn a_forgery_that_arrives_by_pull_is_refused_and_a_restore_recovers() {
    let s = SyncVault::new();
    let identity = s.unlock();
    let id = s
        .vault
        .insert(
            &identity,
            crate::sync::remotesync::tests::sample("Pw"),
            false,
            &crate::sync::remotesync::tests::unlocks(),
        )
        .unwrap();
    let genuine_key = std::fs::read(s.vault.path.join(VAULT_KEY_FILE)).unwrap();
    let genuine_entry = std::fs::read(s.vault.entry_path(id)).unwrap();
    s.vault.vault_key(&identity).unwrap();

    let to = s.vault.encrypt_recipients().unwrap();
    let forged_key = VaultKey::generate();
    let mut forged = s.vault.read_entry(id, &identity).unwrap();
    forged.value = "forged".into();
    let other = s.other();
    other.write_bytes(
        &format!("entries/{id}.age"),
        &entry::seal_entry(id, &forged, &to, &forged_key).unwrap(),
    );
    other.write_bytes(VAULT_KEY_FILE, &forged_key.seal(&to).unwrap());
    other.commit("replace everything");
    other.push();

    s.vault.pull().unwrap();
    let err = s.vault.read_entry(id, &identity).unwrap_err();
    assert!(
        matches!(&err, entry::Error::VaultKey(e) if refused(e)),
        "{err}"
    );

    other.write_bytes(&format!("entries/{id}.age"), &genuine_entry);
    other.write_bytes(VAULT_KEY_FILE, &genuine_key);
    other.commit("restore");
    other.push();
    s.vault.pull().unwrap();
    assert_eq!(s.vault.read_entry(id, &identity).unwrap().value, "hunter2");
}
