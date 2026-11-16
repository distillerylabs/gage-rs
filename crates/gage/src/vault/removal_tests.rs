//! `Vault::remove_from_device`: what it removes, what it keeps, and what it
//! refuses to lose (ADR-0082).

use std::path::PathBuf;

use crate::error::{ErrorKind, HasKind, Remedy};
use crate::gittest::{self, BareRemote};
use crate::prompter::FakePrompter;
use crate::storage::config::{self, Global, RegistryError};
use crate::storage::paths;
use crate::storage::vaultconfig::{UnlockMethod, VaultId, VaultType};
use crate::storage::xdgpaths::Dirs;
use crate::sync::{gitrepo, remoteauth};
use crate::testsupport::fast_scrypt;
use crate::vault::{
    CloneRemoval, InitSpec, KeptClone, LocalOnly, RemoveError, RemoveOptions, Removed, Vault,
};

const NAME: &str = "personal";
const DEVICE: &str = "phone";

struct Machine {
    _root: tempfile::TempDir,
    dirs: Dirs,
    _remote: Option<BareRemote>,
}

impl Machine {
    fn global(&self) -> Global {
        config::load_or_default(&paths::global_config_file(&self.dirs)).unwrap()
    }

    fn vault(&self) -> Vault {
        let global = self.global();
        Vault::from_registration(&self.dirs, NAME, global.entry(NAME).unwrap()).unwrap()
    }

    fn path(&self) -> PathBuf {
        paths::vaults_dir(&self.dirs).join(NAME)
    }

    fn identity(&self) -> PathBuf {
        paths::identity_file_path(&self.dirs, &self.vault().id, DEVICE).unwrap()
    }

    fn remove(&self, options: RemoveOptions) -> Result<Removed, RemoveError> {
        Vault::remove_from_device(&self.dirs, NAME, &options)
    }

    fn registered(&self) -> bool {
        self.global().vaults.contains_key(NAME)
    }
}

struct Setup {
    remote: bool,
    push: bool,
    /// Another recipient beside this device's key.
    other_recipient: bool,
}

const PUBLISHED: Setup = Setup {
    remote: true,
    push: true,
    other_recipient: true,
};

/// A vault `init` made in gage's own vaults directory, as the bindings do.
fn machine(setup: Setup) -> Machine {
    fast_scrypt();
    let root = tempfile::tempdir().unwrap();
    let dirs = Dirs {
        config: root.path().join("config"),
        data: root.path().join("data"),
        state: root.path().join("state"),
    };
    let remote = setup.remote.then(gittest::new_bare_remote);
    let spec = InitSpec {
        name: NAME.into(),
        path: paths::vaults_dir(&dirs).join(NAME),
        vault_type: VaultType::Git,
        method: UnlockMethod::Passphrase,
        device: DEVICE.into(),
        recipients: if setup.other_recipient {
            vec![age::x25519::Identity::generate().to_public().to_string()]
        } else {
            Vec::new()
        },
        recovery_key: None,
        remote: remote
            .as_ref()
            .map(|r| r.path().to_str().unwrap().to_string()),
    };
    let made = Vault::init(&dirs, &spec, &FakePrompter::new().with_passphrase("pw")).unwrap();
    if setup.push {
        made.vault.push().unwrap();
    }
    Machine {
        _root: root,
        dirs,
        _remote: remote,
    }
}

fn delete_identity() -> RemoveOptions {
    RemoveOptions {
        delete_identity: true,
        ..Default::default()
    }
}

fn refused(err: RemoveError) -> LocalOnly {
    match err {
        RemoveError::LocalOnly { what, .. } => what,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn removes_the_clone_trust_cache_and_registration_and_keeps_the_identity() {
    let m = machine(PUBLISHED);
    let id = m.vault().id;
    let identity = m.identity();
    // The first unlock records what this device trusts.
    drop(
        m.vault()
            .unlock(&FakePrompter::new().with_passphrase("pw"))
            .unwrap(),
    );
    let known = paths::trust_cache_dir(&m.dirs, &id).join("known-config.toml");
    assert!(known.exists(), "the unlock recorded a trust record");

    let removed = m.remove(RemoveOptions::default()).unwrap();

    assert_eq!(removed.clone, CloneRemoval::Removed);
    assert!(!removed.identity_removed);
    assert_eq!(removed.still_a_recipient, Some(true));
    assert!(!m.path().exists());
    assert!(!m.registered());
    assert_eq!(m.global().current, None, "current named it, and is cleared");
    assert!(!known.exists(), "the trust cache went");
    assert!(identity.is_file(), "kept unless asked for");
}

#[test]
fn deleting_the_identity_says_this_device_is_still_a_recipient() {
    let m = machine(PUBLISHED);
    let identity = m.identity();

    let removed = m.remove(delete_identity()).unwrap();

    assert!(removed.identity_removed);
    assert_eq!(removed.still_a_recipient, Some(true));
    assert!(!identity.exists());
}

#[test]
fn a_vault_with_no_remote_is_refused_unless_discarded() {
    let m = machine(Setup {
        remote: false,
        push: false,
        other_recipient: true,
    });

    let err = m.remove(RemoveOptions::default()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(err.remedy(), Some(Remedy::SetRemote));
    assert_eq!(refused(err), LocalOnly::NoRemote);
    assert!(m.path().exists() && m.registered(), "nothing was removed");

    m.remove(RemoveOptions {
        discard_local_only: true,
        ..Default::default()
    })
    .unwrap();
    assert!(!m.path().exists() && !m.registered());
}

#[test]
fn a_vault_never_pushed_is_refused() {
    let m = machine(Setup {
        remote: true,
        push: false,
        other_recipient: true,
    });
    let err = m.remove(RemoveOptions::default()).unwrap_err();
    assert_eq!(err.remedy(), Some(Remedy::Push));
    assert_eq!(refused(err), LocalOnly::NeverPushed);
    assert!(m.registered());
}

#[test]
fn unpushed_commits_are_refused_and_counted() {
    let m = machine(PUBLISHED);
    for n in 0..2 {
        std::fs::write(m.path().join(format!("note-{n}")), "x").unwrap();
        gitrepo::commit_all(&m.path(), "local").unwrap();
    }
    let err = m.remove(RemoveOptions::default()).unwrap_err();
    assert!(
        err.to_string()
            .contains("2 commits its remote doesn't have"),
        "{err}"
    );
    assert_eq!(err.remedy(), Some(Remedy::Push));
    assert_eq!(refused(err), LocalOnly::Unpushed { commits: 2 });
    assert!(m.path().exists() && m.registered());
}

#[test]
fn uncommitted_changes_are_refused() {
    let m = machine(PUBLISHED);
    std::fs::write(m.path().join("stray"), "x").unwrap();
    let err = m.remove(RemoveOptions::default()).unwrap_err();
    assert_eq!(err.remedy(), None);
    assert_eq!(refused(err), LocalOnly::Uncommitted);
}

#[test]
fn deleting_the_only_key_is_refused_unless_discarded() {
    let m = machine(Setup {
        remote: true,
        push: true,
        other_recipient: false,
    });

    // Keeping the identity loses nothing.
    let err = m.remove(delete_identity()).unwrap_err();
    assert_eq!(refused(err), LocalOnly::LastKey);
    assert!(m.identity().is_file() && m.path().exists() && m.registered());

    let removed = m
        .remove(RemoveOptions {
            delete_identity: true,
            discard_local_only: true,
        })
        .unwrap();
    assert!(removed.identity_removed);
}

#[test]
fn the_only_key_kept_is_not_refused() {
    let m = machine(Setup {
        remote: true,
        push: true,
        other_recipient: false,
    });
    m.remove(RemoveOptions::default()).unwrap();
    assert!(!m.registered());
}

#[test]
fn a_clone_outside_the_vaults_directory_is_kept() {
    let m = machine(PUBLISHED);
    let elsewhere = m._root.path().join("chosen");
    std::fs::rename(m.path(), &elsewhere).unwrap();
    config::update(&m.dirs, |g| -> Result<(), config::Error> {
        g.vaults.get_mut(NAME).unwrap().path = elsewhere.clone();
        Ok(())
    })
    .unwrap();
    std::fs::write(elsewhere.join("stray"), "x").unwrap();

    let removed = m.remove(delete_identity()).unwrap();

    assert_eq!(
        removed.clone,
        CloneRemoval::Kept(KeptClone::OutsideVaultsDir)
    );
    assert!(
        elsewhere.join("stray").exists(),
        "not looked into, not touched"
    );
    assert!(!m.registered());
}

#[test]
fn a_clone_another_registration_shares_is_kept() {
    let m = machine(PUBLISHED);
    let entry = m.global().entry(NAME).unwrap().clone();
    config::update(&m.dirs, |g| -> Result<(), config::Error> {
        g.vaults.insert("alias".into(), entry.clone());
        Ok(())
    })
    .unwrap();

    let removed = m.remove(RemoveOptions::default()).unwrap();

    assert_eq!(
        removed.clone,
        CloneRemoval::Kept(KeptClone::SharedWith("alias".into()))
    );
    assert!(m.path().exists());
    assert!(m.global().vaults.contains_key("alias"));
}

#[test]
fn a_missing_clone_still_removes_the_registration() {
    let m = machine(PUBLISHED);
    std::fs::remove_dir_all(m.path()).unwrap();

    let removed = m.remove(RemoveOptions::default()).unwrap();

    assert_eq!(removed.clone, CloneRemoval::Absent);
    assert_eq!(
        removed.still_a_recipient, None,
        "nothing left to read it from"
    );
    assert!(!m.registered());
}

#[test]
fn a_clone_gage_cant_read_is_removed_without_asking() {
    let m = machine(PUBLISHED);
    std::fs::remove_dir_all(m.path().join(".git")).unwrap();
    std::fs::write(m.path().join("stray"), "x").unwrap();

    let removed = m.remove(RemoveOptions::default()).unwrap();

    assert_eq!(removed.clone, CloneRemoval::Removed);
    assert!(!m.path().exists());
}

#[test]
fn a_clone_of_another_vault_tells_nothing_about_recipients() {
    // The ReRegister case: the registration's id isn't the clone's.
    let m = machine(PUBLISHED);
    config::update(&m.dirs, |g| -> Result<(), config::Error> {
        g.vaults.get_mut(NAME).unwrap().id = Some(VaultId::new());
        Ok(())
    })
    .unwrap();

    let removed = m.remove(RemoveOptions::default()).unwrap();

    assert_eq!(removed.still_a_recipient, None);
    assert_eq!(removed.clone, CloneRemoval::Removed);
    assert!(!m.registered());
}

#[test]
fn an_unregistered_name_is_not_found() {
    let m = machine(PUBLISHED);
    let err = Vault::remove_from_device(&m.dirs, "nope", &RemoveOptions::default()).unwrap_err();
    assert!(matches!(
        err,
        RemoveError::Registry(RegistryError::NotRegistered(_))
    ));
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[test]
fn the_saved_token_is_left_alone() {
    let m = machine(PUBLISHED);
    remoteauth::save(&m.dirs, "example.com", "secret-token").unwrap();
    m.remove(delete_identity()).unwrap();
    assert_eq!(remoteauth::hosts(&m.dirs).unwrap(), vec!["example.com"]);
}

#[test]
fn other_vaults_are_untouched() {
    let m = machine(PUBLISHED);
    let other = m._root.path().join("other");
    config::update(&m.dirs, |g| -> Result<(), config::Error> {
        g.vaults.insert(
            "other".into(),
            config::VaultEntry {
                path: other.clone(),
                ..Default::default()
            },
        );
        g.current = Some("other".into());
        Ok(())
    })
    .unwrap();

    m.remove(RemoveOptions::default()).unwrap();

    let global = m.global();
    assert!(global.vaults.contains_key("other"));
    assert_eq!(global.current.as_deref(), Some("other"));
}

/// A clone that can't be deleted keeps the registration, so calling again,
/// once whatever blocked it is gone, finishes the job.
#[cfg(unix)]
#[test]
fn a_clone_that_cant_be_deleted_keeps_the_registration_for_a_retry() {
    use std::os::unix::fs::PermissionsExt;

    let m = machine(PUBLISHED);
    let identity = m.identity();
    let entries = m.path().join("entries");
    std::fs::write(entries.join("blocker"), "x").unwrap();
    gitrepo::commit_all(&m.path(), "blocker").unwrap();
    m.vault().push().unwrap();
    std::fs::set_permissions(&entries, std::fs::Permissions::from_mode(0o500)).unwrap();
    if std::fs::remove_file(entries.join("blocker")).is_ok() {
        #[allow(clippy::disallowed_macros)]
        {
            eprintln!("skipping: running as root, which ignores directory permissions");
        }
        return;
    }

    let err = m.remove(delete_identity()).unwrap_err();
    assert!(matches!(err, RemoveError::Clone { .. }), "{err:?}");
    assert_eq!(err.kind(), ErrorKind::Internal);
    assert!(m.registered());
    assert!(identity.is_file(), "nothing after the clone was touched");

    std::fs::set_permissions(&entries, std::fs::Permissions::from_mode(0o700)).unwrap();
    let removed = m.remove(delete_identity()).unwrap();
    assert!(removed.identity_removed);
    assert!(!m.path().exists() && !m.registered());
}
