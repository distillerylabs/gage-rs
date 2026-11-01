//! The bindings against real vaults, real identity files and real scrypt (at
//! the test work factor). The foreign side is played by Rust: a callback
//! interface is a trait, so a test implements it as Swift or Kotlin would.

use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use gage::enrollment::Approval;
use gage::entry::NewEntry;
use gage::gittest::{self, Answer as RecorderAnswer, BareRemote, HttpRecorder};
use gage::prompter::FakePrompter;
use gage::storage::config::{self, Global, VaultEntry};
use gage::storage::xdgpaths::Dirs;
use gage::storage::{identityfile, paths};
use gage::sync::remotesync::RemoteSyncer;
use gage::sync::{gitrepo, syncerr};
use gage::testsupport::fast_scrypt;
use gage::{CreateSpec, ErrorKind, InitSpec, UnlockMethod, VaultId, VaultType};
use zeroize::Zeroizing;

use crate::{
    ClonedVault, GageError, Identity, PromptError, Prompter, Remedy, Roots, UnlockKind,
    UnlockPurpose, UnlockRequest, UnlockResponse, Vault, Warning, clone_vault, init_vault,
    remote_token_hosts, remove_remote_token, save_remote_token,
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
        roots_of(&self.dirs)
    }

    fn open(&self) -> Arc<Vault> {
        Vault::open(self.roots(), VAULT.into()).unwrap()
    }

    /// The registered vault with a seam replaced.
    fn open_wrapped(&self, wrap: impl FnOnce(gage::Vault) -> gage::Vault) -> Vault {
        Vault {
            inner: wrap(self.open().inner.clone()),
            dirs: self.dirs.clone(),
        }
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
    let expected = GageError::from_core(core);
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
            let e = GageError::new(kind, "m".into(), None);
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
    let vault = f.open_wrapped(|v| v.with_locker(locker.clone()));

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
    let vault = f.open_wrapped(|v| v.with_locker(Arc::new(RefusingLocker)));
    let prompter = ScriptedPrompter::answering(vec![Answer::Passphrase(PASSPHRASE)]);

    let identity = vault.unlock(prompter.clone()).unwrap();

    let warnings = prompter.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(!warnings[0].text.is_empty());
    // Degraded, not failed: the identity still decrypts.
    assert_eq!(vault.list(identity).unwrap().len(), 1);
}

#[test]
fn a_warnings_remedy_arrives_as_its_own_case() {
    let f = Fixture::new(&["alpha"]);
    let vault = f.open();
    // The first unlock bootstraps the trust cache this device compares with.
    unlock(&vault);
    let recipients = vault.inner.path.join(".age-recipients");
    let mut edited = std::fs::read_to_string(&recipients).unwrap();
    edited.push_str(&format!(
        "{}\n",
        gage::recovery::new_recovery_key().unwrap().pubkey
    ));
    std::fs::write(&recipients, edited).unwrap();
    let prompter = ScriptedPrompter::answering(vec![Answer::Passphrase(PASSPHRASE)]);

    vault.unlock(prompter.clone()).unwrap();

    let warnings = prompter.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].remedy, Some(Remedy::RecipientVerify));
}

#[test]
fn a_warning_with_no_remedy_arrives_with_none() {
    let f = Fixture::new(&[]);
    let vault = f.open_wrapped(|v| v.with_locker(Arc::new(RefusingLocker)));
    let prompter = ScriptedPrompter::answering(vec![Answer::Passphrase(PASSPHRASE)]);

    vault.unlock(prompter.clone()).unwrap();

    assert_eq!(prompter.warnings()[0].remedy, None);
}

#[test]
fn an_errors_remedy_arrives_as_its_own_case() {
    let f = Fixture::new(&[]);
    // An identity file that isn't there: this device has no way in yet.
    std::fs::remove_dir_all(f.dirs.data.join("identities")).unwrap();

    let err = f
        .open()
        .unlock(ScriptedPrompter::answering(vec![]))
        .unwrap_err();

    assert!(matches!(err, GageError::LockedOrAuth { .. }), "got {err:?}");
    assert_eq!(err.remedy(), Some(Remedy::EnrollOrRecover));
}

#[test]
fn an_error_with_no_remedy_has_none() {
    let f = Fixture::new(&[]);

    let err = Vault::open(f.roots(), "nope".into()).unwrap_err();

    assert_eq!(err.remedy(), None);
}

/// One of every remedy the library has today. `#[non_exhaustive]` stops this
/// file from noticing a new one at compile time, so a remedy added to the
/// library is added here by whoever adds its case.
fn every_library_remedy() -> Vec<gage::Remedy> {
    use gage::Remedy as R;
    vec![
        R::Sync,
        R::SyncAgain,
        R::RecipientVerify,
        R::RecipientRepair,
        R::AuthLogin { host: "h".into() },
        R::AuthLoginAgain { host: "h".into() },
        R::UseHttps,
        R::ReRegister { name: "n".into() },
        R::EnrollOrRecover,
        R::SetRemote,
        R::Reclone { vault: "v".into() },
        R::EnrollAgain {
            auth_refused: true,
            host: Some("h".into()),
        },
        R::Push,
        R::Force,
        R::ChooseDevice,
        R::ApproveWithDevice,
        R::PassName { plain: true },
        R::Yes,
    ]
}

#[test]
fn every_library_remedy_maps_to_its_own_case_with_its_data() {
    // The case's name, and what it carries, match the library's exactly: the
    // two enums print alike, so a swapped arm or a dropped field shows here.
    for remedy in every_library_remedy() {
        let mirrored = Remedy::from(&remedy);
        assert_ne!(mirrored, Remedy::Other, "{remedy:?} fell through to Other");
        assert_eq!(format!("{mirrored:?}"), format!("{remedy:?}"));
    }
}

// --- create and join ----------------------------------------------------

/// An app's three directories, on a device that has nothing yet.
struct Phone {
    _root: tempfile::TempDir,
    dirs: Dirs,
}

impl Phone {
    fn new() -> Phone {
        fast_scrypt();
        let root = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            config: root.path().join("config"),
            data: root.path().join("data"),
            state: root.path().join("state"),
        };
        Phone { _root: root, dirs }
    }

    fn roots(&self) -> Roots {
        roots_of(&self.dirs)
    }

    fn vault_dir(&self) -> std::path::PathBuf {
        paths::vaults_dir(&self.dirs).join(VAULT)
    }

    fn clone_from(&self, remote: &Path) -> ClonedVault {
        clone_vault(
            self.roots(),
            VAULT.into(),
            remote.to_str().unwrap().into(),
            DEVICE.into(),
        )
        .unwrap()
    }

    /// No identity file exists for any vault here.
    fn holds_no_identity(&self) -> bool {
        let dir = self.dirs.data.join("identities");
        !dir.exists()
            || std::fs::read_dir(dir).unwrap().all(|vault| {
                std::fs::read_dir(vault.unwrap().path())
                    .unwrap()
                    .next()
                    .is_none()
            })
    }
}

fn roots_of(dirs: &Dirs) -> Roots {
    let s = |p: &Path| p.to_str().unwrap().to_string();
    Roots {
        config: s(&dirs.config),
        data: s(&dirs.data),
        state: s(&dirs.state),
    }
}

/// An entry to start a vault with: title, value and fields.
type SeedEntry<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

/// A device already in the vault, driven through the library as `gage-cli`
/// drives it: the side of an enrollment that isn't the app.
struct Owner {
    _machine: Phone,
    vault: gage::Vault,
    identity: gage::Identity,
    remote: BareRemote,
}

impl Owner {
    fn publishing(entries: &[SeedEntry<'_>]) -> Owner {
        let machine = Phone::new();
        let remote = gittest::new_bare_remote();
        let vault = gage::Vault::init(
            &machine.dirs,
            &InitSpec {
                name: VAULT.into(),
                path: machine.vault_dir(),
                vault_type: VaultType::Git,
                method: UnlockMethod::Passphrase,
                device: "laptop".into(),
                recipients: Vec::new(),
                recovery_key: None,
                remote: Some(remote.path().to_str().unwrap().into()),
            },
            &FakePrompter::new().with_passphrase(PASSPHRASE),
        )
        .unwrap()
        .vault;
        let identity = vault
            .unlock(&FakePrompter::new().with_passphrase(PASSPHRASE))
            .unwrap();
        for (title, value, fields) in entries {
            vault
                .insert(
                    &identity,
                    NewEntry {
                        title: title.to_string(),
                        description: format!("about {title}"),
                        value: value.to_string(),
                        fields: fields
                            .iter()
                            .map(|(k, v)| (k.to_string(), v.to_string()))
                            .collect(),
                        ..NewEntry::default()
                    },
                    true,
                    &FakePrompter::new(),
                )
                .unwrap();
        }
        vault.push().unwrap();
        Owner {
            _machine: machine,
            vault,
            identity,
            remote,
        }
    }

    /// `gage recipient approve --code`, through the library.
    fn approve(&self, code: &str) {
        self.vault.pull().unwrap();
        let codes = [Zeroizing::new(code.to_string())];
        let pending = self.vault.pending_enrollments().unwrap();
        let approvals: Vec<Approval> = self
            .vault
            .open_enrollment(&pending, &codes)
            .unwrap()
            .into_iter()
            .map(Approval::new)
            .collect();
        self.vault
            .approve_enrollments(&approvals, &codes, &self.identity, &FakePrompter::new())
            .unwrap();
    }
}

fn answering_passphrase() -> Arc<ScriptedPrompter> {
    ScriptedPrompter::answering(vec![Answer::Passphrase(PASSPHRASE)])
}

/// A phone that cloned `owner`'s vault, enrolled, and was approved.
fn joined(owner: &Owner) -> (Phone, Arc<Vault>, Arc<Identity>) {
    let phone = Phone::new();
    let vault = phone.clone_from(owner.remote.path()).vault;
    let request = vault
        .enroll(DEVICE.into(), None, answering_passphrase())
        .unwrap();
    owner.approve(&request.code);
    vault.sync(answering_passphrase()).unwrap();
    let identity = vault.unlock(answering_passphrase()).unwrap();
    (phone, vault, identity)
}

#[test]
fn init_creates_and_registers_a_vault_that_list_then_reads() {
    let phone = Phone::new();
    let prompter = answering_passphrase();

    let made = init_vault(
        phone.roots(),
        VAULT.into(),
        DEVICE.into(),
        None,
        false,
        prompter.clone(),
    )
    .unwrap();

    let requests = prompter.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].purpose, UnlockPurpose::Create);
    assert_eq!(requests[0].device, DEVICE);
    assert!(made.pubkey.starts_with("age1"), "{}", made.pubkey);
    assert_eq!(made.vault.inner.path, phone.vault_dir());

    // Registered: a later launch finds it by name.
    let reopened = Vault::open(phone.roots(), VAULT.into()).unwrap();
    let identity = reopened.unlock(answering_passphrase()).unwrap();
    assert!(reopened.list(identity).unwrap().is_empty());
}

#[test]
fn cancelling_the_new_passphrase_creates_nothing() {
    let phone = Phone::new();
    let prompter = ScriptedPrompter::answering(vec![Answer::Fail(PromptError::Cancelled)]);

    let err = init_vault(
        phone.roots(),
        VAULT.into(),
        DEVICE.into(),
        None,
        true,
        prompter,
    )
    .unwrap_err();

    assert!(matches!(err, GageError::LockedOrAuth { .. }), "got {err:?}");
    assert!(!phone.vault_dir().exists());
    assert!(phone.holds_no_identity());
    assert!(matches!(
        Vault::open(phone.roots(), VAULT.into()).unwrap_err(),
        GageError::NotFound { .. }
    ));
}

#[test]
fn init_returns_the_recovery_key_only_when_asked_and_lists_it_as_a_recipient() {
    let without = Phone::new();
    let made = init_vault(
        without.roots(),
        VAULT.into(),
        DEVICE.into(),
        None,
        false,
        answering_passphrase(),
    )
    .unwrap();
    assert_eq!(made.recovery_key, None);
    assert_eq!(made.vault.inner.info().unwrap().recipients, 1);

    let with = Phone::new();
    let made = init_vault(
        with.roots(),
        VAULT.into(),
        DEVICE.into(),
        None,
        true,
        answering_passphrase(),
    )
    .unwrap();
    let key = made.recovery_key.as_deref().unwrap();
    assert!(key.starts_with("AGE-SECRET-KEY-1"), "not a private key");
    assert_eq!(made.vault.inner.info().unwrap().recipients, 2);
    assert!(
        !format!("{made:?}").contains(key),
        "Debug printed the recovery key"
    );
}

#[test]
fn a_new_vault_with_a_remote_is_published_by_sync() {
    let phone = Phone::new();
    let remote = gittest::new_bare_remote();

    let made = init_vault(
        phone.roots(),
        VAULT.into(),
        DEVICE.into(),
        Some(remote.path().to_str().unwrap().into()),
        false,
        answering_passphrase(),
    )
    .unwrap();
    let report = made.vault.sync(answering_passphrase()).unwrap();

    assert!(report.pushed, "{report:?}");
    assert!(gittest::Device::clone(remote.path()).exists(".gage/config.toml"));
}

#[test]
fn clone_registers_the_vault_and_reports_this_device_holds_no_identity() {
    let owner = Owner::publishing(&[("alpha", "v", &[])]);
    let phone = Phone::new();

    let cloned = phone.clone_from(owner.remote.path());

    assert!(!cloned.holds_identity);
    assert_eq!(cloned.vault.inner.path, phone.vault_dir());
    let reopened = Vault::open(phone.roots(), VAULT.into()).unwrap();
    let err = reopened
        .unlock(ScriptedPrompter::answering(vec![]))
        .unwrap_err();
    assert_eq!(err.remedy(), Some(Remedy::EnrollOrRecover), "{err:?}");
}

#[test]
fn enroll_returns_the_code_and_expiry_and_publishes_the_request() {
    let owner = Owner::publishing(&[]);
    let phone = Phone::new();
    let vault = phone.clone_from(owner.remote.path()).vault;
    let prompter = answering_passphrase();

    let request = vault.enroll(DEVICE.into(), None, prompter.clone()).unwrap();

    assert!(request.published);
    assert!(!request.code.is_empty());
    assert_eq!(request.device, DEVICE);
    assert!(request.pubkey.starts_with("age1"));
    let left = request.expires.duration_since(SystemTime::now()).unwrap();
    assert!(left <= gage::enrollment::DEFAULT_ENROLLMENT_TTL, "{left:?}");
    assert!(left > Duration::from_secs(23 * 60 * 60), "{left:?}");
    // The new identity's passphrase, marked as such.
    assert_eq!(prompter.requests()[0].purpose, UnlockPurpose::Create);
    assert!(
        !format!("{request:?}").contains(&request.code),
        "Debug printed the code"
    );

    owner.vault.pull().unwrap();
    assert_eq!(owner.vault.pending_enrollments().unwrap().len(), 1);

    // The registration now describes this device, so it can tell later.
    let global = config::load(&paths::global_config_file(&phone.dirs)).unwrap();
    let entry = global.entry(VAULT).unwrap();
    assert_eq!(entry.pubkey.as_deref(), Some(request.pubkey.as_str()));
    assert_eq!(entry.device, DEVICE);
}

#[test]
fn enroll_takes_the_ttl_it_is_given_and_refuses_one_out_of_range() {
    let owner = Owner::publishing(&[]);
    let phone = Phone::new();
    let vault = phone.clone_from(owner.remote.path()).vault;

    let err = vault
        .enroll(DEVICE.into(), Some(0), ScriptedPrompter::answering(vec![]))
        .unwrap_err();
    assert!(matches!(err, GageError::Usage { .. }), "got {err:?}");

    let request = vault
        .enroll(DEVICE.into(), Some(600), answering_passphrase())
        .unwrap();
    let left = request.expires.duration_since(SystemTime::now()).unwrap();
    assert!(left <= Duration::from_secs(600), "{left:?}");
}

#[test]
fn once_approved_sync_then_unlock_reads_the_vault() {
    let owner = Owner::publishing(&[("alpha", "v-alpha", &[])]);
    let phone = Phone::new();
    let vault = phone.clone_from(owner.remote.path()).vault;
    let request = vault
        .enroll(DEVICE.into(), None, answering_passphrase())
        .unwrap();
    owner.approve(&request.code);

    let report = vault.sync(answering_passphrase()).unwrap();
    assert!(report.pulled, "{report:?}");
    let identity = vault.unlock(answering_passphrase()).unwrap();

    let rows = vault.list(identity).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "alpha");
}

#[test]
fn enrolling_again_once_a_recipient_publishes_nothing() {
    let owner = Owner::publishing(&[]);
    let (_phone, vault, _identity) = joined(&owner);

    let again = vault
        .enroll(DEVICE.into(), None, ScriptedPrompter::answering(vec![]))
        .unwrap();

    assert!(!again.published);
    assert!(again.code.is_empty());
}

#[test]
fn init_into_a_taken_target_is_a_conflict_and_removes_the_identity_it_made() {
    let phone = Phone::new();
    std::fs::create_dir_all(phone.vault_dir()).unwrap();
    std::fs::write(phone.vault_dir().join("keep.txt"), "mine").unwrap();

    let err = init_vault(
        phone.roots(),
        VAULT.into(),
        DEVICE.into(),
        None,
        false,
        answering_passphrase(),
    )
    .unwrap_err();

    assert!(matches!(err, GageError::Conflict { .. }), "got {err:?}");
    assert!(phone.holds_no_identity());
    assert!(phone.vault_dir().join("keep.txt").exists());
    assert!(matches!(
        Vault::open(phone.roots(), VAULT.into()).unwrap_err(),
        GageError::NotFound { .. }
    ));
}

#[test]
fn a_registered_name_is_refused_before_asking_anything() {
    let phone = Phone::new();
    init_vault(
        phone.roots(),
        VAULT.into(),
        DEVICE.into(),
        None,
        false,
        answering_passphrase(),
    )
    .unwrap();

    let again = init_vault(
        phone.roots(),
        VAULT.into(),
        DEVICE.into(),
        None,
        false,
        ScriptedPrompter::answering(vec![]),
    )
    .unwrap_err();
    let core = gage::storage::config::RegistryError::AlreadyRegistered(VAULT.into());
    assert_eq!(
        format!("{again:?}"),
        format!("{:?}", GageError::from_core(core))
    );

    let cloned = clone_vault(
        phone.roots(),
        VAULT.into(),
        gittest::new_bare_remote().path().to_str().unwrap().into(),
        DEVICE.into(),
    )
    .map(|_| ())
    .unwrap_err();
    assert_eq!(format!("{cloned:?}"), format!("{again:?}"));
}

#[test]
fn a_failed_clone_is_undone_and_registers_nothing() {
    let phone = Phone::new();
    let missing = phone.dirs.data.join("no-such-remote.git");

    let err = clone_vault(
        phone.roots(),
        VAULT.into(),
        missing.to_str().unwrap().into(),
        DEVICE.into(),
    )
    .map(|_| ())
    .unwrap_err();

    assert!(!matches!(err, GageError::Usage { .. }), "got {err:?}");
    assert!(!phone.vault_dir().exists());
    assert!(matches!(
        Vault::open(phone.roots(), VAULT.into()).unwrap_err(),
        GageError::NotFound { .. }
    ));
}

#[test]
fn a_clone_this_build_cannot_read_rolls_back_as_a_conflict() {
    let owner = Owner::publishing(&[]);
    let other = gittest::Device::clone(owner.remote.path());
    let raw = other.read(".gage/config.toml");
    let current = format!(
        "format_version = {}",
        gage::storage::vaultconfig::CURRENT_FORMAT_VERSION
    );
    other.write_commit_push(
        ".gage/config.toml",
        &raw.replace(&current, "format_version = 99"),
        "a newer gage wrote this vault",
    );
    let phone = Phone::new();

    let err = clone_vault(
        phone.roots(),
        VAULT.into(),
        owner.remote.path().to_str().unwrap().into(),
        DEVICE.into(),
    )
    .map(|_| ())
    .unwrap_err();

    assert!(matches!(err, GageError::Conflict { .. }), "got {err:?}");
    assert!(!phone.vault_dir().exists());
}

#[test]
fn a_failed_rollback_is_internal_and_says_both_things() {
    let clone = gage::vault::CloneError::RollbackFailed {
        cause: Box::new(gage::vault::CloneError::InvalidDeviceName("d".into())),
        path: "/somewhere".into(),
        rollback: io::Error::other("still busy"),
    };
    let err = GageError::from_core(clone);
    assert!(matches!(err, GageError::Internal { .. }), "got {err:?}");
    assert!(err.to_string().contains("still busy"), "{err}");

    let init = gage::vault::InitError::RollbackFailed {
        cause: Box::new(gage::vault::CreateError::EmptyPath),
        rollback: Box::new(gage::vault::UnlockError::NoIdentityFile("/k".into())),
    };
    let err = GageError::from_core(init);
    assert!(matches!(err, GageError::Internal { .. }), "got {err:?}");
    assert!(err.to_string().contains("rolling back"), "{err}");
}

#[test]
fn the_questions_nothing_here_asks_stay_refused() {
    use gage::prompter::{PromptError as CorePromptError, Prompter as CorePrompter};
    let foreign = ScriptedPrompter::answering(vec![]);
    let bridge = crate::Bridge(foreign.as_ref());

    assert!(matches!(
        bridge.confirm("?"),
        Err(CorePromptError::NotInteractive)
    ));
    assert!(matches!(
        bridge.confirm_default_yes("?"),
        Err(CorePromptError::NotInteractive)
    ));
    assert!(matches!(
        bridge.value("t"),
        Err(CorePromptError::NotInteractive)
    ));
}

#[test]
fn a_name_that_would_leave_the_vaults_directory_is_refused_before_anything() {
    let phone = Phone::new();
    let escaped = phone.dirs.data.join("evil");

    let init = init_vault(
        phone.roots(),
        "../evil".into(),
        DEVICE.into(),
        None,
        false,
        ScriptedPrompter::answering(vec![]),
    )
    .map(|_| ())
    .unwrap_err();
    let clone = clone_vault(
        phone.roots(),
        "../evil".into(),
        gittest::new_bare_remote().path().to_str().unwrap().into(),
        DEVICE.into(),
    )
    .map(|_| ())
    .unwrap_err();

    assert!(matches!(init, GageError::Usage { .. }), "got {init:?}");
    assert!(matches!(clone, GageError::Usage { .. }), "got {clone:?}");
    assert!(!escaped.exists());
}

// --- remote tokens and sync ---------------------------------------------

const TOKEN: &str = "tok_do_not_print_me";
const RECORDER_HOST: &str = "127.0.0.1";

/// A phone holding a new vault whose origin is `url`.
fn phone_with_origin(url: String) -> (Phone, Arc<Vault>) {
    let phone = Phone::new();
    let vault = init_vault(
        phone.roots(),
        VAULT.into(),
        DEVICE.into(),
        Some(url),
        false,
        answering_passphrase(),
    )
    .unwrap()
    .vault;
    (phone, vault)
}

#[test]
fn a_saved_token_is_used_by_the_next_sync_and_a_refusal_says_to_store_one() {
    let recorder = HttpRecorder::start(RecorderAnswer::Challenge);
    let (phone, vault) = phone_with_origin(recorder.url("me/vault.git"));
    save_remote_token(phone.roots(), RECORDER_HOST.into(), TOKEN.into()).unwrap();

    let err = vault.sync(ScriptedPrompter::answering(vec![])).unwrap_err();

    let sent = recorder.authorizations();
    assert!(sent.contains(&gittest::basic("gage", TOKEN)), "{sent:?}");
    assert!(matches!(err, GageError::LockedOrAuth { .. }), "got {err:?}");
    assert_eq!(
        err.remedy(),
        Some(Remedy::AuthLogin {
            host: RECORDER_HOST.into()
        })
    );
}

#[test]
fn removing_a_token_deletes_it_and_hosts_lists_what_is_stored() {
    let recorder = HttpRecorder::start(RecorderAnswer::Challenge);
    let (phone, vault) = phone_with_origin(recorder.url("me/vault.git"));
    assert!(remote_token_hosts(phone.roots()).unwrap().is_empty());

    save_remote_token(phone.roots(), "git.example.com".into(), TOKEN.into()).unwrap();
    save_remote_token(phone.roots(), RECORDER_HOST.into(), TOKEN.into()).unwrap();
    assert_eq!(
        remote_token_hosts(phone.roots()).unwrap(),
        [RECORDER_HOST, "git.example.com"]
    );

    remove_remote_token(phone.roots(), RECORDER_HOST.into()).unwrap();
    assert_eq!(
        remote_token_hosts(phone.roots()).unwrap(),
        ["git.example.com"]
    );
    // Removing one that isn't there is not an error.
    remove_remote_token(phone.roots(), RECORDER_HOST.into()).unwrap();

    vault.sync(ScriptedPrompter::answering(vec![])).unwrap_err();
    assert!(
        recorder.authorizations().is_empty(),
        "a removed token was still offered"
    );
}

#[test]
fn a_token_never_appears_in_an_error() {
    for answer in [
        RecorderAnswer::Challenge,
        RecorderAnswer::Status(403),
        RecorderAnswer::Status(404),
    ] {
        let recorder = HttpRecorder::start(answer);
        let (phone, vault) = phone_with_origin(recorder.url("me/vault.git"));
        save_remote_token(phone.roots(), RECORDER_HOST.into(), TOKEN.into()).unwrap();

        let err = vault.sync(ScriptedPrompter::answering(vec![])).unwrap_err();

        let shown = format!("{err} {err:?}");
        assert!(!shown.contains(TOKEN), "{shown}");
    }

    // Refused on the way in: the message may name the host, never the token.
    let phone = Phone::new();
    let err = save_remote_token(phone.roots(), "../evil".into(), TOKEN.into()).unwrap_err();
    assert!(matches!(err, GageError::Usage { .. }), "got {err:?}");
    assert!(!format!("{err} {err:?}").contains(TOKEN));
}

#[test]
fn an_empty_token_is_usage() {
    let phone = Phone::new();

    let err = save_remote_token(phone.roots(), "git.example.com".into(), " \n".into()).unwrap_err();

    assert!(matches!(err, GageError::Usage { .. }), "got {err:?}");
    assert!(remote_token_hosts(phone.roots()).unwrap().is_empty());
}

// A stored token someone else can read is refused, not used: AuthLoginAgain,
// which is what the library says about the file rather than the remote.
#[cfg(unix)]
#[test]
fn a_token_file_with_loose_permissions_says_to_store_it_again() {
    use std::os::unix::fs::PermissionsExt;

    let recorder = HttpRecorder::start(RecorderAnswer::Challenge);
    let (phone, vault) = phone_with_origin(recorder.url("me/vault.git"));
    save_remote_token(phone.roots(), RECORDER_HOST.into(), TOKEN.into()).unwrap();
    let file = paths::token_file_path(&phone.dirs, RECORDER_HOST).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

    let err = vault.sync(ScriptedPrompter::answering(vec![])).unwrap_err();

    assert_eq!(
        err.remedy(),
        Some(Remedy::AuthLoginAgain {
            host: RECORDER_HOST.into()
        }),
        "{err:?}"
    );
}

/// A network that isn't there.
struct Offline;

impl RemoteSyncer for Offline {
    fn fetch(&self, _: Instant) -> Result<bool, gitrepo::Error> {
        Err(gitrepo::Error::Sync(syncerr::Error::new(
            syncerr::Kind::Unreachable,
            "connection refused",
        )))
    }

    fn push(&self, deadline: Instant) -> Result<bool, gitrepo::Error> {
        self.fetch(deadline)
    }
}

#[test]
fn an_unreachable_remote_is_unreachable() {
    let owner = Owner::publishing(&[]);
    let phone = Phone::new();
    let cloned = phone.clone_from(owner.remote.path()).vault;
    let vault = Vault {
        inner: cloned.inner.clone().with_remote_syncer(Arc::new(Offline)),
        dirs: phone.dirs.clone(),
    };

    let err = vault.sync(ScriptedPrompter::answering(vec![])).unwrap_err();

    assert!(matches!(err, GageError::Unreachable { .. }), "got {err:?}");
}

#[test]
fn sync_reports_what_it_did() {
    let owner = Owner::publishing(&[]);
    let (_phone, vault, _identity) = joined(&owner);

    let idle = vault.sync(ScriptedPrompter::answering(vec![])).unwrap();
    assert_eq!(
        idle,
        crate::SyncReport {
            pulled: false,
            merged: false,
            pushed: false,
            diverged: false,
            ahead: 0,
        }
    );

    owner
        .vault
        .insert(
            &owner.identity,
            NewEntry {
                title: "later".into(),
                value: "v".into(),
                ..NewEntry::default()
            },
            true,
            &FakePrompter::new(),
        )
        .unwrap();
    owner.vault.push().unwrap();

    assert!(
        vault
            .sync(ScriptedPrompter::answering(vec![]))
            .unwrap()
            .pulled
    );
}

// --- search, show, reveal -----------------------------------------------

const SECRET: &str = "s3cret-value-needle";
const FIELD_SECRET: &str = "AKIA-field-needle";

/// A joined phone reading a vault with one entry holding a value and a field.
fn reader() -> (Owner, Phone, Arc<Vault>, Arc<Identity>) {
    let owner = Owner::publishing(&[
        (
            "cloud",
            SECRET,
            &[("access_key", FIELD_SECRET), ("region", "eu")],
        ),
        ("other", "unrelated", &[]),
    ]);
    let (phone, vault, identity) = joined(&owner);
    (owner, phone, vault, identity)
}

fn id_of(vault: &Vault, identity: &Arc<Identity>, title: &str) -> String {
    vault
        .list(identity.clone())
        .unwrap()
        .into_iter()
        .find(|row| row.title == title)
        .unwrap()
        .id
}

#[test]
fn search_returns_ids_and_titles_and_says_a_body_match_without_saying_where() {
    let (_owner, _phone, vault, identity) = reader();
    let cloud = id_of(&vault, &identity, "cloud");

    let by_title = vault.search(identity.clone(), "CLOUD".into()).unwrap();
    assert_eq!(by_title.len(), 1, "{by_title:?}");
    assert_eq!(by_title[0].id, cloud);
    assert_eq!(by_title[0].title, "cloud");
    assert_eq!(by_title[0].description, "about cloud");
    assert!(!by_title[0].matched_body);

    let by_body = vault
        .search(identity.clone(), "field-needle".into())
        .unwrap();
    assert_eq!(by_body.len(), 1, "{by_body:?}");
    assert_eq!(by_body[0].id, cloud);
    assert!(by_body[0].matched_body);
    let shown = format!("{by_body:?}");
    assert!(
        !shown.contains(FIELD_SECRET) && !shown.contains("access_key"),
        "{shown}"
    );

    assert!(
        vault
            .search(identity, "nothing has this".into())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn show_returns_metadata_and_field_names_with_no_secret_in_it() {
    let (_owner, _phone, vault, identity) = reader();
    let id = id_of(&vault, &identity, "cloud");

    let shown = vault.show(identity, id.clone()).unwrap();

    assert_eq!(shown.id, id);
    assert_eq!(shown.title, "cloud");
    assert_eq!(shown.description, "about cloud");
    assert_eq!(shown.updated_by, "laptop");
    assert!(shown.created.is_some());
    assert_eq!(shown.field_names, ["access_key", "region"]);
    let printed = format!("{shown:?}");
    assert!(
        !printed.contains(SECRET) && !printed.contains(FIELD_SECRET),
        "{printed}"
    );
}

#[test]
fn reveal_returns_the_value_or_one_field() {
    let (_owner, _phone, vault, identity) = reader();
    let id = id_of(&vault, &identity, "cloud");

    assert_eq!(
        vault.reveal(identity.clone(), id.clone(), None).unwrap(),
        SECRET
    );
    assert_eq!(
        vault
            .reveal(identity, id, Some("access_key".into()))
            .unwrap(),
        FIELD_SECRET
    );
}

#[test]
fn a_field_that_is_not_there_is_not_found_and_names_no_value() {
    let (_owner, _phone, vault, identity) = reader();
    let id = id_of(&vault, &identity, "cloud");

    let err = vault.reveal(identity, id, Some("nope".into())).unwrap_err();

    assert!(matches!(err, GageError::NotFound { .. }), "got {err:?}");
    let printed = format!("{err} {err:?}");
    assert!(
        !printed.contains(SECRET) && !printed.contains(FIELD_SECRET),
        "{printed}"
    );
}

#[test]
fn an_unknown_id_is_not_found() {
    let (_owner, _phone, vault, identity) = reader();
    let unknown = "00000000-0000-4000-8000-000000000000".to_string();

    let show = vault.show(identity.clone(), unknown.clone()).unwrap_err();
    let reveal = vault.reveal(identity, unknown, None).unwrap_err();

    assert!(matches!(show, GageError::NotFound { .. }), "got {show:?}");
    assert!(
        matches!(reveal, GageError::NotFound { .. }),
        "got {reveal:?}"
    );
}

#[test]
fn a_string_that_is_not_an_id_is_usage_and_builds_no_path() {
    let (_owner, _phone, vault, identity) = reader();

    for bad in ["cloud", "../../config.toml", ""] {
        let err = vault.show(identity.clone(), bad.into()).unwrap_err();
        assert!(matches!(err, GageError::Usage { .. }), "{bad:?}: {err:?}");
        let err = vault
            .reveal(identity.clone(), bad.into(), None)
            .unwrap_err();
        assert!(matches!(err, GageError::Usage { .. }), "{bad:?}: {err:?}");
    }
}

#[test]
fn an_identity_that_is_not_a_recipient_fails_typed() {
    let owner = Owner::publishing(&[("alpha", "v", &[])]);
    let phone = Phone::new();
    let vault = phone.clone_from(owner.remote.path()).vault;
    // Enrolled and not approved: it has a key, and nothing is encrypted to it.
    vault
        .enroll(DEVICE.into(), None, answering_passphrase())
        .unwrap();
    let identity = vault.unlock(answering_passphrase()).unwrap();
    let id = gage::Vault::entry_ids(&vault.inner).unwrap()[0]
        .hyphenated()
        .to_string();

    let show = vault.show(identity.clone(), id.clone()).unwrap_err();
    let reveal = vault.reveal(identity.clone(), id, None).unwrap_err();
    let search = vault.search(identity, "alpha".into()).unwrap_err();

    for err in [show, reveal, search] {
        assert!(matches!(err, GageError::LockedOrAuth { .. }), "got {err:?}");
    }
}

#[test]
fn a_wiped_entry_holds_no_secret_anywhere_it_could() {
    use gage::entry::{Entry, Extra};
    let mut entry = Entry {
        value: SECRET.into(),
        fields: [("k".to_string(), FIELD_SECRET.to_string())].into(),
        extra: [(
            "unknown".to_string(),
            Extra::Map(vec![(
                "nested".to_string(),
                Extra::Seq(vec![
                    Extra::Plain("plain-secret".into()),
                    Extra::Text("text-secret".into()),
                ]),
            )]),
        )]
        .into(),
        ..Entry::default()
    };

    crate::wipe_entry(&mut entry);

    assert!(entry.value.is_empty());
    assert!(entry.fields.values().all(String::is_empty));
    let Extra::Map(pairs) = &entry.extra["unknown"] else {
        panic!("the shape changed");
    };
    let Extra::Seq(items) = &pairs[0].1 else {
        panic!("the shape changed");
    };
    assert_eq!(items[0], Extra::Plain(String::new()));
    assert_eq!(items[1], Extra::Text(String::new()));
}
