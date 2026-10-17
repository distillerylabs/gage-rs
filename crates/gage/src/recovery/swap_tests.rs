//! RK2: the recovery identity and the swap. Go:
//! `internal/gage/recovery_enroll_test.go`, every row but
//! `TestTheRecoveryLabelIsReservedForTheRecoveryKey` (RK1's). Each test
//! names its Go row.
//!
//! A child of `swap`, so it reaches the private `recovery_identity` and
//! `apply_swap` without either being widened.

use std::sync::Arc;

use zeroize::Zeroizing;

use super::*;
use crate::config::{self, Global, VaultEntry};
use crate::exitcode::{Code, HasCode};
use crate::gittest::{self, BareRemote};
use crate::locker::Locker;
use crate::prompter::FakePrompter;
use crate::recipient::RecipientError;
use crate::recovery::{RecoveryKey, new_recovery_key};
use crate::remotesync::tests::{sample, unlocks};
use crate::testsupport::fast_scrypt;
use crate::trustcache::TrustError;
use crate::vault::{self, CreateSpec, LabelledRecipient};
use crate::vaultconfig::Recipient;
use crate::xdgpaths::Dirs;
use crate::{gitrepo, identityfile, paths, vaultconfig};

const NAME: &str = "personal";
const OWNER: &str = "laptop-1";

// --- the fixture -------------------------------------------------------------

/// A vault in the shape `gage init` now leaves: the owner's wrapped
/// identity on disk and registered, a recovery recipient beside it, a
/// published first commit (Go's `newRecoverFixture`).
pub(super) struct Recover {
    _root: tempfile::TempDir,
    _remote: BareRemote,
    pub(super) vault: Vault,
    pub(super) owner_pubkey: String,
    /// `None` for a vault made without one.
    pub(super) key: Option<RecoveryKey>,
}

impl Recover {
    pub(super) fn new() -> Recover {
        Recover::build(|_, recovery| {
            vec![LabelledRecipient {
                device: RECOVERY_DEVICE_LABEL.into(),
                pubkey: recovery.pubkey.clone(),
            }]
        })
    }

    /// The `--no-recovery-key` shape.
    pub(super) fn without_recovery_key() -> Recover {
        let mut f = Recover::build(|_, _| Vec::new());
        f.key = None;
        f
    }

    /// `extras` gets the owner's pubkey and the generated recovery key.
    fn build(extras: impl FnOnce(&str, &RecoveryKey) -> Vec<LabelledRecipient>) -> Recover {
        fast_scrypt();
        let root = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            config: root.path().join("config"),
            data: root.path().join("data"),
            state: root.path().join("state"),
        };
        let remote = gittest::new_bare_remote();
        let id = vaultconfig::new_id();
        let owner_pubkey = identityfile::create(&dirs, &id, NAME, OWNER, &unlocks()).unwrap();
        let key = new_recovery_key().unwrap();
        let path = root.path().join("vault");
        let origin = remote.path().to_str().unwrap().to_string();
        let vault = Vault::create(
            &dirs,
            &CreateSpec {
                name: NAME.into(),
                id: id.clone(),
                path: path.clone(),
                vault_type: vault::TYPE_GIT.into(),
                method: vault::METHOD_PASSPHRASE.into(),
                device: OWNER.into(),
                recipients: vec![owner_pubkey.clone()],
                extra_recipients: extras(&owner_pubkey, &key),
                remote: Some(origin.clone()),
            },
        )
        .unwrap();
        let mut global = Global::default();
        global
            .register(
                NAME,
                VaultEntry {
                    path,
                    id,
                    vault_type: vault::TYPE_GIT.into(),
                    device: OWNER.into(),
                    method: vault::METHOD_PASSPHRASE.into(),
                    git: config::GitMeta { origin },
                    ..Default::default()
                },
            )
            .unwrap();
        config::save(&paths::global_config_file(&dirs), &global).unwrap();
        gitrepo::push(&vault.path, &gittest::remote_options()).unwrap();
        Recover {
            _root: root,
            _remote: remote,
            vault,
            owner_pubkey,
            key: Some(key),
        }
    }

    pub(super) fn key(&self) -> &RecoveryKey {
        self.key.as_ref().expect("this fixture has a recovery key")
    }

    /// What a human pastes: the recovery secret, as the prompter returns
    /// it.
    pub(super) fn pasted(&self) -> Zeroizing<String> {
        Zeroizing::new(self.key().secret.to_string())
    }

    pub(super) fn unlock(&self) -> Identity {
        self.vault.unlock(&unlocks()).unwrap()
    }

    pub(super) fn insert(&self, title: &str) {
        self.vault
            .insert(&self.unlock(), sample(title), true, &FakePrompter::new())
            .unwrap();
    }

    pub(super) fn head(&self) -> String {
        crate::recipient_tests::head_hash(&self.vault.path)
    }

    pub(super) fn head_message(&self) -> String {
        let repo = git2::Repository::open(&self.vault.path).unwrap();
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        commit.message().unwrap().trim().to_string()
    }

    pub(super) fn commits(&self) -> usize {
        gitrepo::commit_count(&self.vault.path).unwrap()
    }

    pub(super) fn is_clean(&self) -> bool {
        gitrepo::is_clean(&self.vault.path).unwrap()
    }

    /// The list as `device=pubkey` pairs, in order (Go's `labels`).
    pub(super) fn labels(&self) -> Vec<String> {
        self.vault
            .recipients()
            .unwrap()
            .into_iter()
            .map(|r| format!("{}={}", r.device, r.pubkey))
            .collect()
    }

    pub(super) fn devices(&self) -> Vec<String> {
        self.vault
            .recipients()
            .unwrap()
            .into_iter()
            .map(|r| r.device)
            .collect()
    }

    /// `.age-recipients`, read as itself.
    pub(super) fn recipients_file(&self) -> Vec<String> {
        crate::recipients::read(&self.vault.path.join(".age-recipients")).unwrap()
    }

    /// Whether `secret` opens every entry on disk (Go's `decryptsWith`).
    pub(super) fn decrypts_with(&self, secret: &str) -> bool {
        let key: age::x25519::Identity = secret.trim().parse().unwrap();
        let identity = Identity::for_tests(&key);
        let ids = self.vault.entry_ids().unwrap();
        assert!(!ids.is_empty(), "no entries, so this proves nothing");
        ids.iter()
            .all(|id| self.vault.read_entry(*id, &identity).is_ok())
    }
}

/// A key belonging to nothing in particular (Go's `freshPubkey`).
pub(super) fn fresh_pubkey() -> String {
    new_recovery_key().unwrap().pubkey
}

/// Fails on every formatting of `err` that contains `secret`, or any
/// 20-character run of it.
#[track_caller]
pub(super) fn assert_never_quotes(err: &RecoveryError, secret: &str) {
    let secret = secret.trim();
    let renderings = [format!("{err}"), format!("{err:?}"), format!("{err:#}")];
    let chars: Vec<char> = secret.chars().collect();
    for text in &renderings {
        assert!(!text.contains(secret), "quotes the secret: {text}");
        for run in chars.windows(20) {
            let run: String = run.iter().collect();
            assert!(!text.contains(&run), "quotes {run:?} of the secret: {text}");
        }
    }
}

/// A platform that refuses every page lock.
struct RefusesToLock;

impl Locker for RefusesToLock {
    fn lock(&self, _: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other("locked-memory limit reached"))
    }
    fn unlock(&self, _: &[u8]) -> std::io::Result<()> {
        Ok(())
    }
}

// --- task 1: recovery_identity ------------------------------------------------

// Go: TestRecoveryIdentityAcceptsOnlyTheRecoveryRecipient.
#[test]
fn recovery_identity_accepts_only_the_recovery_recipient() {
    let f = Recover::new();

    let id = f
        .vault
        .recovery_identity(f.pasted(), &FakePrompter::new())
        .expect("the vault's own recovery key was refused");

    assert_eq!(id.device(), RECOVERY_DEVICE_LABEL);
    assert_eq!(id.recipient(), f.key().pubkey);
}

// Go: TestRecoveryIdentityRefusesADeviceKey. D12: the recovery path is for
// the recovery key, not any key that happens to be a recipient. A device's
// key is wrapped and reaches gage through unlock; letting a bare one in
// here would make "recovery only" a comment rather than a property.
#[test]
fn recovery_identity_refuses_a_device_key() {
    let f = Recover::new();
    f.insert("first");
    // A device whose bare secret this test holds, admitted the ordinary
    // way, so every entry really is encrypted to it.
    let desktop = new_recovery_key().unwrap();
    f.vault
        .add_recipient(
            "desktop-1",
            &desktop.pubkey,
            &f.unlock(),
            &FakePrompter::new(),
        )
        .unwrap();
    assert!(f.decrypts_with(&desktop.secret), "the key would decrypt");

    let err = f
        .vault
        .recovery_identity(
            Zeroizing::new(desktop.secret.to_string()),
            &FakePrompter::new(),
        )
        .unwrap_err();

    assert!(
        matches!(&err, RecoveryError::NotRecoveryKey { device: Some(d), pubkey, .. }
            if d == "desktop-1" && *pubkey == desktop.pubkey),
        "{err:?}"
    );
    assert_eq!(err.code(), Code::LockedOrAuth);
    assert!(err.to_string().contains("desktop-1"), "{err}");
}

// Go: TestRecoveryIdentityRefusesStrangersAndGarbage.
#[test]
fn recovery_identity_refuses_strangers_and_garbage() {
    let f = Recover::new();
    let stranger = new_recovery_key().unwrap();

    let err = f
        .vault
        .recovery_identity(
            Zeroizing::new(stranger.secret.to_string()),
            &FakePrompter::new(),
        )
        .unwrap_err();
    assert!(
        matches!(&err, RecoveryError::NotRecoveryKey { device: None, pubkey, vault }
            if *pubkey == stranger.pubkey && vault == NAME),
        "{err:?}"
    );
    assert_eq!(err.code(), Code::LockedOrAuth);

    for (name, garbage) in [
        ("garbage", "not a key"),
        ("empty", ""),
        ("a public key", f.key().pubkey.as_str()),
    ] {
        let err = f
            .vault
            .recovery_identity(Zeroizing::new(garbage.to_string()), &FakePrompter::new())
            .unwrap_err();
        assert!(
            matches!(err, RecoveryError::MalformedRecoveryKey),
            "{name}: {err:?}"
        );
        assert_eq!(err.code(), Code::Usage, "{name}");
    }
}

// Go: TestRecoveryIdentityToleratesPastedWhitespace.
#[test]
fn recovery_identity_tolerates_pasted_whitespace() {
    let f = Recover::new();

    let padded = Zeroizing::new(format!("  \n{}\r\n\t", f.key().secret.as_str()));
    let id = f
        .vault
        .recovery_identity(padded, &FakePrompter::new())
        .unwrap();

    assert_eq!(id.recipient(), f.key().pubkey);
}

// Beyond Go's rows: a platform refusing the page lock warns with unlock's
// sentence, naming the recovery label, and the identity still works.
#[test]
fn recovery_identity_warns_like_unlock_when_the_page_lock_fails() {
    let f = Recover::new();
    let vault = f.vault.clone().with_locker(Arc::new(RefusesToLock));
    let p = FakePrompter::new();

    let id = vault.recovery_identity(f.pasted(), &p).unwrap();

    assert!(!id.page_locked());
    let warnings = p.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].starts_with(&format!(
            "gage: could not lock {RECOVERY_DEVICE_LABEL}'s key material into memory"
        )),
        "{warnings:?}"
    );
}

// Go: TestRecoveryErrorsNeverQuoteTheSecret. age's own parse errors can
// echo their input, and a malformed paste is usually most of a real key.
// This covers recovery_identity's refusals; recover_device's are checked
// in its two refusal tables, against the real secret.
#[test]
fn recovery_errors_never_quote_the_secret() {
    let f = Recover::new();
    let stranger = new_recovery_key().unwrap();
    let mangled = format!("X{}", f.key().secret.as_str());
    let truncated = f.key().secret[..f.key().secret.len() - 1].to_string();

    for secret in [stranger.secret.to_string(), mangled, truncated] {
        let err = f
            .vault
            .recovery_identity(Zeroizing::new(secret.clone()), &FakePrompter::new())
            .unwrap_err();
        assert_never_quotes(&err, &secret);
    }
}

// --- task 2: swap_recipients and recover_device -----------------------------

impl Recover {
    pub(super) fn recover(
        &self,
        spec: &RecoverSpec,
        p: &FakePrompter,
    ) -> Result<RecoverResult, RecoveryError> {
        self.vault.recover_device(spec, self.pasted(), p)
    }
}

pub(super) fn spec(device: &str, pubkey: &str) -> RecoverSpec {
    RecoverSpec {
        device: device.into(),
        pubkey: pubkey.into(),
        replaces: None,
        new_recovery_pubkey: None,
    }
}

/// Every entry's `updated_by`, keyed by id.
fn updated_by(f: &Recover) -> Vec<(uuid::Uuid, String)> {
    let id = f.unlock();
    f.vault
        .entry_ids()
        .unwrap()
        .into_iter()
        .map(|e| (e, f.vault.read_entry(e, &id).unwrap().updated_by))
        .collect()
}

/// Asserts a refused swap left HEAD, the list and the tree exactly as
/// they were.
#[track_caller]
pub(super) fn assert_wrote_nothing(f: &Recover, head: &str, labels: &[String], what: &str) {
    assert_eq!(f.head(), head, "{what}: a refused swap moved HEAD");
    assert_eq!(f.labels(), labels, "{what}: the list changed");
    assert!(f.is_clean(), "{what}: the tree is dirty");
}

// Go: TestRecoverDeviceSwapsTheListInOneCommit.
#[test]
fn recover_device_swaps_the_list_in_one_commit() {
    let f = Recover::new();
    f.insert("first");
    f.insert("second");
    let device = fresh_pubkey();
    let next = new_recovery_key().unwrap();
    let (before, commits) = (f.head(), f.commits());

    let res = f
        .recover(
            &RecoverSpec {
                new_recovery_pubkey: Some(next.pubkey.clone()),
                ..spec("laptop-2", &device)
            },
            &FakePrompter::new(),
        )
        .unwrap();

    let want = [
        format!("{OWNER}={}", f.owner_pubkey),
        format!("laptop-2={device}"),
        format!("{RECOVERY_DEVICE_LABEL}={}", next.pubkey),
    ];
    assert_eq!(f.labels(), want);
    assert_eq!(
        f.recipients_file(),
        [f.owner_pubkey.clone(), device.clone(), next.pubkey.clone()]
    );
    assert_ne!(f.head(), before);
    assert_eq!(f.commits(), commits + 1, "the swap must be one commit");
    assert_eq!(res.commit, f.head());
    assert_eq!(res.reencrypted, 2);
    assert_eq!(res.retired_recovery, f.key().pubkey);
    assert_eq!(
        res.new_recovery_pubkey.as_deref(),
        Some(next.pubkey.as_str())
    );
    assert_eq!(res.replaced, None);
    assert!(f.is_clean());

    // `replaces` removes the lost device in that same one commit.
    let g = Recover::new();
    g.insert("first");
    let commits = g.commits();
    let res = g
        .recover(
            &RecoverSpec {
                replaces: Some(OWNER.into()),
                ..spec("laptop-2", &device)
            },
            &FakePrompter::new(),
        )
        .unwrap();
    assert_eq!(g.labels(), [format!("laptop-2={device}")]);
    assert_eq!(g.recipients_file(), [device]);
    assert_eq!(g.commits(), commits + 1);
    assert_eq!(res.replaced.as_deref(), Some(OWNER));
    assert_eq!(
        g.head_message(),
        format!("gage: recovery enroll laptop-2, retire {RECOVERY_DEVICE_LABEL} (reencrypt)")
    );
}

// Go: TestRecoverDeviceRetiresTheUsedKeyForFutureWrites. The claim the
// feature rests on: the key pasted into a terminal stops reading what the
// vault writes next.
#[test]
fn recover_device_retires_the_used_key_for_future_writes() {
    let f = Recover::new();
    f.insert("first");
    let next = new_recovery_key().unwrap();
    let used = f.key().secret.to_string();

    f.recover(
        &RecoverSpec {
            new_recovery_pubkey: Some(next.pubkey.clone()),
            ..spec("laptop-2", &fresh_pubkey())
        },
        &FakePrompter::new(),
    )
    .unwrap();

    assert!(
        !f.decrypts_with(&used),
        "the swap's own rewrite still opens"
    );
    // And what the vault writes afterwards.
    f.insert("written after the swap");
    let old: age::x25519::Identity = used.parse().unwrap();
    let old = Identity::for_tests(&old);
    for id in f.vault.entry_ids().unwrap() {
        assert!(
            f.vault.read_entry(id, &old).is_err(),
            "the retired key opens {id}"
        );
    }
    assert!(f.decrypts_with(&next.secret));
}

// Go: TestRecoverDeviceLetsTheNewDeviceRead. The point of the exercise.
#[test]
fn recover_device_lets_the_new_device_read() {
    let f = Recover::new();
    f.insert("first");
    f.insert("second");
    let replacement = new_recovery_key().unwrap();
    let next = new_recovery_key().unwrap();

    f.recover(
        &RecoverSpec {
            device: "laptop-2".into(),
            pubkey: replacement.pubkey.clone(),
            replaces: Some(OWNER.into()),
            new_recovery_pubkey: Some(next.pubkey.clone()),
        },
        &FakePrompter::new(),
    )
    .unwrap();

    assert_eq!(f.devices(), ["laptop-2", RECOVERY_DEVICE_LABEL]);
    assert!(f.decrypts_with(&replacement.secret), "the new device");
    assert!(f.decrypts_with(&next.secret), "the new recovery key");
}

// Go: TestRecoverDeviceWithoutAReplacementLeavesNoRecoveryRecipient.
#[test]
fn recover_device_without_a_replacement_leaves_no_recovery_recipient() {
    let f = Recover::new();
    f.insert("first");

    let res = f
        .recover(&spec("laptop-2", &fresh_pubkey()), &FakePrompter::new())
        .unwrap();

    assert_eq!(f.devices(), [OWNER, "laptop-2"]);
    assert_eq!(res.new_recovery_pubkey, None);
}

// Go: TestRecoverDeviceLeavesUpdatedByAlone. Re-encryption is not an edit.
#[test]
fn recover_device_leaves_updated_by_alone() {
    let f = Recover::new();
    f.insert("first");
    let before = updated_by(&f);

    f.recover(
        &RecoverSpec {
            new_recovery_pubkey: Some(fresh_pubkey()),
            ..spec("laptop-2", &fresh_pubkey())
        },
        &FakePrompter::new(),
    )
    .unwrap();

    assert_eq!(updated_by(&f), before);
    assert!(before.iter().all(|(_, by)| by == OWNER));
}

// Go: TestRecoverDeviceRefusalsWriteNothing. One case per refusal, each
// pinning its variant and code.
#[test]
fn recover_device_refusals_write_nothing() {
    use RecipientError as R;
    type Check = fn(&RecoveryError) -> bool;
    type Make = Box<dyn Fn(&Recover) -> RecoverSpec>;
    let stranger = fresh_pubkey();
    let cases: Vec<(&str, Make, Check, Code)> = vec![
        (
            "a device name already in use",
            Box::new({
                let k = stranger.clone();
                move |_| spec(OWNER, &k)
            }),
            |e| {
                matches!(e, RecoveryError::Recipient(R::RecipientExists { device, pubkey: None })
                    if device == OWNER)
            },
            Code::Conflict,
        ),
        (
            "a public key already in use",
            Box::new(|f| spec("laptop-2", &f.owner_pubkey)),
            |e| {
                matches!(e, RecoveryError::Recipient(R::RecipientExists { device, pubkey: Some(_) })
                    if device == OWNER)
            },
            Code::Conflict,
        ),
        (
            "a device claiming the recovery label",
            Box::new({
                let k = stranger.clone();
                move |_| spec(RECOVERY_DEVICE_LABEL, &k)
            }),
            |e| matches!(e, RecoveryError::Recipient(R::ReservedDeviceLabel(_))),
            Code::Usage,
        ),
        (
            "an invalid device name",
            Box::new({
                let k = stranger.clone();
                move |_| spec("../escape", &k)
            }),
            |e| matches!(e, RecoveryError::Recipient(R::InvalidDeviceName(_))),
            Code::Usage,
        ),
        (
            "a malformed device key",
            Box::new(|_| spec("laptop-2", "age1nope")),
            |e| matches!(e, RecoveryError::Recipient(R::InvalidRecipient(_))),
            Code::Usage,
        ),
        (
            "a malformed replacement recovery key",
            Box::new({
                let k = stranger.clone();
                move |_| RecoverSpec {
                    new_recovery_pubkey: Some("age1nope".into()),
                    ..spec("laptop-2", &k)
                }
            }),
            |e| matches!(e, RecoveryError::Recipient(R::InvalidRecipient(_))),
            Code::Usage,
        ),
        (
            "a replacement recovery key already in use",
            Box::new({
                let k = stranger.clone();
                move |f| RecoverSpec {
                    new_recovery_pubkey: Some(f.owner_pubkey.clone()),
                    ..spec("laptop-2", &k)
                }
            }),
            |e| {
                matches!(e, RecoveryError::Recipient(R::RecipientExists { device, pubkey: Some(_) })
                    if device == OWNER)
            },
            Code::Conflict,
        ),
        (
            "--replaces naming nobody",
            Box::new({
                let k = stranger.clone();
                move |_| RecoverSpec {
                    replaces: Some("desktop-9".into()),
                    ..spec("laptop-2", &k)
                }
            }),
            |e| {
                matches!(e, RecoveryError::Recipient(R::RecipientNotFound { query })
                    if query == "desktop-9")
            },
            Code::NotFound,
        ),
        (
            "--replaces naming the recovery key",
            Box::new({
                let k = stranger.clone();
                move |_| RecoverSpec {
                    replaces: Some(RECOVERY_DEVICE_LABEL.into()),
                    ..spec("laptop-2", &k)
                }
            }),
            |e| matches!(e, RecoveryError::ReplacesRecoveryKey),
            Code::Usage,
        ),
    ];
    for (name, make, check, code) in cases {
        let f = Recover::new();
        f.insert("first");
        let (head, labels) = (f.head(), f.labels());

        let err = f.recover(&make(&f), &FakePrompter::new()).unwrap_err();

        assert!(check(&err), "{name}: {err:?}");
        assert_eq!(err.code(), code, "{name}");
        assert_wrote_nothing(&f, &head, &labels, name);
        assert_never_quotes(&err, &f.key().secret);
    }
}

// Go: TestRecoverDeviceReusingTheReplacedName. Rebuilding a machine under
// the name it always had is the ordinary case, so collisions are checked
// against the final list.
#[test]
fn recover_device_reusing_the_replaced_name() {
    let f = Recover::new();
    f.insert("first");
    let fresh = fresh_pubkey();

    f.recover(
        &RecoverSpec {
            replaces: Some(OWNER.into()),
            ..spec(OWNER, &fresh)
        },
        &FakePrompter::new(),
    )
    .expect("reusing the replaced device's name was refused");

    assert_eq!(f.labels(), [format!("{OWNER}={fresh}")]);
}

// Go: TestRecoverDeviceRefusesToReAddTheRetiredKey. The swap removes the
// recovery label before it checks the additions, so without an explicit
// check the retired key sails back in under another label and
// "retirement" reports success while the pasted secret stays a recipient.
#[test]
fn recover_device_refuses_to_re_add_the_retired_key() {
    type Make = fn(&Recover) -> RecoverSpec;
    let cases: [(&str, Make, &str); 2] = [
        // The worst shape: a bare, passphrase-less secret made a permanent
        // device recipient.
        (
            "as the enrolled device's key",
            |f| spec("laptop-2", &f.key().pubkey),
            "laptop-2",
        ),
        // Retired and re-seated under its own label: nothing changed.
        (
            "as its own replacement",
            |f| RecoverSpec {
                new_recovery_pubkey: Some(f.key().pubkey.clone()),
                ..spec("laptop-2", &fresh_pubkey())
            },
            RECOVERY_DEVICE_LABEL,
        ),
    ];
    for (name, make, as_role) in cases {
        let f = Recover::new();
        f.insert("first");
        let (head, labels) = (f.head(), f.labels());

        let err = f.recover(&make(&f), &FakePrompter::new()).unwrap_err();

        assert!(
            matches!(&err, RecoveryError::RetiredKeyReused { pubkey, as_device }
                if *pubkey == f.key().pubkey && as_device == as_role),
            "{name}: {err:?}"
        );
        assert_eq!(err.code(), Code::Conflict, "{name}");
        assert_wrote_nothing(&f, &head, &labels, name);
        // Not a silent partial: still the vault's recovery key, so a
        // corrected retry works.
        assert!(f.decrypts_with(&f.key().secret), "{name}");
        assert_never_quotes(&err, &f.key().secret);
    }
}

// Go: TestRecoverDeviceStillAsksM10sQuestion. The recovery path is not a
// way around the trust cache: a list nobody here reviewed is still shown.
#[test]
fn recover_device_still_asks_m10s_question() {
    let f = Recover::new();
    f.insert("first");
    crate::trustcache_tests::commit_routine_change(&f.vault, "phone-1", &fresh_pubkey());
    let (head, labels) = (f.head(), f.labels());
    let p = FakePrompter::new().with_recipient_answers(&[false]);

    let err = f
        .recover(&spec("laptop-2", &fresh_pubkey()), &p)
        .unwrap_err();

    assert_eq!(
        p.recipient_changes().len(),
        1,
        "M10's question was not asked"
    );
    assert!(
        matches!(
            err,
            RecoveryError::Recipient(RecipientError::Trust(TrustError::ChangeDeclined { .. }))
        ),
        "{err:?}"
    );
    assert_wrote_nothing(&f, &head, &labels, "declined");
}

// Go: TestApplySwapRefusesLeavingNothingToEncryptTo. Every caller adds at
// least one recipient today, so this drives the pure function itself.
#[test]
fn apply_swap_refuses_leaving_nothing_to_encrypt_to() {
    let current = [Recipient {
        device: OWNER.into(),
        pubkey: fresh_pubkey(),
    }];

    let err = apply_swap(
        &current,
        &RecipientSwap {
            remove: vec![OWNER.into()],
            ..RecipientSwap::default()
        },
    )
    .unwrap_err();

    assert!(
        matches!(
            &err,
            RecoveryError::Recipient(RecipientError::LastRecipient { .. })
        ),
        "{err:?}"
    );
    assert_eq!(err.code(), Code::Conflict);
}

// --- task 3: the secret's lifetime --------------------------------------------

/// `recovery/`'s non-test source, by file name.
fn recovery_sources() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/recovery");
    let mut files: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "rs") && !p.to_string_lossy().ends_with("_tests.rs")
        })
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            (name, std::fs::read_to_string(&p).unwrap())
        })
        .collect();
    files.sort();
    assert!(
        files.iter().any(|(n, _)| n == "swap.rs"),
        "the walk found no swap.rs; it is not looking where it thinks it is"
    );
    files
}

/// The body of `fn name`, comment lines dropped, by brace matching from
/// its signature.
fn fn_body(code: &str, name: &str) -> String {
    let start = code
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("no fn {name}"));
    let open = start + code[start..].find('{').unwrap();
    let mut depth = 0;
    let mut end = open;
    for (i, c) in code[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = open + i;
                    break;
                }
            }
            _ => {}
        }
    }
    // The signature too: the parameter's declaration is one of the uses.
    code[start..=end]
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The lines of `body` that mention `ident` as a whole identifier, trimmed.
fn lines_using(body: &str, ident: &str) -> Vec<String> {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    body.lines()
        .filter(|line| {
            line.match_indices(ident).any(|(i, _)| {
                !line[..i].chars().next_back().is_some_and(is_ident)
                    && !line[i + ident.len()..].chars().next().is_some_and(is_ident)
            })
        })
        .map(|l| l.trim().to_string())
        .collect()
}

// Go: TestRecoverDeviceZeroesTheSecretItWasHanded, ported per decision 2.
// Go zeroes the caller's buffer because it can't take it; here ownership
// moves in, and the buffer is dropped (zeroed) once parsed, on every path.
// What is left to prove is that nothing copies it on the way.
#[test]
fn recover_device_takes_the_secret_by_value() {
    // The signature: a `Zeroizing<String>`, by value. A `&str` or `&mut`
    // here would hand the zeroing back to the caller.
    let _: fn(&Vault, &RecoverSpec, Zeroizing<String>, &dyn Prompter) -> _ = Vault::recover_device;

    let sources = recovery_sources();
    let swap = &sources.iter().find(|(n, _)| n == "swap.rs").unwrap().1;

    // Every use of the secret, whitelisted: declared, moved on, parsed by
    // KeyLine::parse into page-lockable memory, dropped.
    assert_eq!(
        lines_using(&fn_body(swap, "recover_device"), "secret"),
        [
            "secret: Zeroizing<String>,",
            "let identity = self.recovery_identity(secret, prompter)?;",
        ]
    );
    assert_eq!(
        lines_using(&fn_body(swap, "recovery_identity"), "secret"),
        [
            "secret: Zeroizing<String>,",
            "let key = KeyLine::parse(secret.trim().as_bytes())",
            "drop(secret);",
        ]
    );

    // And nowhere in recovery/ is a secret copied into a plain String.
    for (name, code) in &sources {
        for banned in [
            "secret.clone()",
            "secret.to_string()",
            "secret.to_owned()",
            "String::from(secret",
            "secret.as_str().to_",
        ] {
            assert!(!code.contains(banned), "{name} copies a secret: {banned}");
        }
    }
}

// --- task 4: atomicity ---------------------------------------------------------

/// Asserts the state an interrupted swap must leave: HEAD, both recipient
/// files and the recovery key exactly as committed before it, and then the
/// next write's dirty-tree reset ending on a clean tree.
#[track_caller]
pub(super) fn assert_interrupted_swap_invisible(
    f: &Recover,
    head: &str,
    labels: &[String],
    file: &[String],
    what: &str,
) {
    assert_eq!(f.head(), head, "{what}: an interrupted swap moved HEAD");
    // The working tree may hold the new list's ciphertext (the reset
    // discards it); what HEAD carries is the guarantee, so read it there.
    let committed = |rel: &str| {
        let repo = git2::Repository::open(&f.vault.path).unwrap();
        let tree = repo.head().unwrap().peel_to_tree().unwrap();
        let blob = tree.get_path(std::path::Path::new(rel)).unwrap();
        String::from_utf8(repo.find_blob(blob.id()).unwrap().content().to_vec()).unwrap()
    };
    for key in file {
        assert!(
            committed(".age-recipients").contains(key.as_str()),
            "{what}"
        );
    }
    let config = committed(".gage/config.toml");
    for pair in labels {
        let (device, key) = pair.split_once('=').unwrap();
        assert!(config.contains(device) && config.contains(key), "{what}");
    }

    // The next write resets the leftovers, and the lock the crash unwound
    // through was released, or this would time out.
    let p = FakePrompter::new();
    f.vault
        .insert(&f.unlock(), sample("after the crash"), true, &p)
        .unwrap();
    assert!(f.is_clean(), "{what}: the reset left a dirty tree");
    assert!(
        p.warnings()
            .iter()
            .any(|w| w.contains("discarded uncommitted")),
        "{what}: {:?}",
        p.warnings()
    );
    assert_eq!(f.labels(), labels, "{what}");
    assert_eq!(f.recipients_file(), file, "{what}");
}

// Go: TestInterruptedRecoverDeviceLeavesHEADUntouched. A crash partway
// through the re-encryption commits nothing and the old recovery key still
// works, so re-running is the whole recovery procedure. At the first entry
// and at the last; the window after the last entry and before the commit
// (the recipient files written) is the shared commit_recipient_list's, and
// M9b's interrupted_recipient_write_leaves_no_half_migrated_state proves it.
#[test]
fn interrupted_recover_device_leaves_head_untouched() {
    for crash_after in [1, 3] {
        let what = format!("crash after entry {crash_after}");
        let f = Recover::new();
        for title in ["first", "second", "third"] {
            f.insert(title);
        }
        let (head, labels, file) = (f.head(), f.labels(), f.recipients_file());
        let device = fresh_pubkey();
        let crashing = crate::reencrypt_tests::crashing_after(&f.vault, crash_after);

        let died = crate::reencrypt_tests::crashed(|| {
            let _ = crashing.recover_device(
                &spec("laptop-2", &device),
                f.pasted(),
                &FakePrompter::new(),
            );
        });

        assert!(
            died,
            "{what}: the crash never fired, so this asserts nothing"
        );
        assert!(
            !f.is_clean(),
            "{what}: nothing was written before the crash"
        );
        assert_eq!(
            f.vault.verify_recovery_key(&f.key().secret).unwrap(),
            RECOVERY_DEVICE_LABEL,
            "{what}"
        );
        assert!(!f.devices().contains(&"laptop-2".to_string()), "{what}");
        assert_interrupted_swap_invisible(&f, &head, &labels, &file, &what);

        // And the retry is the whole recovery.
        f.recover(&spec("laptop-2", &device), &FakePrompter::new())
            .unwrap_or_else(|e| panic!("{what}: the retry failed: {e}"));
        assert!(f.devices().contains(&"laptop-2".to_string()), "{what}");
    }
}

// --- task 5: rotate_recovery_key --------------------------------------------

impl Recover {
    /// Rotates as the owner, through an ordinary unlock (Go's `rotateAs`).
    pub(super) fn rotate(
        &self,
        new_pubkey: &str,
        p: &FakePrompter,
    ) -> Result<RotateResult, RecoveryError> {
        self.vault
            .rotate_recovery_key(new_pubkey, &self.unlock(), p)
    }
}

// Go: TestRotateRecoveryKeySwapsInOneCommit. The old key stops opening new
// ciphertext and the new one opens everything.
#[test]
fn rotate_recovery_key_swaps_in_one_commit() {
    let f = Recover::new();
    f.insert("first");
    f.insert("second");
    let next = new_recovery_key().unwrap();
    let old = f.key().secret.to_string();
    let (before, commits) = (f.head(), f.commits());

    let res = f.rotate(&next.pubkey, &FakePrompter::new()).unwrap();

    assert_eq!(
        f.labels(),
        [
            format!("{OWNER}={}", f.owner_pubkey),
            format!("{RECOVERY_DEVICE_LABEL}={}", next.pubkey),
        ]
    );
    assert_eq!(
        f.recipients_file(),
        [f.owner_pubkey.clone(), next.pubkey.clone()]
    );
    assert_eq!(res.retired.as_deref(), Some(f.key().pubkey.as_str()));
    assert!(!res.created);
    assert_eq!(res.pubkey, next.pubkey);
    assert_ne!(f.head(), before);
    assert_eq!(res.commit, f.head());
    assert_eq!(f.commits(), commits + 1, "the swap must be one commit");
    assert_eq!(res.reencrypted, 2);
    assert_eq!(f.head_message(), "gage: recovery rotate (reencrypt)");
    assert!(
        !f.decrypts_with(&old),
        "the retired key still opens entries"
    );
    assert!(f.decrypts_with(&next.secret));
    assert!(f.is_clean());
}

// Go: TestRotateRecoveryKeyCreatesOneWhenThereIsNone. How a vault made
// with --no-recovery-key gets one later.
#[test]
fn rotate_recovery_key_creates_one_when_there_is_none() {
    let f = Recover::without_recovery_key();
    f.insert("first");
    let next = new_recovery_key().unwrap();

    let res = f.rotate(&next.pubkey, &FakePrompter::new()).unwrap();

    assert!(res.created);
    assert_eq!(res.retired, None);
    assert_eq!(
        f.labels(),
        [
            format!("{OWNER}={}", f.owner_pubkey),
            format!("{RECOVERY_DEVICE_LABEL}={}", next.pubkey),
        ]
    );
    assert!(f.decrypts_with(&next.secret));
}

// Go: TestRotateRecoveryKeyRefusesTheKeyItIsRetiring. The swap removes the
// old label before it checks the additions, so re-offering the retired key
// would sail through and report a rotation that changed nothing.
#[test]
fn rotate_recovery_key_refuses_the_key_it_is_retiring() {
    let f = Recover::new();
    f.insert("first");
    let (head, labels) = (f.head(), f.labels());

    let err = f.rotate(&f.key().pubkey, &FakePrompter::new()).unwrap_err();

    assert!(
        matches!(&err, RecoveryError::RetiredKeyReused { pubkey, as_device }
            if *pubkey == f.key().pubkey && as_device == RECOVERY_DEVICE_LABEL),
        "{err:?}"
    );
    assert_eq!(err.code(), Code::Conflict);
    assert_wrote_nothing(&f, &head, &labels, "retired key");
}

// Go: TestRotateRecoveryKeyRefusalsWriteNothing.
#[test]
fn rotate_recovery_key_refusals_write_nothing() {
    type Check = fn(&RecoveryError) -> bool;
    type Key = fn(&Recover) -> String;
    let cases: [(&str, Key, Check, Code); 2] = [
        (
            "a malformed key",
            |_| "age1nope".into(),
            |e| {
                matches!(
                    e,
                    RecoveryError::Recipient(RecipientError::InvalidRecipient(_))
                )
            },
            Code::Usage,
        ),
        (
            "a key that is a device's",
            |f| f.owner_pubkey.clone(),
            |e| {
                matches!(e, RecoveryError::Recipient(RecipientError::RecipientExists {
                    device, pubkey: Some(_) }) if device == OWNER)
            },
            Code::Conflict,
        ),
    ];
    for (name, key, check, code) in cases {
        let f = Recover::new();
        f.insert("first");
        let (head, labels) = (f.head(), f.labels());

        let err = f.rotate(&key(&f), &FakePrompter::new()).unwrap_err();

        assert!(check(&err), "{name}: {err:?}");
        assert_eq!(err.code(), code, "{name}");
        assert_wrote_nothing(&f, &head, &labels, name);
    }
}

// Go: TestInterruptedRotateRecoveryKeyLeavesHEADUntouched. The recovery
// key stays the vault's recovery recipient, so a retry is the whole
// procedure. Checked against the committed list, not by decrypting
// entries off disk: an interrupted write leaves entries/ holding the new
// list's ciphertext until the next write's reset, and HEAD carries the
// guarantee.
#[test]
fn interrupted_rotate_recovery_key_leaves_head_untouched() {
    for crash_after in [1, 3] {
        let what = format!("crash after entry {crash_after}");
        let f = Recover::new();
        for title in ["first", "second", "third"] {
            f.insert(title);
        }
        let (head, labels, file) = (f.head(), f.labels(), f.recipients_file());
        let next = fresh_pubkey();
        let crashing = crate::reencrypt_tests::crashing_after(&f.vault, crash_after);
        let identity = f.unlock();

        let died = crate::reencrypt_tests::crashed(|| {
            let _ = crashing.rotate_recovery_key(&next, &identity, &FakePrompter::new());
        });

        assert!(
            died,
            "{what}: the crash never fired, so this asserts nothing"
        );
        assert!(
            !f.is_clean(),
            "{what}: nothing was written before the crash"
        );
        assert_eq!(
            f.vault.verify_recovery_key(&f.key().secret).unwrap(),
            RECOVERY_DEVICE_LABEL,
            "{what}"
        );
        assert_interrupted_swap_invisible(&f, &head, &labels, &file, &what);
        assert!(f.decrypts_with(&f.key().secret), "{what}");
    }
}

// Go: TestRotateRecoveryKeyStillAsksM10sQuestion.
#[test]
fn rotate_recovery_key_still_asks_m10s_question() {
    let f = Recover::new();
    f.insert("first");
    crate::trustcache_tests::commit_routine_change(&f.vault, "phone-1", &fresh_pubkey());
    let (head, labels) = (f.head(), f.labels());
    let p = FakePrompter::new().with_recipient_answers(&[false]);

    let err = f.rotate(&fresh_pubkey(), &p).unwrap_err();

    assert_eq!(
        p.recipient_changes().len(),
        1,
        "M10's question was not asked"
    );
    assert!(
        matches!(
            err,
            RecoveryError::Recipient(RecipientError::Trust(TrustError::ChangeDeclined { .. }))
        ),
        "{err:?}"
    );
    assert_wrote_nothing(&f, &head, &labels, "declined");
}

// Go: TestRotateRefusesToEvictTheActingDevice. The backstop under the
// label reservation: even if something ever let a device hold the label,
// rotate must not be what locks out the machine running it. No reserved
// path reaches this state, so it is written by hand and noted reviewed, as
// Go's test constructs it directly.
#[test]
fn rotate_refuses_to_evict_the_acting_device() {
    let f = Recover::without_recovery_key();
    f.vault
        .write_recipient_files(&[
            Recipient {
                device: OWNER.into(),
                pubkey: fresh_pubkey(),
            },
            Recipient {
                device: RECOVERY_DEVICE_LABEL.into(),
                pubkey: f.owner_pubkey.clone(),
            },
        ])
        .unwrap();
    gitrepo::commit_all(&f.vault.path, "test: the owner's key under the label").unwrap();
    f.vault.note_recipients_reviewed().unwrap();
    f.insert("first");
    let (head, labels) = (f.head(), f.labels());

    let err = f.rotate(&fresh_pubkey(), &FakePrompter::new()).unwrap_err();

    assert!(
        matches!(&err, RecoveryError::WouldEvictActingDevice { vault } if vault == NAME),
        "{err:?}"
    );
    assert_eq!(err.code(), Code::Conflict);
    assert_wrote_nothing(&f, &head, &labels, "acting device");
}

// Go: TestRotateWarnsThatRetirementIsFutureOnly. The leak-response user
// most needs to hear that the retired key still opens history, and rotate
// retires through remove_if_present.
#[test]
fn rotate_warns_that_retirement_is_future_only() {
    let f = Recover::new();
    f.insert("first");
    let p = FakePrompter::new();

    f.rotate(&fresh_pubkey(), &p).unwrap();

    let warned: Vec<String> = p
        .warnings()
        .into_iter()
        .filter(|w| w.contains("git history"))
        .collect();
    assert_eq!(warned.len(), 1, "{:?}", p.warnings());
    assert!(
        warned[0].contains(&format!("{RECOVERY_DEVICE_LABEL:?}")),
        "{warned:?}"
    );

    // Nothing to retire, nothing to warn about.
    let g = Recover::without_recovery_key();
    g.insert("first");
    let p = FakePrompter::new();
    g.rotate(&fresh_pubkey(), &p).unwrap();
    assert!(
        !p.warnings().iter().any(|w| w.contains("git history")),
        "{:?}",
        p.warnings()
    );
}

// --- task 6: found by the review -------------------------------------------

// Beyond Go's rows, found by the review's mutation pass (task 6): deleting
// the swap's require_full_access left every test green. A19 holds for both
// swaps. An actor who can't read the whole vault has nothing whole to
// grant, and the refusal comes before any question or byte, never midway
// through on an opaque entry id.
#[test]
fn both_swaps_refuse_an_actor_who_cannot_read_every_entry() {
    /// Rewrites one entry so only `to` can open it, and commits it: what a
    /// partial add from before A19 would have left.
    fn readable_only_by(f: &Recover, to: &str) {
        let id = f.vault.entry_ids().unwrap()[0];
        let entry = f.vault.read_entry(id, &f.unlock()).unwrap();
        let to = crate::crypt::Recipient::parse(to).unwrap();
        f.vault.write_entry_to(id, &entry, &[to]).unwrap();
        gitrepo::commit_all(&f.vault.path, "test: an entry only one key opens").unwrap();
    }

    // recover_device: an entry the recovery key can't open.
    let f = Recover::new();
    f.insert("first");
    f.insert("second");
    readable_only_by(&f, &f.owner_pubkey.clone());
    let (head, labels) = (f.head(), f.labels());
    let p = FakePrompter::new();
    let err = f
        .recover(&spec("laptop-2", &fresh_pubkey()), &p)
        .unwrap_err();
    assert!(
        matches!(&err, RecoveryError::Recipient(RecipientError::CannotGrantFullAccess {
            unreadable: 1, total: 2, device, .. }) if device == RECOVERY_DEVICE_LABEL),
        "{err:?}"
    );
    assert_eq!(err.code(), Code::Conflict);
    assert_wrote_nothing(&f, &head, &labels, "recover");
    assert!(
        p.warnings().iter().all(|w| !w.contains("git history")),
        "warned before refusing"
    );

    // rotate_recovery_key: an entry the acting device can't open.
    let f = Recover::new();
    f.insert("first");
    f.insert("second");
    readable_only_by(&f, &f.key().pubkey.clone());
    let (head, labels) = (f.head(), f.labels());
    let err = f.rotate(&fresh_pubkey(), &FakePrompter::new()).unwrap_err();
    assert!(
        matches!(&err, RecoveryError::Recipient(RecipientError::CannotGrantFullAccess {
            unreadable: 1, device, .. }) if device == OWNER),
        "{err:?}"
    );
    assert_wrote_nothing(&f, &head, &labels, "rotate");
}
