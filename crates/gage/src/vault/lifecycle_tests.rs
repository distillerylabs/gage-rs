//! `Vault::init` and `Vault::clone_remote`: what each leaves on disk, and
//! in what order, when it succeeds and at each point it can fail. The CLI's
//! tests pin messages and exit codes; these pin the files.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::error::{ErrorKind, HasKind};
use crate::gittest::{self, BareRemote};
use crate::prompter::{FakePrompter, PromptError};
use crate::recovery::RECOVERY_DEVICE_LABEL;
use crate::storage::config::{self, Global, RegistryError};
use crate::storage::identityfile::{self, UnlockError};
use crate::storage::paths;
use crate::storage::vaultconfig::{self, UnlockMethod, VaultId, VaultType};
use crate::storage::xdgpaths::Dirs;
use crate::testsupport::fast_scrypt;
use crate::trust::recipient;
use crate::vault::lifecycle::hooks;
use crate::vault::{
    CloneError, CloneSpec, CreateError, InitError, InitRollbackError, InitSpec, NameError, Vault,
};

const PASSPHRASE: &str = "correct horse battery staple";
const NAME: &str = "personal";
const DEVICE: &str = "laptop-1";

/// One isolated machine: its own config, data and state roots, and a
/// scratch area for vault directories.
struct Machine {
    root: tempfile::TempDir,
    dirs: Dirs,
}

impl Machine {
    fn new() -> Machine {
        fast_scrypt();
        let root = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            config: root.path().join("config").join("gage"),
            data: root.path().join("data").join("gage"),
            state: root.path().join("state").join("gage"),
        };
        Machine { root, dirs }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.path().join(rel)
    }

    fn spec(&self, name: &str) -> InitSpec {
        InitSpec {
            name: name.into(),
            path: self.path(name),
            vault_type: VaultType::Git,
            method: UnlockMethod::Passphrase,
            device: DEVICE.into(),
            recipients: Vec::new(),
            recovery_key: None,
            remote: None,
        }
    }

    fn global(&self) -> Global {
        config::load_or_default(&paths::global_config_file(&self.dirs)).unwrap()
    }

    /// Registers `name` by hand, pointing nowhere in particular.
    fn preregister(&self, name: &str) {
        let mut global = self.global();
        global
            .register(
                name,
                config::VaultEntry {
                    path: self.path("elsewhere"),
                    id: Some(VaultId::new()),
                    device: DEVICE.into(),
                    method: UnlockMethod::Passphrase,
                    ..Default::default()
                },
            )
            .unwrap();
        config::save_creating_dir(&paths::global_config_file(&self.dirs), &global).unwrap();
    }

    /// Every identity file this machine holds, any vault.
    fn identity_files(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let Ok(vaults) = std::fs::read_dir(self.dirs.data.join("identities")) else {
            return found;
        };
        for dir in vaults {
            for file in std::fs::read_dir(dir.unwrap().path()).unwrap() {
                let path = file.unwrap().path();
                if path.extension().is_some_and(|e| e == "age") {
                    found.push(path);
                }
            }
        }
        found
    }

    /// Every per-vault directory this machine holds under its state root.
    /// The global config's lock file sits there too, and isn't one.
    fn trust_caches(&self) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.dirs.state) else {
            return Vec::new();
        };
        entries
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_dir() && path.file_name().is_some_and(|name| name != "locks"))
            .collect()
    }
}

fn passphrase() -> FakePrompter {
    FakePrompter::new().with_passphrase(PASSPHRASE)
}

fn new_pubkey() -> String {
    age::x25519::Identity::generate().to_public().to_string()
}

/// Makes a directory writable again when dropped.
#[cfg(unix)]
struct Restore(PathBuf);

#[cfg(unix)]
impl Drop for Restore {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
    }
}

/// Makes `dir` read-only until the guard drops; `None` when the process
/// ignores directory permissions (root), so the caller can skip loudly.
#[cfg(unix)]
fn read_only(dir: &std::path::Path) -> Option<Restore> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let restore = Restore(dir.to_path_buf());
    let probe = dir.join(".probe");
    if std::fs::write(&probe, b"").is_ok() {
        let _ = std::fs::remove_file(probe);
        #[allow(clippy::disallowed_macros)]
        {
            eprintln!(
                "skipping: running as root, which ignores the unwritable directory this test relies on"
            );
        }
        return None;
    }
    Some(restore)
}

// --- Vault::init --------------------------------------------------------------

#[test]
fn init_creates_registers_and_returns_the_public_key() {
    let m = Machine::new();
    let mut spec = m.spec(NAME);
    spec.method = UnlockMethod::Passphrase;
    spec.remote = Some("https://example.invalid/vault.git".into());

    let made = Vault::init(&m.dirs, &spec, &passphrase()).unwrap();

    assert_eq!(made.vault.name, NAME);
    assert_eq!(made.vault.path, spec.path);
    let global = m.global();
    let entry = global.entry(NAME).unwrap();
    assert_eq!(entry.path, spec.path);
    assert_eq!(entry.id.as_ref(), Some(&made.vault.id));
    assert_eq!(entry.vault_type, VaultType::Git);
    assert_eq!(entry.device, DEVICE);
    assert_eq!(entry.pubkey.as_deref(), Some(made.pubkey.as_str()));
    assert_eq!(entry.method, UnlockMethod::Passphrase);
    assert_eq!(entry.git.origin, spec.remote);
    assert_eq!(global.current.as_deref(), Some(NAME));

    let file = vaultconfig::load(&spec.path.join(".gage/config.toml")).unwrap();
    assert_eq!(file.recipients.len(), 1);
    assert_eq!(file.recipients[0].device, DEVICE);
    assert_eq!(file.recipients[0].pubkey, made.pubkey);
    // A registration every other command accepts, and an unlock that works.
    let vault = Vault::from_registration(&m.dirs, NAME, entry).unwrap();
    let identity = vault.unlock(&passphrase()).unwrap();
    assert_eq!(identity.recipient(), made.pubkey);
}

#[test]
fn init_files_the_identity_under_the_id_the_vault_carries() {
    let m = Machine::new();
    let spec = m.spec(NAME);
    let made = Vault::init(&m.dirs, &spec, &passphrase()).unwrap();

    let file = vaultconfig::load(&spec.path.join(".gage/config.toml")).unwrap();
    assert_eq!(file.vault.id, made.vault.id);
    let expected = paths::identity_file_path(&m.dirs, &made.vault.id, DEVICE).unwrap();
    assert!(expected.starts_with(m.dirs.data.join("identities").join(made.vault.id.as_str())));
    assert_eq!(m.identity_files(), vec![expected]);
}

#[test]
fn init_refuses_a_registered_name_before_asking_anything() {
    let m = Machine::new();
    m.preregister(NAME);
    let prompter = FakePrompter::new();

    let err = Vault::init(&m.dirs, &m.spec(NAME), &prompter).unwrap_err();

    assert!(
        matches!(&err, InitError::Registry(RegistryError::AlreadyRegistered(n)) if n == NAME),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(
        err.to_string(),
        format!("{NAME:?} is already a registered vault")
    );
    assert!(prompter.unlock_requests().is_empty());
    assert!(m.identity_files().is_empty());
    assert!(!m.path(NAME).exists());
}

#[test]
fn init_refuses_an_invalid_spec_before_asking_anything() {
    let m = Machine::new();
    let prompter = FakePrompter::new();
    type Change = Box<dyn Fn(&mut InitSpec)>;
    let cases: Vec<(&str, Change)> = vec![
        (
            "method",
            Box::new(|s| s.method = UnlockMethod::from("yubikey")),
        ),
        ("device", Box::new(|s| s.device = "../escape".into())),
        (
            "reserved device",
            Box::new(|s| s.device = RECOVERY_DEVICE_LABEL.into()),
        ),
        (
            "recipient",
            Box::new(|s| s.recipients = vec!["not-a-key".into()]),
        ),
        (
            "recovery key",
            Box::new(|s| s.recovery_key = Some("not-a-key".into())),
        ),
        ("empty path", Box::new(|s| s.path = PathBuf::new())),
    ];
    for (what, change) in cases {
        let mut spec = m.spec(NAME);
        change(&mut spec);
        let err = Vault::init(&m.dirs, &spec, &prompter).unwrap_err();
        assert!(matches!(err, InitError::Create(_)), "{what}: {err:?}");
        assert_eq!(err.kind(), ErrorKind::Usage, "{what}: {err}");
    }
    assert!(prompter.unlock_requests().is_empty());
    assert!(m.identity_files().is_empty());
    assert!(!paths::global_config_file(&m.dirs).exists());
}

#[test]
fn a_failed_create_removes_the_identity_and_registers_nothing() {
    let m = Machine::new();
    let spec = m.spec(NAME);
    std::fs::create_dir_all(&spec.path).unwrap();
    std::fs::write(spec.path.join("keep.txt"), "mine").unwrap();
    let prompter = passphrase();

    let err = Vault::init(&m.dirs, &spec, &prompter).unwrap_err();

    assert!(
        matches!(&err, InitError::Create(e) if matches!(**e, CreateError::TargetNotEmpty(_))),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    // It got as far as the identity, and took it back.
    assert_eq!(prompter.unlock_requests().len(), 1);
    assert!(m.identity_files().is_empty());
    assert!(
        !m.dirs.data.join("identities").exists() || {
            std::fs::read_dir(m.dirs.data.join("identities"))
                .unwrap()
                .next()
                .is_none()
        }
    );
    assert!(m.global().vaults.is_empty());
    assert_eq!(
        std::fs::read_to_string(spec.path.join("keep.txt")).unwrap(),
        "mine"
    );
}

#[test]
fn a_cancelled_passphrase_creates_nothing() {
    let m = Machine::new();
    let spec = m.spec(NAME);
    let prompter = FakePrompter::new().with_unlock_error(PromptError::Cancelled);

    let err = Vault::init(&m.dirs, &spec, &prompter).unwrap_err();

    assert!(
        matches!(&err, InitError::Identity(e) if matches!(**e, UnlockError::Prompter(_))),
        "{err:?}"
    );
    assert!(m.identity_files().is_empty());
    assert!(!spec.path.exists());
    assert!(!paths::global_config_file(&m.dirs).exists());
}

#[test]
fn the_recovery_key_follows_the_device_key_in_both_recipient_files() {
    let m = Machine::new();
    let mut spec = m.spec(NAME);
    let recovery = new_pubkey();
    let other = new_pubkey();
    spec.recovery_key = Some(recovery.clone());
    spec.recipients = vec![other.clone()];

    let made = Vault::init(&m.dirs, &spec, &passphrase()).unwrap();

    let file = vaultconfig::load(&spec.path.join(".gage/config.toml")).unwrap();
    let listed: Vec<(&str, &str)> = file
        .recipients
        .iter()
        .map(|r| (r.device.as_str(), r.pubkey.as_str()))
        .collect();
    assert_eq!(
        listed,
        vec![
            (DEVICE, made.pubkey.as_str()),
            (RECOVERY_DEVICE_LABEL, recovery.as_str()),
            ("recipient-2", other.as_str()),
        ]
    );
    assert_eq!(
        recipient::load(&spec.path.join(".age-recipients")).unwrap(),
        vec![made.pubkey.clone(), recovery, other]
    );
}

#[test]
fn the_identity_hook_runs_once_with_the_vaults_id() {
    let m = Machine::new();
    let seen: Rc<Cell<Option<VaultId>>> = Rc::default();
    let calls = Rc::new(Cell::new(0));
    let _hook = {
        let (seen, calls) = (seen.clone(), calls.clone());
        hooks::after_identity_created(move |id| {
            calls.set(calls.get() + 1);
            seen.set(Some(id.clone()));
        })
    };

    let made = Vault::init(&m.dirs, &m.spec(NAME), &passphrase()).unwrap();

    assert_eq!(calls.get(), 1);
    assert_eq!(seen.take(), Some(made.vault.id));
}

#[cfg(unix)]
#[test]
fn a_failed_rollback_is_reported_beside_the_failure() {
    let m = Machine::new();
    let spec = m.spec(NAME);
    std::fs::create_dir_all(&spec.path).unwrap();
    std::fs::write(spec.path.join("keep.txt"), "mine").unwrap();
    let guard: Rc<Cell<Option<Restore>>> = Rc::default();
    let skipped = Rc::new(Cell::new(false));
    let _hook = {
        let (dirs, guard, skipped) = (m.dirs.clone(), guard.clone(), skipped.clone());
        hooks::after_identity_created(move |id| {
            match read_only(&paths::identities_dir(&dirs, id)) {
                Some(g) => guard.set(Some(g)),
                None => skipped.set(true),
            }
        })
    };

    let err = Vault::init(&m.dirs, &spec, &passphrase()).unwrap_err();
    if skipped.get() {
        return;
    }

    let InitError::RollbackFailed { cause, rollback } = &err else {
        panic!("{err:?}");
    };
    assert!(
        matches!(**cause, CreateError::TargetNotEmpty(_)),
        "{cause:?}"
    );
    assert!(matches!(**rollback, UnlockError::Io { .. }), "{rollback:?}");
    assert_eq!(err.kind(), ErrorKind::Internal);
    assert_eq!(
        err.to_string(),
        format!("{cause} (and rolling back the generated identity failed: {rollback})")
    );
    // The identity it couldn't remove is still there, and nothing points
    // at it: exactly what the message warns of.
    drop(guard.take());
    assert_eq!(m.identity_files().len(), 1);
    assert!(m.global().vaults.is_empty());
}

#[cfg(unix)]
#[test]
fn a_failed_registry_save_removes_the_vault_and_identity() {
    let m = Machine::new();
    let spec = m.spec(NAME);
    std::fs::create_dir_all(&m.dirs.config).unwrap();
    let Some(ro) = read_only(&m.dirs.config) else {
        return;
    };

    let err = Vault::init(&m.dirs, &spec, &passphrase()).unwrap_err();

    assert!(
        matches!(&err, InitError::SaveRegistry(e) if matches!(**e, config::Error::Write { .. })),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Internal);
    assert!(
        !spec.path.exists(),
        "the unregistered vault was left behind"
    );
    assert!(m.identity_files().is_empty());
    assert!(!paths::global_config_file(&m.dirs).exists());
    assert!(m.trust_caches().is_empty(), "{:?}", m.trust_caches());

    // Nothing is left in a retry's way.
    drop(ro);
    let made = Vault::init(&m.dirs, &spec, &passphrase()).unwrap();
    assert_eq!(
        m.identity_files(),
        vec![paths::identity_file_path(&m.dirs, &made.vault.id, DEVICE).unwrap()]
    );
    assert!(m.global().vaults.contains_key(NAME));
}

#[cfg(unix)]
#[test]
fn a_failed_registry_save_leaves_an_empty_target_directory_empty() {
    let m = Machine::new();
    let spec = m.spec(NAME);
    std::fs::create_dir(&spec.path).unwrap();
    std::fs::create_dir_all(&m.dirs.config).unwrap();
    let Some(_ro) = read_only(&m.dirs.config) else {
        return;
    };

    Vault::init(&m.dirs, &spec, &passphrase()).unwrap_err();

    assert!(spec.path.is_dir(), "the caller's own directory was removed");
    assert_eq!(std::fs::read_dir(&spec.path).unwrap().count(), 0);
}

// The id is minted by the call, so it is what tells this call's vault from
// anything else that has come to be at the path.
#[test]
fn a_target_that_no_longer_carries_the_minted_id_is_left_alone() {
    let m = Rc::new(Machine::new());
    let spec = m.spec(NAME);
    let racer = Rc::clone(&m);
    let config_file = spec.path.join(".gage/config.toml");
    let replaced = config_file.clone();
    let _hook = hooks::before_registering(move || {
        let id = vaultconfig::load(&replaced).unwrap().vault.id;
        let raw = std::fs::read_to_string(&replaced).unwrap();
        std::fs::write(&replaced, raw.replace(id.as_str(), VaultId::new().as_str())).unwrap();
        racer.preregister(NAME);
    });

    let err = Vault::init(&m.dirs, &spec, &passphrase()).unwrap_err();

    assert!(matches!(err, InitError::Registry(_)), "{err:?}");
    assert!(config_file.exists(), "another vault's files were deleted");
    assert!(m.identity_files().is_empty());
}

#[cfg(unix)]
#[test]
fn a_vault_that_cant_be_removed_is_reported_and_keeps_its_identity() {
    let m = Rc::new(Machine::new());
    let spec = m.spec(NAME);
    let guard: Rc<Cell<Option<Restore>>> = Rc::default();
    let skipped = Rc::new(Cell::new(false));
    let _hook = {
        let (racer, gage_dir, guard, skipped) = (
            Rc::clone(&m),
            spec.path.join(".gage"),
            guard.clone(),
            skipped.clone(),
        );
        hooks::before_registering(move || {
            racer.preregister(NAME);
            match read_only(&gage_dir) {
                Some(g) => guard.set(Some(g)),
                None => skipped.set(true),
            }
        })
    };

    let err = Vault::init(&m.dirs, &spec, &passphrase()).unwrap_err();
    if skipped.get() {
        return;
    }

    let InitError::RegistrationRollbackFailed { cause, rollback } = &err else {
        panic!("{err:?}");
    };
    assert!(matches!(**cause, InitError::Registry(_)), "{cause:?}");
    assert!(
        matches!(&**rollback, InitRollbackError::Vault { path, .. } if *path == spec.path),
        "{rollback:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Internal);
    assert_eq!(
        err.to_string(),
        format!("{cause} (and undoing the new vault failed: {rollback})")
    );
    // The identity goes only once the vault it opens has gone.
    drop(guard.take());
    assert_eq!(m.identity_files().len(), 1);
    assert!(spec.path.join(".gage").exists());
}

#[cfg(unix)]
#[test]
fn an_identity_that_cant_be_removed_after_a_failed_registration_is_reported() {
    let m = Rc::new(Machine::new());
    let spec = m.spec(NAME);
    let guard: Rc<Cell<Option<Restore>>> = Rc::default();
    let skipped = Rc::new(Cell::new(false));
    let _identity_hook = {
        let (dirs, guard, skipped) = (m.dirs.clone(), guard.clone(), skipped.clone());
        hooks::after_identity_created(move |id| {
            match read_only(&paths::identities_dir(&dirs, id)) {
                Some(g) => guard.set(Some(g)),
                None => skipped.set(true),
            }
        })
    };
    let racer = Rc::clone(&m);
    let _name_hook = hooks::before_registering(move || racer.preregister(NAME));

    let err = Vault::init(&m.dirs, &spec, &passphrase()).unwrap_err();
    if skipped.get() {
        return;
    }

    let InitError::RegistrationRollbackFailed { rollback, .. } = &err else {
        panic!("{err:?}");
    };
    assert!(
        matches!(**rollback, InitRollbackError::Identity(_)),
        "{rollback:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Internal);
    assert!(!spec.path.exists());
    drop(guard.take());
    assert_eq!(m.identity_files().len(), 1);
}

// The passphrase prompts sit between the first read of the registry and
// the registration: a vault registered in that time must not be lost.
#[test]
fn a_registration_made_while_init_ran_survives_it() {
    let m = Rc::new(Machine::new());
    let racer = Rc::clone(&m);
    let _hook = hooks::before_registering(move || racer.preregister("other"));

    Vault::init(&m.dirs, &m.spec(NAME), &passphrase()).unwrap();

    let global = m.global();
    assert!(global.vaults.contains_key("other"), "{global:?}");
    assert!(global.vaults.contains_key(NAME), "{global:?}");
}

#[test]
fn a_name_taken_while_init_ran_is_refused_and_the_winner_kept() {
    let m = Rc::new(Machine::new());
    let spec = m.spec(NAME);
    let racer = Rc::clone(&m);
    let _hook = hooks::before_registering(move || racer.preregister(NAME));

    let err = Vault::init(&m.dirs, &spec, &passphrase()).unwrap_err();

    assert!(
        matches!(&err, InitError::Registry(RegistryError::AlreadyRegistered(n)) if n == NAME),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(m.global().vaults[NAME].path, m.path("elsewhere"));
    // Undone, like any other failure to register.
    assert!(!spec.path.exists());
    assert!(m.identity_files().is_empty());
}

// --- Vault::clone_remote ------------------------------------------------------

/// A vault `init` created on its own machine and published to a bare
/// remote. The machine is returned so its files outlive the test body.
fn published() -> (Machine, BareRemote, VaultId) {
    let first = Machine::new();
    let remote = gittest::new_bare_remote();
    let mut spec = first.spec(NAME);
    spec.remote = Some(remote.path().to_str().unwrap().into());
    let made = Vault::init(&first.dirs, &spec, &passphrase()).unwrap();
    made.vault.push().unwrap();
    (first, remote, made.vault.id)
}

fn clone_spec(m: &Machine, remote: &BareRemote) -> CloneSpec {
    CloneSpec {
        name: NAME.into(),
        path: m.path("cloned"),
        url: remote.path().to_str().unwrap().into(),
        device: "laptop-2".into(),
    }
}

#[test]
fn clone_registers_the_vaults_id_default_method_and_origin() {
    let (_first, remote, id) = published();
    let m = Machine::new();
    let spec = clone_spec(&m, &remote);

    let cloned = Vault::clone_remote(&m.dirs, &spec).unwrap();

    assert_eq!(cloned.vault.id, id);
    assert_eq!(cloned.vault.name, NAME);
    assert_eq!(cloned.vault.path, spec.path);
    let global = m.global();
    let entry = global.entry(NAME).unwrap();
    assert_eq!(entry.path, spec.path);
    assert_eq!(entry.id.as_ref(), Some(&id));
    assert_eq!(entry.vault_type, VaultType::Git);
    assert_eq!(entry.device, "laptop-2");
    assert_eq!(entry.pubkey, None);
    assert_eq!(entry.method, UnlockMethod::Passphrase);
    assert_eq!(entry.git.origin.as_deref(), Some(spec.url.as_str()));
    assert!(spec.path.join(".age-recipients").exists());
    Vault::from_registration(&m.dirs, NAME, entry).unwrap();
}

#[test]
fn cloned_reports_whether_this_device_holds_an_identity() {
    let (_first, remote, id) = published();

    let m = Machine::new();
    let cloned = Vault::clone_remote(&m.dirs, &clone_spec(&m, &remote)).unwrap();
    assert!(!cloned.holds_identity);

    let m = Machine::new();
    identityfile::create(&m.dirs, &id, NAME, "laptop-2", &passphrase()).unwrap();
    let cloned = Vault::clone_remote(&m.dirs, &clone_spec(&m, &remote)).unwrap();
    assert!(cloned.holds_identity);
}

#[test]
fn a_failed_clone_into_an_absent_path_leaves_no_directory() {
    let m = Machine::new();
    let mut spec = clone_spec(&m, &gittest::new_bare_remote());
    spec.url = m.path("no-such-remote.git").to_str().unwrap().into();

    let err = Vault::clone_remote(&m.dirs, &spec).unwrap_err();

    assert!(matches!(err, CloneError::Git(_)), "{err:?}");
    assert!(!spec.path.exists());
    assert!(!paths::global_config_file(&m.dirs).exists());
}

#[test]
fn a_failed_clone_into_an_empty_directory_leaves_it_empty() {
    let m = Machine::new();
    let mut spec = clone_spec(&m, &gittest::new_bare_remote());
    spec.url = m.path("no-such-remote.git").to_str().unwrap().into();
    std::fs::create_dir(&spec.path).unwrap();

    let err = Vault::clone_remote(&m.dirs, &spec).unwrap_err();

    assert!(matches!(err, CloneError::Git(_)), "{err:?}");
    assert!(spec.path.is_dir(), "the caller's own directory was removed");
    assert_eq!(std::fs::read_dir(&spec.path).unwrap().count(), 0);
}

#[test]
fn a_non_empty_target_is_refused_and_left_untouched() {
    let (_first, remote, _) = published();
    let m = Machine::new();
    let spec = clone_spec(&m, &remote);
    std::fs::create_dir(&spec.path).unwrap();
    std::fs::write(spec.path.join("keep.txt"), "mine").unwrap();

    let err = Vault::clone_remote(&m.dirs, &spec).unwrap_err();

    assert!(matches!(err, CloneError::Git(_)), "{err:?}");
    assert_eq!(
        std::fs::read_to_string(spec.path.join("keep.txt")).unwrap(),
        "mine"
    );
    assert_eq!(std::fs::read_dir(&spec.path).unwrap().count(), 1);
    assert!(!paths::global_config_file(&m.dirs).exists());
}

#[test]
fn an_unknown_format_version_rolls_the_clone_back() {
    let (_first, remote, _) = published();
    let other = gittest::Device::clone(remote.path());
    let raw = other.read(".gage/config.toml");
    let current = format!("format_version = {}", vaultconfig::CURRENT_FORMAT_VERSION);
    assert!(raw.contains(&current), "{raw}");
    other.write_commit_push(
        ".gage/config.toml",
        &raw.replace(&current, "format_version = 99"),
        "a newer gage wrote this vault",
    );
    let m = Machine::new();
    let spec = clone_spec(&m, &remote);

    let err = Vault::clone_remote(&m.dirs, &spec).unwrap_err();

    assert!(
        matches!(
            err,
            CloneError::Config(vaultconfig::Error::UnsupportedFormatVersion { .. })
        ),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(
        !spec.path.exists(),
        "the refused clone left its directory behind"
    );
    assert!(!paths::global_config_file(&m.dirs).exists());
}

#[test]
fn clone_refuses_a_registered_name_before_cloning() {
    let (_first, remote, _) = published();
    let m = Machine::new();
    m.preregister(NAME);
    let spec = clone_spec(&m, &remote);

    let err = Vault::clone_remote(&m.dirs, &spec).unwrap_err();

    assert!(
        matches!(&err, CloneError::Registry(RegistryError::AlreadyRegistered(n)) if n == NAME),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    // The library's message names no CLI flag; the frontend adds its hint.
    assert_eq!(
        err.to_string(),
        format!("{NAME:?} is already a registered vault")
    );
    assert!(!spec.path.exists());
}

#[test]
fn clone_refuses_a_bad_name_or_device_before_cloning() {
    let m = Machine::new();
    let remote = gittest::new_bare_remote();

    let mut spec = clone_spec(&m, &remote);
    spec.name = "../escape".into();
    let err = Vault::clone_remote(&m.dirs, &spec).unwrap_err();
    assert!(
        matches!(err, CloneError::Name(NameError::NotOnePathComponent(_))),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Usage);

    let mut spec = clone_spec(&m, &remote);
    spec.device = "../escape".into();
    let err = Vault::clone_remote(&m.dirs, &spec).unwrap_err();
    assert!(matches!(err, CloneError::InvalidDeviceName(_)), "{err:?}");
    assert_eq!(err.kind(), ErrorKind::Usage);
    assert!(!spec.path.exists());
}

#[cfg(unix)]
#[test]
fn a_failed_registry_save_removes_the_clone() {
    let (_first, remote, id) = published();
    let m = Machine::new();
    let spec = clone_spec(&m, &remote);
    std::fs::create_dir_all(&m.dirs.config).unwrap();
    let Some(ro) = read_only(&m.dirs.config) else {
        return;
    };

    let err = Vault::clone_remote(&m.dirs, &spec).unwrap_err();

    assert!(
        matches!(err, CloneError::SaveRegistry(config::Error::Write { .. })),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Internal);
    assert!(
        !spec.path.exists(),
        "the unregistered clone was left behind"
    );
    assert!(!paths::global_config_file(&m.dirs).exists());

    // Nothing is left in a retry's way.
    drop(ro);
    let cloned = Vault::clone_remote(&m.dirs, &spec).unwrap();
    assert_eq!(cloned.vault.id, id);
}

#[cfg(unix)]
#[test]
fn a_clone_that_cant_be_removed_after_a_failed_registration_is_reported() {
    let (_first, remote, _id) = published();
    let m = Rc::new(Machine::new());
    let spec = clone_spec(&m, &remote);
    let guard: Rc<Cell<Option<Restore>>> = Rc::default();
    let skipped = Rc::new(Cell::new(false));
    let _hook = {
        let (racer, taken, gage_dir, guard, skipped) = (
            Rc::clone(&m),
            spec.name.clone(),
            spec.path.join(".gage"),
            guard.clone(),
            skipped.clone(),
        );
        hooks::before_registering(move || {
            racer.preregister(&taken);
            match read_only(&gage_dir) {
                Some(g) => guard.set(Some(g)),
                None => skipped.set(true),
            }
        })
    };

    let err = Vault::clone_remote(&m.dirs, &spec).unwrap_err();
    if skipped.get() {
        return;
    }

    let CloneError::RollbackFailed { cause, path, .. } = &err else {
        panic!("{err:?}");
    };
    assert!(matches!(**cause, CloneError::Registry(_)), "{cause:?}");
    assert_eq!(*path, spec.path);
    assert_eq!(err.kind(), ErrorKind::Internal);
    drop(guard.take());
}

// The network clone sits between the first read of the registry and the
// registration: a vault registered in that time must not be lost.
#[test]
fn a_registration_made_while_clone_ran_survives_it() {
    let (_first, remote, _id) = published();
    let m = Rc::new(Machine::new());
    let spec = clone_spec(&m, &remote);
    let racer = Rc::clone(&m);
    let _hook = hooks::before_registering(move || racer.preregister("other"));

    Vault::clone_remote(&m.dirs, &spec).unwrap();

    let global = m.global();
    assert!(global.vaults.contains_key("other"), "{global:?}");
    assert!(global.vaults.contains_key(&spec.name), "{global:?}");
}

#[test]
fn a_name_taken_while_clone_ran_is_refused_and_the_winner_kept() {
    let (_first, remote, _id) = published();
    let m = Rc::new(Machine::new());
    let spec = clone_spec(&m, &remote);
    let racer = Rc::clone(&m);
    let taken = spec.name.clone();
    let _hook = hooks::before_registering(move || racer.preregister(&taken));

    let err = Vault::clone_remote(&m.dirs, &spec).unwrap_err();

    assert!(
        matches!(&err, CloneError::Registry(RegistryError::AlreadyRegistered(n)) if *n == spec.name),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(m.global().vaults[&spec.name].path, m.path("elsewhere"));
    // Undone, like any other failure to register.
    assert!(!spec.path.exists());
}

#[test]
fn a_failed_clone_rollback_reads_as_both_failures() {
    let path = PathBuf::from("/vaults/personal");
    let cause = CloneError::Config(vaultconfig::Error::UnsupportedFormatVersion {
        path: path.join(".gage/config.toml"),
        found: 99,
        skew: vaultconfig::VersionSkew::Newer,
    });
    let cause_text = cause.to_string();
    let err = CloneError::RollbackFailed {
        cause: Box::new(cause),
        path: path.clone(),
        rollback: std::io::Error::other("permission denied"),
    };
    assert_eq!(err.kind(), ErrorKind::Internal);
    assert_eq!(
        err.to_string(),
        format!(
            "{cause_text} (and removing the partial clone at {} failed: permission denied)",
            path.display()
        )
    );
}

// Clippy's `result_large_err` limit is 128 bytes, and only Windows CI
// crossed it (its `PathBuf` is 8 bytes larger), so it went unnoticed on
// Linux and macOS. `Vault::init` returns `InitError`, so it stays small
// here on every platform, with room to spare.
#[test]
fn init_error_stays_small_enough_to_carry() {
    use std::mem::size_of;
    assert!(size_of::<InitError>() <= 96, "{}", size_of::<InitError>());
}
