//! The two-device fixture every P5 milestone builds on: two gage installs
//! over one bare origin. Each has its own config, data and state roots, its
//! own registration of the vault and its own identity file. Device A is
//! [`SyncVault`], the state `gage init --remote` leaves; device B is a
//! [`Machine`] cloned from the same origin the way `gage clone` leaves one:
//! registered, with no identity and an empty `pubkey`.
//!
//! The tests in this file are M9a's; the fixture is shared with M9b
//! (re-encryption) and M10 (a pulled recipient change).

use crate::config::{self, Global, VaultEntry};
use crate::exitcode::{Code, HasCode};
use crate::gitrepo;
use crate::gittest;
use crate::identity_add::IdentityAddError;
use crate::identityfile;
use crate::prompter::FakePrompter;
use crate::remotesync::tests::{DEVICE, NAME, SyncVault};
use crate::testsupport::fast_scrypt;
use crate::vault::Vault;
use crate::xdgpaths::Dirs;
use crate::{paths, recipients, vaultconfig};

/// The passphrase device B wraps its identity with: distinct from A's, so a
/// test that opens the wrong device's file fails instead of passing by
/// coincidence.
pub(crate) const B_PASSPHRASE: &str = "a different passphrase for device b";

pub(crate) const B_DEVICE: &str = "laptop-2";

/// A second gage install over the same origin as an existing device.
pub(crate) struct Machine {
    _root: tempfile::TempDir,
    pub(crate) dirs: Dirs,
    pub(crate) vault: Vault,
}

impl Machine {
    /// What `gage clone` leaves on a new machine: the vault cloned into
    /// this machine's own data root and registered under `device`, with no
    /// identity file and no recorded public key.
    pub(crate) fn cloned_from(first: &SyncVault, device: &str) -> Machine {
        fast_scrypt();
        let root = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            config: root.path().join("config"),
            data: root.path().join("data"),
            state: root.path().join("state"),
        };
        let origin = first.remote.path().to_str().unwrap().to_string();
        let path = paths::vaults_dir(&dirs).join(NAME);
        gitrepo::clone(&origin, &path, &gittest::remote_options()).expect("cloning the origin");
        let file = vaultconfig::read(&path.join(".gage").join("config.toml")).unwrap();
        let entry = VaultEntry {
            path: path.clone(),
            id: file.vault.id.clone(),
            vault_type: file.vault.vault_type.clone(),
            device: device.into(),
            pubkey: String::new(),
            method: file.method.default.clone(),
            git: config::GitMeta { origin },
        };
        let mut global = Global::default();
        global.register(NAME, entry.clone()).unwrap();
        config::save(&paths::global_config_file(&dirs), &global).unwrap();
        let vault = Vault::from_registration(&dirs, NAME, &entry).unwrap();
        Machine {
            _root: root,
            dirs,
            vault,
        }
    }

    /// Registers this device's identity for the vault and returns its
    /// public key. Records it in global config as the CLI does, since the
    /// library never writes global config.
    pub(crate) fn identity_add(&self, device: &str) -> String {
        let p = FakePrompter::new().with_passphrase(B_PASSPHRASE);
        let pubkey = self.vault.add_identity(device, &p).unwrap();
        let path = paths::global_config_file(&self.dirs);
        let mut global = config::load(&path).unwrap();
        let entry = global.vaults.get_mut(NAME).unwrap();
        entry.device = device.into();
        entry.pubkey = pubkey.clone();
        config::save(&path, &global).unwrap();
        pubkey
    }

    pub(crate) fn unlock(&self) -> crate::identity::Identity {
        self.vault
            .unlock(&FakePrompter::new().with_passphrase(B_PASSPHRASE))
            .unwrap()
    }
}

/// Replaces `.age-recipients` in the vault's working tree, uncommitted: a
/// hand edit, for the divergence tests.
pub(crate) fn edit_recipients_file(vault: &Vault, keys: &[String]) {
    recipients::write(&vault.path.join(".age-recipients"), keys).unwrap();
}

/// Replaces `[[recipients]]` in the vault's `.gage/config.toml`,
/// uncommitted.
pub(crate) fn edit_config_recipients(vault: &Vault, list: Vec<vaultconfig::Recipient>) {
    let path = vault.config_path();
    let mut file = vaultconfig::read(&path).unwrap();
    file.recipients = list;
    vaultconfig::write(&path, &file).unwrap();
}

// Task 1: device A inits and pushes, device B clones and registers its own
// identity. B has its own file under its own `$GAGE_DATA/identities/<id>/`,
// and A's is unaffected.
#[test]
fn a_second_device_registers_its_own_identity_and_leaves_the_first_alone() {
    let a = SyncVault::new();
    let a_file = paths::identity_file_path(a.vault.dirs(), &a.vault.id, DEVICE).unwrap();
    let a_before = std::fs::read(&a_file).unwrap();

    let b = Machine::cloned_from(&a, B_DEVICE);
    let b_file = paths::identity_file_path(&b.dirs, &b.vault.id, B_DEVICE).unwrap();
    assert!(!b_file.exists(), "a clone holds no identity");

    let b_key = b.identity_add(B_DEVICE);

    assert!(b_key.starts_with("age1"), "{b_key}");
    assert!(b_file.is_file());
    assert!(
        b_file.starts_with(b.dirs.data.join("identities").join(&b.vault.id)),
        "{}",
        b_file.display()
    );
    assert_ne!(a_file, b_file, "two installs, two roots");
    assert_eq!(std::fs::read(&a_file).unwrap(), a_before);
    // A still unlocks with its own passphrase; B's is its own.
    let _ = a.unlock();
    let _ = b.unlock();
}

fn stray_key() -> String {
    age::x25519::Identity::generate().to_public().to_string()
}

fn other_passphrase() -> FakePrompter {
    FakePrompter::new().with_passphrase("yet another passphrase")
}

// --- recipients and verify_recipients (task 3) ---------------------------

// Go: TestRecipientListMatchesConfigExactly. Order and labels are the
// config's own, not re-derived from `.age-recipients`.
#[test]
fn recipients_match_the_config_exactly() {
    let a = SyncVault::new();
    let first = a.vault.recipients().unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].device, DEVICE);

    let listed = vec![
        vaultconfig::Recipient {
            device: "zebra".into(),
            pubkey: stray_key(),
        },
        first[0].clone(),
        vaultconfig::Recipient {
            device: "aardvark".into(),
            pubkey: stray_key(),
        },
    ];
    edit_config_recipients(&a.vault, listed.clone());

    assert_eq!(a.vault.recipients().unwrap(), listed);
}

// Go: TestVerifyReportsInSyncAndTheSpecificDifferences, both directions.
#[test]
fn verify_reports_in_sync_and_the_specific_differences() {
    let a = SyncVault::new();
    let verdict = a.vault.verify_recipients().unwrap();
    assert!(verdict.in_sync, "{verdict:?}");
    assert!(verdict.only_in_recipients_file.is_empty());
    assert!(verdict.only_in_config.is_empty());

    let own = a.vault.recipients().unwrap()[0].pubkey.clone();

    // A key only in `.age-recipients`.
    let stray = stray_key();
    edit_recipients_file(&a.vault, &[own.clone(), stray.clone()]);
    let verdict = a.vault.verify_recipients().unwrap();
    assert!(!verdict.in_sync);
    assert_eq!(verdict.only_in_recipients_file, [stray]);
    assert!(verdict.only_in_config.is_empty());

    // A key only in `config.toml`.
    edit_recipients_file(&a.vault, std::slice::from_ref(&own));
    let listed = stray_key();
    edit_config_recipients(
        &a.vault,
        vec![
            vaultconfig::Recipient {
                device: DEVICE.into(),
                pubkey: own,
            },
            vaultconfig::Recipient {
                device: "ghost".into(),
                pubkey: listed.clone(),
            },
        ],
    );
    let verdict = a.vault.verify_recipients().unwrap();
    assert!(!verdict.in_sync);
    assert!(verdict.only_in_recipients_file.is_empty());
    assert_eq!(verdict.only_in_config, [listed]);
}

// Go: TestVerifyNeedsNoIdentityFileOrPrompter. Device B has just cloned: a
// registration, no identity file, no prompter in reach.
#[test]
fn verify_needs_no_identity_file_or_prompter() {
    let a = SyncVault::new();
    let b = Machine::cloned_from(&a, B_DEVICE);
    assert!(
        !paths::identities_dir(&b.dirs, &b.vault.id)
            .unwrap()
            .exists()
    );

    assert!(b.vault.verify_recipients().unwrap().in_sync);
    assert_eq!(b.vault.recipients().unwrap(), a.vault.recipients().unwrap());
}

// Go: TestVerifyReadsNeitherIdentityNorPrompter. `verify_recipients` takes
// no `Prompter` and no `Identity`, so the compiler proves the Go test's
// point; what remains to port is that a divergence is still reported on a
// device that holds no identity.
#[test]
fn verify_reports_a_divergence_on_a_device_with_no_identity() {
    let a = SyncVault::new();
    let b = Machine::cloned_from(&a, B_DEVICE);
    let stray = stray_key();
    let own = b.vault.recipients().unwrap()[0].pubkey.clone();
    edit_recipients_file(&b.vault, &[own, stray.clone()]);

    let verdict = b.vault.verify_recipients().unwrap();
    assert!(!verdict.in_sync);
    assert_eq!(verdict.only_in_recipients_file, [stray]);
}

// --- add_identity and list_identities (task 4) ----------------------------

// Go: TestAddIdentityWritesItsOwnWrappedFileAndReturnsAPublicKey.
#[test]
fn add_identity_writes_its_own_wrapped_file_and_returns_a_public_key() {
    let a = SyncVault::new();
    let b = Machine::cloned_from(&a, B_DEVICE);

    let key = b
        .vault
        .add_identity(B_DEVICE, &FakePrompter::new().with_passphrase(B_PASSPHRASE))
        .unwrap();

    key.parse::<age::x25519::Recipient>().unwrap();
    let path = paths::identity_file_path(&b.dirs, &b.vault.id, B_DEVICE).unwrap();
    let wrapped = std::fs::read(&path).unwrap();
    // The file is the wrapped key, not the public key, and opens to the
    // key that was returned.
    assert!(!String::from_utf8_lossy(&wrapped).contains(&key));
    let opened = identityfile::open(
        NAME,
        B_DEVICE,
        &wrapped,
        &FakePrompter::new().with_passphrase(B_PASSPHRASE),
    )
    .unwrap();
    assert_eq!(opened.recipient(), key);
    assert!(
        path.parent()
            .unwrap()
            .join(identityfile::MARKER_FILE_NAME)
            .is_file()
    );
}

// Go: TestEachDeviceHasItsOwnIdentityFile. Deleting one leaves the other
// openable, with its own key.
#[test]
fn each_device_has_its_own_identity_file() {
    let a = SyncVault::new();
    let b = Machine::cloned_from(&a, B_DEVICE);
    let first = b
        .vault
        .add_identity(B_DEVICE, &FakePrompter::new().with_passphrase(B_PASSPHRASE))
        .unwrap();
    let second = b
        .vault
        .add_identity("desktop", &other_passphrase())
        .unwrap();
    assert_ne!(first, second);

    let first_path = paths::identity_file_path(&b.dirs, &b.vault.id, B_DEVICE).unwrap();
    let second_path = paths::identity_file_path(&b.dirs, &b.vault.id, "desktop").unwrap();
    assert_ne!(first_path, second_path);
    std::fs::remove_file(&first_path).unwrap();

    let wrapped = std::fs::read(&second_path).unwrap();
    let opened = identityfile::open(NAME, "desktop", &wrapped, &other_passphrase()).unwrap();
    assert_eq!(opened.recipient(), second);
}

// Go: TestAddIdentityRejectsANameAlreadyRegisteredAsARecipient. Device B
// asks for the label device A holds; nothing is written.
#[test]
fn add_identity_rejects_a_name_already_registered_as_a_recipient() {
    let a = SyncVault::new();
    let b = Machine::cloned_from(&a, B_DEVICE);

    let err = b
        .vault
        .add_identity(DEVICE, &other_passphrase())
        .unwrap_err();

    assert!(
        matches!(&err, IdentityAddError::DeviceNameTaken { device, .. } if device == DEVICE),
        "{err:?}"
    );
    assert_eq!(err.code(), Code::Conflict);
    assert!(
        !paths::identities_dir(&b.dirs, &b.vault.id)
            .unwrap()
            .exists(),
        "a refused name must leave no directory behind"
    );
}

// Go: TestAddIdentityRejectsAnInvalidDeviceName.
#[test]
fn add_identity_rejects_an_invalid_device_name() {
    let a = SyncVault::new();
    let b = Machine::cloned_from(&a, B_DEVICE);
    for bad in ["", "../escape", "has space", "Upper", "..", "a/b"] {
        let err = b.vault.add_identity(bad, &other_passphrase()).unwrap_err();
        assert!(
            matches!(&err, IdentityAddError::InvalidDeviceName(n) if n == bad),
            "{bad:?}: {err:?}"
        );
        assert_eq!(err.code(), Code::Usage);
    }
    assert!(
        !paths::identities_dir(&b.dirs, &b.vault.id)
            .unwrap()
            .exists()
    );
}

// Go: TestListIdentitiesReportsThisDevicesRegisteredIdentities. `current`
// marks the registered device only.
#[test]
fn list_identities_reports_this_devices_registered_identities() {
    let a = SyncVault::new();
    let listed = a.vault.list_identities().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].device, DEVICE);
    assert!(listed[0].current);
    assert_eq!(
        listed[0].path,
        paths::identity_file_path(a.vault.dirs(), &a.vault.id, DEVICE).unwrap()
    );

    let b = Machine::cloned_from(&a, B_DEVICE);
    assert!(b.vault.list_identities().unwrap().is_empty());
    b.identity_add(B_DEVICE);
    b.vault
        .add_identity("desktop", &other_passphrase())
        .unwrap();
    let listed = b.vault.list_identities().unwrap();
    let summary: Vec<(&str, bool)> = listed
        .iter()
        .map(|i| (i.device.as_str(), i.current))
        .collect();
    assert_eq!(summary, [("desktop", false), (B_DEVICE, true)]);
}

// Go: TestListIdentitiesSkipsSubdirectoriesAndInvalidlyNamedFiles.
#[test]
fn list_identities_skips_subdirectories_and_invalidly_named_files() {
    let a = SyncVault::new();
    let dir = paths::identities_dir(a.vault.dirs(), &a.vault.id).unwrap();
    std::fs::create_dir(dir.join("subdir.age")).unwrap();
    std::fs::write(dir.join("Bad Name.age"), b"x").unwrap();
    std::fs::write(dir.join("..age"), b"x").unwrap();
    std::fs::write(dir.join("notes.txt"), b"x").unwrap();

    let devices: Vec<String> = a
        .vault
        .list_identities()
        .unwrap()
        .into_iter()
        .map(|i| i.device)
        .collect();
    assert_eq!(devices, [DEVICE]);
}
