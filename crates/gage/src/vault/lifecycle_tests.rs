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
use crate::vault::{CloneError, CloneSpec, CreateError, InitError, InitSpec, NameError, Vault};

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
fn a_failed_registry_save_leaves_the_vault_and_identity_in_place() {
    let m = Machine::new();
    let spec = m.spec(NAME);
    std::fs::create_dir_all(&m.dirs.config).unwrap();
    let Some(_ro) = read_only(&m.dirs.config) else {
        return;
    };

    let err = Vault::init(&m.dirs, &spec, &passphrase()).unwrap_err();

    assert!(
        matches!(&err, InitError::SaveRegistry(e) if matches!(**e, config::Error::Write { .. })),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Internal);
    // As today (decision 6): not rolled back.
    let file = vaultconfig::load(&spec.path.join(".gage/config.toml")).unwrap();
    assert_eq!(
        m.identity_files(),
        vec![paths::identity_file_path(&m.dirs, &file.vault.id, DEVICE).unwrap()]
    );
    assert!(!paths::global_config_file(&m.dirs).exists());
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
fn a_failed_registry_save_leaves_the_clone_in_place_unregistered() {
    let (_first, remote, id) = published();
    let m = Machine::new();
    let spec = clone_spec(&m, &remote);
    std::fs::create_dir_all(&m.dirs.config).unwrap();
    let Some(_ro) = read_only(&m.dirs.config) else {
        return;
    };

    let err = Vault::clone_remote(&m.dirs, &spec).unwrap_err();

    assert!(
        matches!(err, CloneError::SaveRegistry(config::Error::Write { .. })),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Internal);
    // As today (decision 6): not rolled back.
    let file = vaultconfig::load(&spec.path.join(".gage/config.toml")).unwrap();
    assert_eq!(file.vault.id, id);
    assert!(!paths::global_config_file(&m.dirs).exists());
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
