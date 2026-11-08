//! Recipient writes (`recipient add` and `remove`), as library tests.
//!
//! [`Fixture`] is shared with `reencrypt_tests`: one local vault and the
//! key that created it. A second device is just another key here, the full
//! two-install fixture (`twodevice_tests`) is used where a second
//! machine's own identity file matters.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::crypto::identity::Identity;
use crate::crypto::vaultkey::VaultKey;
use crate::entry::{Entry, NewEntry};
use crate::error::{ErrorKind, HasKind};
use crate::prompter::FakePrompter;
use crate::storage::vaultconfig;
use crate::storage::xdgpaths::Dirs;
use crate::sync::gitrepo;
use crate::sync::remotesync::tests::{Holder, SyncVault};
use crate::testsupport::fast_scrypt;
use crate::trust::recipient;
use crate::trust::recipient::RecipientError;
use crate::vault::reencrypt_tests::{crashed, crashing_after};
use crate::vault::twodevice_tests::{B_DEVICE, Machine};
use crate::vault::{CreateSpec, Vault};

pub(crate) const DEVICE: &str = "laptop-1";

/// A local vault and the key that can read it.
pub(crate) struct Fixture {
    pub(crate) _root: tempfile::TempDir,
    pub(crate) vault: Vault,
    pub(crate) key: age::x25519::Identity,
}

pub(crate) fn dirs_in(root: &Path) -> Dirs {
    Dirs {
        config: root.join("config"),
        data: root.join("data"),
        state: root.join("state"),
    }
}

impl Fixture {
    pub(crate) fn new() -> Fixture {
        fast_scrypt();
        let root = tempfile::tempdir().unwrap();
        let key = age::x25519::Identity::generate();
        let vault = Vault::create(
            &dirs_in(root.path()),
            &CreateSpec {
                name: "personal".into(),
                id: vaultconfig::VaultId::new(),
                path: root.path().join("vault"),
                vault_type: crate::storage::vaultconfig::VaultType::Git,
                method: "passphrase".into(),
                device: DEVICE.into(),
                recipients: vec![key.to_public().to_string()],
                extra_recipients: Vec::new(),
                remote: None,
            },
        )
        .unwrap();
        Fixture {
            _root: root,
            vault,
            key,
        }
    }

    pub(crate) fn identity(&self) -> Identity {
        Identity::for_tests(&self.key)
    }

    pub(crate) fn pubkey(&self) -> String {
        self.key.to_public().to_string()
    }

    /// Inserts `n` entries titled `entry-a`, `entry-b`, … and returns their
    /// ids in insertion order.
    pub(crate) fn seed(&self, n: usize) -> Vec<Uuid> {
        (0..n)
            .map(|i| {
                self.vault
                    .insert(
                        &self.identity(),
                        NewEntry {
                            title: format!("entry-{}", (b'a' + i as u8) as char),
                            value: format!("secret-{i}"),
                            ..NewEntry::default()
                        },
                        true,
                        &FakePrompter::new(),
                    )
                    .unwrap()
            })
            .collect()
    }

    /// Overwrites `id`'s file with bytes nobody committed, the way an
    /// interrupted re-encryption leaves one, and returns its vault-relative
    /// path.
    pub(crate) fn dirty_entry(&self, id: Uuid) -> String {
        let rel = format!("entries/{id}.age");
        std::fs::write(self.vault.path.join(&rel), b"half-written ciphertext").unwrap();
        rel
    }

    pub(crate) fn commits(&self) -> usize {
        gitrepo::commit_count(&self.vault.path).unwrap()
    }

    pub(crate) fn head(&self) -> String {
        head_hash(&self.vault.path)
    }

    pub(crate) fn is_clean(&self) -> bool {
        gitrepo::is_clean(&self.vault.path).unwrap()
    }

    /// `.age-recipients`, read as itself.
    pub(crate) fn recipients_file(&self) -> Vec<String> {
        recipient::load(&self.vault.path.join(".age-recipients")).unwrap()
    }

    /// `.gage/config.toml`'s keys, in order.
    pub(crate) fn config_keys(&self) -> Vec<String> {
        self.vault
            .recipients()
            .unwrap()
            .into_iter()
            .map(|r| r.pubkey)
            .collect()
    }

    pub(crate) fn read(&self, id: Uuid, identity: &Identity) -> Result<Entry, crate::entry::Error> {
        self.vault.read_entry(id, identity)
    }
}

pub(crate) fn head_hash(dir: &Path) -> String {
    let repo = git2::Repository::open(dir).unwrap();
    repo.head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
        .to_string()
}

/// The vault-relative paths `hash` changed against its first parent: the
/// only way to assert "exactly one commit, holding both files" rather than
/// merely "one new commit".
pub(crate) fn commit_paths(dir: &Path, hash: &str) -> Vec<String> {
    let repo = git2::Repository::open(dir).unwrap();
    let commit = repo
        .find_commit(git2::Oid::from_str(hash).unwrap())
        .unwrap();
    let tree = commit.tree().unwrap();
    let parent = commit.parent(0).ok().map(|p| p.tree().unwrap());
    let diff = repo
        .diff_tree_to_tree(parent.as_ref(), Some(&tree), None)
        .unwrap();
    let mut paths: Vec<String> = diff
        .deltas()
        .map(|d| {
            d.new_file()
                .path()
                .or_else(|| d.old_file().path())
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/")
        })
        .collect();
    paths.sort();
    paths
}

/// A fresh real key: what another device's `identity add` prints.
pub(crate) fn new_key() -> age::x25519::Identity {
    age::x25519::Identity::generate()
}

// --- shared setup for the full-access refusal -------------------------------------

/// Rewrites both recipient files from `list` and commits them, as a hand
/// edit followed by `git commit` would. Through the real
/// `write_recipient_files`, so the two can't be left disagreeing.
pub(crate) fn commit_recipient_files(vault: &Vault, list: &[vaultconfig::Recipient]) {
    vault.write_recipient_files(list).unwrap();
    gitrepo::commit_all(&vault.path, "test: hand-edited recipient list").unwrap();
}

/// Leaves the vault holding one entry this fixture's key can't read, while
/// both recipient files still agree and still list it: the state a
/// recipient admitted before the full-access rule (ADR-0009) lives in, built by the only other route
/// into it. Returns the hidden entry's id.
pub(crate) fn hide_one_entry(f: &Fixture) -> Uuid {
    let original = f.vault.recipients().unwrap();
    commit_recipient_files(
        &f.vault,
        &[vaultconfig::Recipient {
            device: "foreign".into(),
            pubkey: new_key().to_public().to_string(),
        }],
    );
    // force, so the duplicate-title check doesn't decrypt anything. The
    // list just changed under this device, so the insert asks, and is
    // told yes.
    let hidden = f
        .vault
        .insert(
            &f.identity(),
            NewEntry {
                title: "hidden".into(),
                value: "v".into(),
                ..NewEntry::default()
            },
            true,
            &FakePrompter::new().with_recipient_answers(&[true]),
        )
        .unwrap();
    commit_recipient_files(&f.vault, &original);
    hidden
}

/// A counter the re-encryption hook bumps: how many entries were written.
pub(crate) fn counting(vault: &Vault) -> (Vault, Arc<AtomicUsize>) {
    let n = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&n);
    let v = vault
        .clone()
        .with_reencrypt_hook(move |done| seen.store(done, Ordering::SeqCst));
    (v, n)
}

/// Everything a refused write must have left alone.
pub(crate) struct Untouched {
    head: String,
    commits: usize,
    recipients_file: Vec<String>,
    config_keys: Vec<String>,
}

impl Untouched {
    pub(crate) fn of(f: &Fixture) -> Untouched {
        Untouched {
            head: f.head(),
            commits: f.commits(),
            recipients_file: f.recipients_file(),
            config_keys: f.config_keys(),
        }
    }

    pub(crate) fn assert_still(&self, f: &Fixture) {
        assert_eq!(f.head(), self.head, "HEAD moved");
        assert_eq!(f.commits(), self.commits, "something was committed");
        assert_eq!(f.recipients_file(), self.recipients_file);
        assert_eq!(f.config_keys(), self.config_keys);
    }
}

// --- require_full_access and add_recipient ------------------------

// The shape enrollment approval depends on: a
// pass a caller places itself.
#[test]
fn require_full_access_is_callable_on_its_own() {
    let f = Fixture::new();
    f.seed(2);
    f.vault.require_full_access(&f.identity()).unwrap();

    hide_one_entry(&f);
    let err = f.vault.require_full_access(&f.identity()).unwrap_err();
    assert!(
        matches!(
            err,
            RecipientError::CannotGrantFullAccess {
                unreadable: 1,
                total: 3,
                ..
            }
        ),
        "{err:?}"
    );
}

// A corrupt entry is not a lack of access: the pass counts only entries
// the identity isn't a recipient of, and leaves naming a damaged one to the
// re-encryption, which says which.
#[test]
fn require_full_access_does_not_count_a_corrupt_entry() {
    let f = Fixture::new();
    let ids = f.seed(2);
    std::fs::write(
        f.vault.path.join(format!("entries/{}.age", ids[0])),
        b"junk",
    )
    .unwrap();
    f.vault.require_full_access(&f.identity()).unwrap();
}

// The count is
// the diagnosis; the id is not, and nothing changes.
#[test]
fn recipient_add_refuses_an_actor_that_cannot_read_every_entry() {
    let f = Fixture::new();
    f.seed(2);
    let hidden = hide_one_entry(&f);
    let before = Untouched::of(&f);
    let (vault, rewritten) = counting(&f.vault);
    let phone = new_key().to_public().to_string();

    let p = FakePrompter::new();
    let err = vault
        .add_recipient("phone-1", &phone, &f.identity(), &p)
        .unwrap_err();

    assert!(
        matches!(err, RecipientError::CannotGrantFullAccess { .. }),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    let msg = err.to_string();
    assert!(msg.contains("1 of 3"), "{msg}");
    assert!(msg.contains("personal"), "{msg}");
    assert!(!msg.contains(&hidden.to_string()), "{msg}");
    assert_eq!(rewritten.load(Ordering::SeqCst), 0);
    assert!(p.warnings().is_empty() && p.confirm_prompts().is_empty());
    before.assert_still(&f);
    assert!(f.is_clean());
}

// Another process holds
// the lock for the whole call: an add that reached it would wait out the
// timeout and fail as contention, so a prompt refusal proves the pass ran
// outside it.
#[test]
fn full_access_preflight_runs_before_the_vault_lock() {
    let f = Fixture::new();
    f.seed(1);
    hide_one_entry(&f);
    let lock = f.vault.lock_file_path();
    std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
    let _holder = Holder::spawn(&lock);

    let start = Instant::now();
    let err = f
        .vault
        .add_recipient(
            "phone-1",
            &new_key().to_public().to_string(),
            &f.identity(),
            &FakePrompter::new(),
        )
        .unwrap_err();

    assert!(
        matches!(err, RecipientError::CannotGrantFullAccess { .. }),
        "the add reached the lock before the pre-flight: {err:?}"
    );
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "it waited on the lock"
    );
}

// On a
// dirty tree the refusal moves past the reset, never away: the unreadable
// entry is committed, so the reset can't make it go.
#[test]
fn add_is_still_refused_on_a_dirty_tree_when_access_is_genuinely_partial() {
    let f = Fixture::new();
    f.seed(2);
    let hidden = hide_one_entry(&f);
    std::fs::write(f.vault.path.join("left-behind.txt"), "uncommitted\n").unwrap();
    assert!(!f.is_clean());
    let before = Untouched::of(&f);

    let p = FakePrompter::new();
    let err = f
        .vault
        .add_recipient(
            "phone-1",
            &new_key().to_public().to_string(),
            &f.identity(),
            &p,
        )
        .unwrap_err();

    assert!(
        matches!(err, RecipientError::CannotGrantFullAccess { .. }),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(err.to_string().contains("1 of 3"), "{err}");
    assert!(!err.to_string().contains(&hidden.to_string()), "{err}");
    assert!(p.confirm_prompts().is_empty());
    before.assert_still(&f);
}

// Checked first,
// ahead of the tree check and the lock.
#[test]
fn add_recipient_rejects_invalid_device_name_or_pubkey() {
    let f = Fixture::new();
    let before = Untouched::of(&f);
    let plugin = "age1yubikey1qqqsyqcyq5rqwzqfpg9scrgwpugpzysnzs23v9ccrydpk8qarc0s9hkmc0";
    for (device, key) in [
        ("not a valid name!", plugin),
        ("../escape", plugin),
        ("phone-1", "not-an-age-key"),
    ] {
        let err = f
            .vault
            .add_recipient(device, key, &f.identity(), &FakePrompter::new())
            .unwrap_err();
        assert!(
            matches!(
                err,
                RecipientError::InvalidDeviceName(_) | RecipientError::InvalidRecipient(_)
            ),
            "{device:?} {key:?}: {err:?}"
        );
        assert_eq!(err.kind(), ErrorKind::Usage, "{device:?} {key:?}");
    }
    before.assert_still(&f);
}

// Adding a duplicate key is rejected, and so is its device-name twin: a
// list naming one key (or one label) twice buys nothing and muddies verify.
#[test]
fn recipient_add_rejects_a_duplicate_key() {
    let f = Fixture::new();
    let before = Untouched::of(&f);

    let err = f
        .vault
        .add_recipient(
            "laptop-again",
            &f.pubkey(),
            &f.identity(),
            &FakePrompter::new(),
        )
        .unwrap_err();
    assert!(
        matches!(err, RecipientError::RecipientExists { .. }),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(err.to_string().contains(DEVICE), "{err}");

    let err = f
        .vault
        .add_recipient(
            DEVICE,
            &new_key().to_public().to_string(),
            &f.identity(),
            &FakePrompter::new(),
        )
        .unwrap_err();
    assert!(
        matches!(err, RecipientError::RecipientExists { .. }),
        "{err:?}"
    );
    before.assert_still(&f);
}

// One commit holds both
// recipient files and every re-encrypted entry.
#[test]
fn recipient_add_commits_both_files_together() {
    let f = Fixture::new();
    let ids = f.seed(3);
    let phone = new_key().to_public().to_string();
    let before = f.commits();

    let change = f
        .vault
        .add_recipient("phone-1", &phone, &f.identity(), &FakePrompter::new())
        .unwrap();

    assert_eq!(f.commits(), before + 1);
    assert_eq!(change.reencrypted, ids.len());
    assert_eq!(change.commit, f.head());
    assert_eq!(
        (change.device.as_str(), change.pubkey.as_str()),
        ("phone-1", phone.as_str())
    );
    let mut want: Vec<String> = ids.iter().map(|id| format!("entries/{id}.age")).collect();
    want.extend([
        ".age-recipients".into(),
        ".gage/config.toml".into(),
        ".gage/vault-key.age".into(),
    ]);
    want.sort();
    assert_eq!(commit_paths(&f.vault.path, &f.head()), want);
    let head = gitrepo::head_commit(&f.vault.path).unwrap();
    assert_eq!(head.message, "gage: recipient add phone-1 (reencrypt)");
    assert!(f.is_clean());
    assert!(f.recipients_file().contains(&phone));
    let added = f.vault.recipients().unwrap();
    assert_eq!(added.last().unwrap().device, "phone-1");
    assert_eq!(added.last().unwrap().pubkey, phone);
}

// Adding a recipient has no partial form (ADR-0009). Entries from before the add
// and after it are equally readable by the new key, proven by a real
// decrypt with it.
#[test]
fn recipient_add_has_no_partial_form() {
    let f = Fixture::new();
    let before = f.seed(1)[0];
    let phone = new_key();
    f.vault
        .add_recipient(
            "phone-1",
            &phone.to_public().to_string(),
            &f.identity(),
            &FakePrompter::new(),
        )
        .unwrap();
    let after = f.seed(1)[0];

    let as_phone = Identity::for_tests(&phone);
    f.read(after, &as_phone).unwrap();
    f.read(before, &as_phone)
        .expect("no add may leave a recipient reading only part of the vault");
}

// Adding a recipient makes history readable, on the two-device fixture:
// device B registers its own identity, A adds it, and B, after the pull
// on its own unlock, reads every entry A wrote before.
#[test]
fn recipient_add_makes_history_readable() {
    let a = SyncVault::new();
    let id_a = a.unlock();
    let mut titles = Vec::new();
    for title in ["ProtonMail", "Bank", "Router"] {
        a.vault
            .insert(
                &id_a,
                NewEntry {
                    title: title.into(),
                    value: format!("secret-{title}"),
                    ..NewEntry::default()
                },
                false,
                &FakePrompter::new(),
            )
            .unwrap();
        titles.push(title.to_string());
    }
    let b = Machine::cloned_from(&a, B_DEVICE);
    let b_key = b.identity_add(B_DEVICE);

    let p = FakePrompter::new();
    let change = a.vault.add_recipient(B_DEVICE, &b_key, &id_a, &p).unwrap();
    assert_eq!(change.reencrypted, 3);
    assert!(
        p.warnings().is_empty(),
        "the push failed: {:?}",
        p.warnings()
    );

    let id_b = b.unlock();
    let mut read: Vec<String> = b
        .vault
        .entry_ids()
        .unwrap()
        .into_iter()
        .map(|id| b.vault.read_entry(id, &id_b).unwrap().title)
        .collect();
    read.sort();
    titles.sort();
    assert_eq!(read, titles);
}

// --- remove_recipient ----------------------------------------------

/// The fixture with a second recipient, so removing either isn't the
/// last-recipient refusal.
fn with_phone(f: &Fixture) -> age::x25519::Identity {
    let phone = new_key();
    f.vault
        .add_recipient(
            "phone-1",
            &phone.to_public().to_string(),
            &f.identity(),
            &FakePrompter::new(),
        )
        .unwrap();
    phone
}

fn not_a_recipient(r: Result<Entry, crate::entry::Error>) -> bool {
    matches!(
        r,
        Err(crate::entry::Error::Crypt(
            crate::crypto::crypt::Error::NotARecipient
        ))
    )
}

// A rejection of the
// command line: before the lock and before anything is read or asked.
#[test]
fn recipient_remove_without_reencrypt_is_rejected() {
    let f = Fixture::new();
    let phone = with_phone(&f);
    let before = Untouched::of(&f);
    let lock = f.vault.lock_file_path();
    let _holder = Holder::spawn(&lock);

    let start = Instant::now();
    let err = f
        .vault
        .remove_recipient("phone-1", false, &f.identity(), &FakePrompter::new())
        .unwrap_err();

    assert!(matches!(err, RecipientError::ReencryptRequired), "{err:?}");
    assert_eq!(err.kind(), ErrorKind::Usage);
    assert!(err.to_string().contains("--reencrypt"), "{err}");
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "it waited on the lock"
    );
    before.assert_still(&f);
    assert!(f.recipients_file().contains(&phone.to_public().to_string()));
}

// Proven from
// the removed device's side: it read the entry before, and can't after.
#[test]
fn recipient_remove_with_reencrypt_excludes_the_removed_key() {
    let f = Fixture::new();
    let phone = with_phone(&f);
    let id = f.seed(1)[0];
    let as_phone = Identity::for_tests(&phone);
    f.read(id, &as_phone).unwrap();

    let change = f
        .vault
        .remove_recipient("phone-1", true, &f.identity(), &FakePrompter::new())
        .unwrap();

    assert_eq!(change.reencrypted, 1);
    assert_eq!(change.commit, f.head());
    assert_eq!(
        gitrepo::head_commit(&f.vault.path).unwrap().message,
        "gage: recipient remove phone-1 (reencrypt)"
    );
    f.read(id, &f.identity()).unwrap();
    let key = phone.to_public().to_string();
    assert!(!f.recipients_file().contains(&key));
    assert!(!f.config_keys().contains(&key));
    assert!(not_a_recipient(f.read(id, &as_phone)));
    assert!(f.is_clean());
}

// Through the
// prompter; the library never prints.
#[test]
fn recipient_remove_warns_that_revocation_is_future_only() {
    let f = Fixture::new();
    with_phone(&f);
    let p = FakePrompter::new();
    f.vault
        .remove_recipient("phone-1", true, &f.identity(), &p)
        .unwrap();
    let warned = p.warnings().join("\n");
    for want in ["future access", "already read", "phone-1"] {
        assert!(warned.contains(want), "{warned:?}");
    }
}

#[test]
fn recipient_remove_accepts_either_a_device_name_or_a_public_key() {
    for by_key in [false, true] {
        let f = Fixture::new();
        let key = with_phone(&f).to_public().to_string();
        let query = if by_key {
            key.clone()
        } else {
            "phone-1".into()
        };
        let change = f
            .vault
            .remove_recipient(&query, true, &f.identity(), &FakePrompter::new())
            .unwrap();
        assert_eq!(change.pubkey, key, "{query}");
        assert_eq!(change.device, "phone-1", "{query}");
        assert!(!f.recipients_file().contains(&key), "{query}");
    }
}

// A typo must not
// read as a completed revocation.
#[test]
fn recipient_remove_of_an_unknown_recipient_is_rejected() {
    let f = Fixture::new();
    with_phone(&f);
    let before = Untouched::of(&f);
    let err = f
        .vault
        .remove_recipient("no-such-device", true, &f.identity(), &FakePrompter::new())
        .unwrap_err();
    assert!(
        matches!(&err, RecipientError::RecipientNotFound { query } if query == "no-such-device"),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::NotFound);
    before.assert_still(&f);
}

// Refused outright, and
// before the self-removal question: the only recipient is this device, and
// an unscripted confirm would panic the fake, so a "yes" never meets it.
#[test]
fn recipient_remove_refuses_the_last_recipient() {
    let f = Fixture::new();
    f.seed(1);
    let before = Untouched::of(&f);
    let p = FakePrompter::new();
    let err = f
        .vault
        .remove_recipient(DEVICE, true, &f.identity(), &p)
        .unwrap_err();
    assert!(
        matches!(&err, RecipientError::LastRecipient { device } if device == DEVICE),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(p.confirm_prompts().is_empty());
    before.assert_still(&f);
    assert_eq!(f.recipients_file().len(), 1);
}

// Declined (which is
// also every non-interactive prompter's answer) writes nothing; confirmed
// works, because every entry is decrypted with the still-valid identity
// before the list shrinks.
#[test]
fn recipient_remove_of_this_device_needs_confirmation() {
    let f = Fixture::new();
    let phone = with_phone(&f);
    let id = f.seed(1)[0];
    let before = Untouched::of(&f);

    let declining = FakePrompter::new().with_confirm(false);
    let err = f
        .vault
        .remove_recipient(DEVICE, true, &f.identity(), &declining)
        .unwrap_err();
    assert!(
        matches!(err, RecipientError::SelfRemovalDeclined { .. }),
        "{err:?}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    let asked = declining.confirm_prompts();
    assert_eq!(asked.len(), 1);
    assert!(
        asked[0].contains(DEVICE) && asked[0].contains("personal"),
        "{asked:?}"
    );
    assert!(declining.warnings().is_empty(), "warned before the answer");
    before.assert_still(&f);

    // By key too: the identity's own key is this device whatever it's
    // called in the list.
    let accepting = FakePrompter::new().with_confirm(true);
    f.vault
        .remove_recipient(&f.pubkey(), true, &f.identity(), &accepting)
        .unwrap();
    assert!(!f.recipients_file().contains(&f.pubkey()));
    assert!(not_a_recipient(f.read(id, &f.identity())));
    f.read(id, &Identity::for_tests(&phone)).unwrap();
}

// An interrupted
// self-removal leaves entries/ unreadable by this device while HEAD lists
// it and is entirely readable: the pre-flight must not refuse over a tree
// the reset is about to discard.
#[test]
fn add_is_not_refused_over_a_tree_the_reset_discards() {
    let f = Fixture::new();
    f.seed(3);
    with_phone(&f);

    let v = crashing_after(&f.vault, 1);
    let p = FakePrompter::new().with_confirm(true);
    assert!(
        crashed(|| {
            let _ = v.remove_recipient(DEVICE, true, &f.identity(), &p);
        }),
        "the crash seam never fired"
    );
    assert!(!f.is_clean(), "nothing for the reset to discard");
    assert!(
        f.vault.require_full_access(&f.identity()).is_err(),
        "the interrupted tree isn't the shape under test"
    );

    let third = new_key().to_public().to_string();
    let change = f
        .vault
        .add_recipient("phone-2", &third, &f.identity(), &FakePrompter::new())
        .unwrap();
    assert_eq!(
        change.reencrypted, 3,
        "the entries at HEAD, not the discarded ones"
    );
    assert!(f.recipients_file().contains(&third));
    assert!(f.config_keys().contains(&third));
}

// --- the MAC key across membership changes (ADR-0066) ------------------

/// `e`, tagged under `key` and encrypted to the vault's recipients, as
/// entry `id`: what a holder of `key` can file, committed.
fn plant_under(f: &Fixture, id: Uuid, key: &VaultKey) {
    let e = Entry {
        title: "forged".into(),
        ..Entry::default()
    };
    let to = f.vault.encrypt_recipients().unwrap();
    let sealed = crate::entry::seal_entry(id, &e, &to, key).unwrap();
    std::fs::write(f.vault.entry_path(id), sealed).unwrap();
    gitrepo::commit_all(&f.vault.path, "test: a planted entry").unwrap();
}

fn unauthenticated(r: Result<Entry, crate::entry::Error>) -> bool {
    matches!(r, Err(crate::entry::Error::Unauthenticated { .. }))
}

#[test]
fn remove_rotates_so_the_removed_key_cannot_produce_a_valid_entry() {
    let f = Fixture::new();
    let ids = f.seed(3);
    let phone = with_phone(&f);
    let phone_id = Identity::for_tests(&phone);
    // What the phone walks away with.
    let its_copy = f.vault.vault_key(&phone_id).unwrap();

    f.vault
        .remove_recipient("phone-1", true, &f.identity(), &FakePrompter::new())
        .unwrap();

    // It can't open the key any more.
    assert!(matches!(
        f.vault.vault_key(&phone_id),
        Err(crate::entry::Error::VaultKey(e))
            if matches!(*e, crate::crypto::vaultkey::Error::Crypt {
                source: crate::crypto::crypt::Error::NotARecipient, .. })
    ));
    // An entry under the key it kept is refused by everyone who can read.
    let forged = Uuid::new_v4();
    plant_under(&f, forged, &its_copy);
    assert!(unauthenticated(f.read(forged, &f.identity())));
    std::fs::remove_file(f.vault.entry_path(forged)).unwrap();
    gitrepo::commit_all(&f.vault.path, "test: remove the planted entry").unwrap();
    // And the recipients that stayed read every entry.
    for id in ids {
        f.read(id, &f.identity()).unwrap();
    }
}

#[test]
fn add_reseals_the_same_key_for_the_new_recipient() {
    let f = Fixture::new();
    let ids = f.seed(2);
    let before = f.vault.vault_key(&f.identity()).unwrap();

    let phone = with_phone(&f);
    let phone_id = Identity::for_tests(&phone);

    let opened = f.vault.vault_key(&phone_id).unwrap();
    assert!(
        opened.continues(&before) && before.continues(&opened),
        "add re-seals, it doesn't rotate"
    );
    for id in ids {
        f.read(id, &phone_id).unwrap();
    }
}

// A key leaving is what rotates, wherever the change came from: the
// decision is made where the list is written.
#[test]
fn a_list_that_only_grows_does_not_rotate_and_one_that_shrinks_does() {
    let f = Fixture::new();
    f.seed(1);
    let before = f.vault.vault_key(&f.identity()).unwrap();
    with_phone(&f);
    let added = f.vault.vault_key(&f.identity()).unwrap();
    assert!(added.continues(&before) && before.continues(&added));

    f.vault
        .remove_recipient("phone-1", true, &f.identity(), &FakePrompter::new())
        .unwrap();
    let removed = f.vault.vault_key(&f.identity()).unwrap();
    assert!(removed.continues(&before), "the old key is retired");
    assert!(!before.continues(&removed), "and a new one is current");
}

#[test]
fn history_still_reads_every_revision_from_before_a_rotation() {
    let f = Fixture::new();
    let id = f.seed(1)[0];
    let p = FakePrompter::new();
    let mut edited = f.read(id, &f.identity()).unwrap();
    edited.value = "second".into();
    f.vault.update(&f.identity(), id, &edited, &p).unwrap();
    with_phone(&f);
    f.vault
        .remove_recipient("phone-1", true, &f.identity(), &p)
        .unwrap();
    // A second rotation, so more than one key is retired.
    with_phone(&f);
    f.vault
        .remove_recipient("phone-1", true, &f.identity(), &p)
        .unwrap();

    let revisions = f.vault.history(id, &f.identity()).unwrap();
    assert!(revisions.len() >= 3, "{}", revisions.len());
    assert!(revisions.iter().all(|r| r.entry.is_some()));
    let values: Vec<_> = revisions
        .iter()
        .map(|r| r.entry.as_ref().unwrap().value.clone())
        .collect();
    assert!(values.contains(&"second".to_string()), "{values:?}");
}

// A retired key reads history and nothing else.
#[test]
fn a_live_entry_under_a_retired_key_is_refused_though_history_accepts_it() {
    let f = Fixture::new();
    f.seed(1);
    let old = f.vault.vault_key(&f.identity()).unwrap();
    with_phone(&f);
    f.vault
        .remove_recipient("phone-1", true, &f.identity(), &FakePrompter::new())
        .unwrap();
    let id = Uuid::new_v4();
    plant_under(&f, id, &old);
    assert!(unauthenticated(f.read(id, &f.identity())));
    assert!(f.vault.history(id, &f.identity()).is_ok());
}

#[test]
fn a_crash_during_rotation_leaves_head_intact_and_the_reset_restores_it() {
    let f = Fixture::new();
    let ids = f.seed(3);
    with_phone(&f);
    let head = f.head();
    let identity = f.identity();

    let dying = crashing_after(&f.vault, 2);
    assert!(crashed(|| {
        let _ = dying.remove_recipient("phone-1", true, &identity, &FakePrompter::new());
    }));
    assert_eq!(f.head(), head, "nothing was committed");
    assert!(!f.is_clean(), "the crash left the rewrite behind");

    // The next write's reset puts back the state HEAD describes, which
    // this write's rotation then replaces whole.
    f.vault
        .remove_recipient("phone-1", true, &f.identity(), &FakePrompter::new())
        .unwrap();
    assert!(f.is_clean());
    for id in ids {
        f.read(id, &f.identity()).unwrap();
    }
}
