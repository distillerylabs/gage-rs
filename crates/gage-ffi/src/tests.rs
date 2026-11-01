//! The bindings against real vaults, real identity files and real scrypt (at
//! the test work factor). The foreign side is played by Rust: a callback
//! interface is a trait, so a test implements it as Swift or Kotlin would.

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};

use gage::entry::NewEntry;
use gage::prompter::FakePrompter;
use gage::storage::config::{self, Global, VaultEntry};
use gage::storage::xdgpaths::Dirs;
use gage::storage::{identityfile, paths};
use gage::testsupport::fast_scrypt;
use gage::{CreateSpec, ErrorKind, HasKind, UnlockMethod, VaultId, VaultType};

use crate::{
    GageError, Identity, PromptError, Prompter, Roots, UnlockKind, UnlockPurpose, UnlockRequest,
    UnlockResponse, Vault, Warning,
};

const PASSPHRASE: &str = "correct horse battery staple";
const DEVICE: &str = "phone-1";
const VAULT: &str = "personal";

/// One simulated device holding one registered vault, with entries titled
/// `titles`.
struct Fixture {
    _root: tempfile::TempDir,
    dirs: Dirs,
}

impl Fixture {
    fn new(titles: &[&str]) -> Fixture {
        fast_scrypt();
        let root = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            config: root.path().join("config"),
            data: root.path().join("data"),
            state: root.path().join("state"),
        };
        let id = VaultId::new();
        let path = root.path().join(VAULT);
        let pubkey = identityfile::create(
            &dirs,
            &id,
            VAULT,
            DEVICE,
            &FakePrompter::new().with_passphrase(PASSPHRASE),
        )
        .unwrap();
        let vault = gage::Vault::create(
            &dirs,
            &CreateSpec {
                name: VAULT.into(),
                id: id.clone(),
                path: path.clone(),
                vault_type: VaultType::Git,
                method: UnlockMethod::Passphrase,
                device: DEVICE.into(),
                recipients: vec![pubkey],
                extra_recipients: Vec::new(),
                remote: None,
            },
        )
        .unwrap();
        let mut global = Global::default();
        global
            .register(
                VAULT,
                VaultEntry {
                    path,
                    id: Some(id),
                    vault_type: VaultType::Git,
                    device: DEVICE.into(),
                    method: UnlockMethod::Passphrase,
                    ..Default::default()
                },
            )
            .unwrap();
        config::save_creating_dir(&paths::global_config_file(&dirs), &global).unwrap();

        let ident = vault
            .unlock(&FakePrompter::new().with_passphrase(PASSPHRASE))
            .unwrap();
        for title in titles {
            vault
                .insert(
                    &ident,
                    NewEntry {
                        title: title.to_string(),
                        description: format!("about {title}"),
                        value: format!("v-{title}"),
                        ..NewEntry::default()
                    },
                    true,
                    &FakePrompter::new(),
                )
                .unwrap();
        }
        Fixture { _root: root, dirs }
    }

    fn roots(&self) -> Roots {
        let s = |p: &std::path::Path| p.to_str().unwrap().to_string();
        Roots {
            config: s(&self.dirs.config),
            data: s(&self.dirs.data),
            state: s(&self.dirs.state),
        }
    }

    fn open(&self) -> Arc<Vault> {
        Vault::open(self.roots(), VAULT.into()).unwrap()
    }
}

/// What a foreign prompter answers, in order.
enum Answer {
    Passphrase(&'static str),
    Fail(PromptError),
}

/// A scripted foreign prompter that records every request and warning.
#[derive(Default)]
struct ScriptedPrompter {
    answers: Mutex<VecDeque<Answer>>,
    requests: Mutex<Vec<UnlockRequest>>,
    warnings: Mutex<Vec<Warning>>,
}

impl ScriptedPrompter {
    fn answering(answers: Vec<Answer>) -> Arc<ScriptedPrompter> {
        Arc::new(ScriptedPrompter {
            answers: Mutex::new(answers.into()),
            ..Default::default()
        })
    }

    fn requests(&self) -> Vec<UnlockRequest> {
        self.requests.lock().unwrap().clone()
    }

    fn warnings(&self) -> Vec<Warning> {
        self.warnings.lock().unwrap().clone()
    }
}

impl Prompter for ScriptedPrompter {
    fn unlock(&self, request: UnlockRequest) -> Result<UnlockResponse, PromptError> {
        self.requests.lock().unwrap().push(request);
        match self.answers.lock().unwrap().pop_front() {
            Some(Answer::Passphrase(p)) => Ok(UnlockResponse::Passphrase {
                passphrase: p.into(),
            }),
            Some(Answer::Fail(e)) => Err(e),
            None => panic!("unscripted unlock request"),
        }
    }

    fn warn(&self, warning: Warning) {
        self.warnings.lock().unwrap().push(warning);
    }
}

/// A `Locker` that counts the locks still held.
#[derive(Default)]
struct CountingLocker {
    live: Mutex<i64>,
}

impl gage::crypto::locker::Locker for CountingLocker {
    fn lock(&self, _: &[u8]) -> io::Result<()> {
        *self.live.lock().unwrap() += 1;
        Ok(())
    }

    fn unlock(&self, _: &[u8]) -> io::Result<()> {
        *self.live.lock().unwrap() -= 1;
        Ok(())
    }
}

/// A `Locker` on a platform that refuses to page-lock.
struct RefusingLocker;

impl gage::crypto::locker::Locker for RefusingLocker {
    fn lock(&self, _: &[u8]) -> io::Result<()> {
        Err(io::Error::other("no mlock here"))
    }

    fn unlock(&self, _: &[u8]) -> io::Result<()> {
        panic!("a lock that never took must not be released");
    }
}

fn unlock(vault: &Vault) -> Arc<Identity> {
    vault
        .unlock(ScriptedPrompter::answering(vec![Answer::Passphrase(
            PASSPHRASE,
        )]))
        .unwrap()
}

#[test]
fn unlock_through_the_callback_then_list_returns_rows_sorted_by_title() {
    let f = Fixture::new(&["beta", "alpha"]);
    let vault = f.open();
    let identity = unlock(&vault);

    let rows = vault.list(identity).unwrap();

    let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(titles, ["alpha", "beta"]);
    assert_eq!(rows[0].description, "about alpha");
    assert_eq!(rows[0].updated_by, DEVICE);
    assert!(rows[0].created.is_some());
    for row in &rows {
        let groups: Vec<usize> = row.id.split('-').map(str::len).collect();
        assert_eq!(
            groups,
            [8, 4, 4, 4, 12],
            "{:?} is not a hyphenated UUID",
            row.id
        );
    }
}

#[test]
fn the_callback_sees_each_attempt_and_giving_up_after_a_wrong_one_is_locked_or_auth() {
    let f = Fixture::new(&[]);
    let prompter = ScriptedPrompter::answering(vec![
        Answer::Passphrase("wrong"),
        Answer::Fail(PromptError::Cancelled),
    ]);

    let err = f.open().unlock(prompter.clone()).unwrap_err();

    assert!(matches!(err, GageError::LockedOrAuth { .. }), "got {err:?}");
    let requests = prompter.requests();
    assert_eq!(
        requests.iter().map(|r| r.attempt).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(requests[0].vault, VAULT);
    assert_eq!(requests[0].device, DEVICE);
    assert_eq!(requests[0].kind, UnlockKind::Passphrase);
    assert_eq!(requests[0].purpose, UnlockPurpose::Unlock);
}

#[test]
fn too_many_attempts_from_the_callback_is_locked_or_auth() {
    let f = Fixture::new(&[]);
    let prompter = ScriptedPrompter::answering(vec![
        Answer::Passphrase("wrong"),
        Answer::Fail(PromptError::TooManyAttempts),
    ]);

    let err = f.open().unlock(prompter).unwrap_err();

    assert!(matches!(err, GageError::LockedOrAuth { .. }), "got {err:?}");
}

#[test]
fn a_failing_callback_keeps_its_message_and_the_library_kind() {
    let f = Fixture::new(&[]);
    let prompter = ScriptedPrompter::answering(vec![Answer::Fail(PromptError::Failed {
        message: "keychain unavailable".into(),
    })]);

    let err = f.open().unlock(prompter).unwrap_err();

    // The kind is the library's call for this failure, not the bindings'.
    let core = gage::vault::UnlockError::Prompter(gage::prompter::PromptError::Io(
        io::Error::other("keychain unavailable"),
    ));
    let expected = GageError::new(core.kind(), core.to_string());
    assert_eq!(format!("{err:?}"), format!("{expected:?}"));
    assert!(err.to_string().contains("keychain unavailable"), "{err}");
}

#[test]
fn an_unregistered_vault_is_not_found() {
    let f = Fixture::new(&[]);

    let err = Vault::open(f.roots(), "nope".into()).unwrap_err();

    assert!(matches!(err, GageError::NotFound { .. }), "got {err:?}");
}

#[test]
fn every_error_kind_has_its_own_variant() {
    let variants: std::collections::BTreeSet<String> = ErrorKind::ALL
        .iter()
        .map(|&kind| {
            let e = GageError::new(kind, "m".into());
            format!("{e:?}")
                .split_whitespace()
                .next()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(variants.len(), ErrorKind::ALL.len(), "{variants:?}");
}

#[test]
fn an_error_carries_the_library_message_unchanged() {
    let f = Fixture::new(&[]);
    let core = gage::storage::config::RegistryError::NotRegistered("nope".into());

    let err = Vault::open(f.roots(), "nope".into()).unwrap_err();

    assert_eq!(err.to_string(), core.to_string());
}

#[test]
fn dropping_the_last_identity_handle_releases_its_page_lock() {
    let f = Fixture::new(&[]);
    let locker = Arc::new(CountingLocker::default());
    let vault = Vault {
        inner: f.open().inner.clone().with_locker(locker.clone()),
    };

    let identity = unlock(&vault);
    let second_handle = identity.clone();
    assert_eq!(*locker.live.lock().unwrap(), 1);

    drop(identity);
    assert_eq!(*locker.live.lock().unwrap(), 1, "a handle is still held");

    drop(second_handle);
    assert_eq!(*locker.live.lock().unwrap(), 0);
}

#[test]
fn a_page_lock_failure_reaches_the_callback_as_one_warning() {
    let f = Fixture::new(&["alpha"]);
    let vault = Vault {
        inner: f.open().inner.clone().with_locker(Arc::new(RefusingLocker)),
    };
    let prompter = ScriptedPrompter::answering(vec![Answer::Passphrase(PASSPHRASE)]);

    let identity = vault.unlock(prompter.clone()).unwrap();

    let warnings = prompter.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(!warnings[0].text.is_empty());
    // Degraded, not failed: the identity still decrypts.
    assert_eq!(vault.list(identity).unwrap().len(), 1);
}
