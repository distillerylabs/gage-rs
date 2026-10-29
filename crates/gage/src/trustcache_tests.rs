//! The trust cache, as library tests.
//!
//! The trust cache is per-device local state, so nearly every test is a
//! two-actor story: this device, and whoever changed the recipient list
//! behind its back. The second actor is deliberately not a second gage
//! vault: it commits straight into the working tree, the way a pulled
//! commit from another machine arrives, which is the only way to produce a
//! change this device has genuinely never reviewed. Where the pull itself
//! matters, the change comes through a real origin instead.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::config::{self, Global, VaultEntry};
use crate::entry::{NewEntry, WriteError};
use crate::error::{ErrorKind, HasKind};
use crate::gitrepo;
use crate::identity::Identity;
use crate::prompter::FakePrompter;
use crate::recipient_tests::head_hash;
use crate::testsupport::fast_scrypt;
use crate::trustcache::{
    DiffOp, RecipientChangeWarning, TrustCache, TrustError, longest_common_subsequence,
    split_diff_lines,
};
use crate::vault::{CreateSpec, Vault};
use crate::xdgpaths::Dirs;
use crate::{identityfile, paths, recipients, vaultconfig};

pub(crate) const NAME: &str = "personal";
pub(crate) const DEVICE: &str = "laptop-1";
pub(crate) const PASSPHRASE: &str = "correct horse battery staple";

/// A local vault registered on a machine of its own, with this device's
/// real identity file, so an unlock is a real unlock. No origin: nothing here is about publishing.
pub(crate) struct TrustVault {
    _root: tempfile::TempDir,
    pub(crate) dirs: Dirs,
    pub(crate) vault: Vault,
}

impl TrustVault {
    pub(crate) fn new() -> TrustVault {
        fast_scrypt();
        let root = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            config: root.path().join("config"),
            data: root.path().join("data"),
            state: root.path().join("state"),
        };
        let id = vaultconfig::VaultId::new();
        let pubkey = identityfile::create(&dirs, &id, NAME, DEVICE, &unlocking()).unwrap();
        let path = root.path().join("vault");
        let vault = Vault::create(
            &dirs,
            &CreateSpec {
                name: NAME.into(),
                id: id.clone(),
                path: path.clone(),
                vault_type: crate::vaultconfig::VaultType::Git,
                method: crate::vaultconfig::UnlockMethod::Passphrase,
                device: DEVICE.into(),
                recipients: vec![pubkey.clone()],
                extra_recipients: Vec::new(),
                remote: None,
            },
        )
        .unwrap();
        let mut global = Global::default();
        global
            .register(
                NAME,
                VaultEntry {
                    path,
                    id: Some(id),
                    vault_type: crate::vaultconfig::VaultType::Git,
                    device: DEVICE.into(),
                    pubkey: Some(pubkey),
                    method: crate::vaultconfig::UnlockMethod::Passphrase,
                    ..Default::default()
                },
            )
            .unwrap();
        config::save(&paths::global_config_file(&dirs), &global).unwrap();
        TrustVault {
            _root: root,
            dirs,
            vault,
        }
    }

    /// A real unlock through `p`, which must script the passphrase.
    pub(crate) fn unlock(&self, p: &FakePrompter) -> Identity {
        self.vault.unlock(p).unwrap()
    }

    /// The first successful use: an unlock with nothing else to say, which
    /// bootstraps the cache.
    pub(crate) fn bootstrap(&self) {
        drop(self.unlock(&unlocking()));
    }

    /// This device's cached record of the vault, read as the code under
    /// test reads it.
    pub(crate) fn cache(&self) -> Option<TrustCache> {
        self.vault.load_trust_cache().unwrap()
    }

    /// Where the cache's config copy lives, spelled out literally rather
    /// than asked of the code under test: "under `$GAGE_STATE`, keyed by
    /// vault id" is a claim about a path, and a test that computed it the
    /// way the implementation does could not catch the implementation
    /// computing it wrongly.
    pub(crate) fn known_config_path(&self) -> PathBuf {
        self.dirs
            .state
            .join(&self.vault.id)
            .join("known-config.toml")
    }

    pub(crate) fn file(&self, rel: &str) -> Vec<u8> {
        std::fs::read(self.vault.path.join(rel)).unwrap()
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

    pub(crate) fn insert(&self, identity: &Identity, title: &str, p: &FakePrompter) -> Uuid {
        self.try_insert(identity, title, p).unwrap()
    }

    pub(crate) fn try_insert(
        &self,
        identity: &Identity,
        title: &str,
        p: &FakePrompter,
    ) -> Result<Uuid, WriteError> {
        self.vault.insert(identity, sample(title), true, p)
    }

    /// `.age-recipients`, read as itself.
    pub(crate) fn recipients_file(&self) -> Vec<String> {
        recipients::read(&self.vault.path.join(".age-recipients")).unwrap()
    }
}

/// A prompter that answers this device's passphrase once, and nothing else.
pub(crate) fn unlocking() -> FakePrompter {
    FakePrompter::new().with_passphrase(PASSPHRASE)
}

/// [`unlocking`], answering the recipient-change question with `answers`,
/// in order.
pub(crate) fn trusting(answers: &[bool]) -> FakePrompter {
    unlocking().with_recipient_answers(answers)
}

pub(crate) fn sample(title: &str) -> NewEntry {
    NewEntry {
        title: title.into(),
        value: "hunter2".into(),
        ..NewEntry::default()
    }
}

/// A fresh, real age public key: what an attacker appends to
/// `.age-recipients`, or what a legitimate `recipient add` on another
/// device commits.
pub(crate) fn new_key() -> String {
    age::x25519::Identity::generate().to_public().to_string()
}

/// Appends a recipient to both files and commits them together: what a
/// legitimate `recipient add` on another device leaves behind, as this
/// device finds it after a pull. The routine case: `verify` still passes.
pub(crate) fn commit_routine_change(vault: &Vault, device: &str, pubkey: &str) {
    let path = vault.path.join(".age-recipients");
    let mut keys = recipients::read(&path).unwrap();
    keys.push(pubkey.into());
    recipients::write(&path, &keys).unwrap();
    let mut file = vaultconfig::read(&vault.config_path()).unwrap();
    file.recipients.push(vaultconfig::Recipient {
        device: device.into(),
        pubkey: pubkey.into(),
    });
    vaultconfig::write(&vault.config_path(), &file).unwrap();
    gitrepo::commit_all(&vault.path, &format!("gage: recipient add {device}")).unwrap();
}

/// Appends a key to `.age-recipients` and nothing else, then commits it:
/// the hand edit of the file encryption actually reads, which diffing
/// `config.toml` alone would never see. The mismatched case: `verify`
/// fails afterwards.
pub(crate) fn tamper_recipients_file(vault: &Vault, pubkey: &str) {
    let path = vault.path.join(".age-recipients");
    let mut keys = recipients::read(&path).unwrap();
    keys.push(pubkey.into());
    recipients::write(&path, &keys).unwrap();
    gitrepo::commit_all(&vault.path, "add a key").unwrap();
}

/// The hex SHA-256 of a vault file, computed independently of the cache,
/// so "the stored hash is the hash of `.age-recipients`" is checked
/// against the file and not against the implementation's own idea of it.
fn sha256_hex(path: &Path) -> String {
    use sha2::Digest;
    let sum = sha2::Sha256::digest(std::fs::read(path).unwrap());
    sum.iter().map(|b| format!("{b:02x}")).collect()
}

/// The single line of the committed `.gage/config.toml` holding `want`:
/// the literal text a diff of that file has to carry. The quoting is the
/// TOML encoder's to choose, so it is read rather than spelled out.
fn config_line_containing(vault: &Vault, want: &str) -> String {
    let text = std::fs::read_to_string(vault.config_path()).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| l.contains(want)).collect();
    assert_eq!(
        lines.len(),
        1,
        "{want:?} is not on exactly one line:\n{text}"
    );
    lines[0].to_string()
}

// --- bootstrap and storage --------------------------------------------------

// After the first
// successful use the cache exists; a later unreviewed change is asked
// about before the next encrypt.
#[test]
fn first_use_writes_the_cache_then_an_unreviewed_change_warns() {
    let t = TrustVault::new();
    assert!(
        t.cache().is_none(),
        "a cache exists before the vault was ever used; the bootstrap is on first use"
    );

    // Nothing to compare against yet, so the first use writes the cache
    // silently.
    let first = trusting(&[]);
    drop(t.unlock(&first));
    assert!(first.recipient_changes().is_empty());
    assert!(first.warnings().is_empty(), "{:?}", first.warnings());
    assert!(t.known_config_path().is_file());
    let cache = t.cache().expect("no cache after the first successful use");
    assert!(!cache.recipients_hash.is_empty());

    // Another device adds a recipient, correctly touching both files.
    commit_routine_change(&t.vault, "phone-1", &new_key());

    let p = trusting(&[true]);
    let identity = t.unlock(&p);
    t.insert(&identity, "ProtonMail", &p);

    let asked = p.recipient_changes();
    assert_eq!(asked.len(), 1, "the encrypt never asked about the change");
    assert_eq!(asked[0].vault, NAME);
    assert!(asked[0].diff.contains("phone-1"), "{}", asked[0].diff);
    assert!(
        !asked[0].mismatched(),
        "both files changed: the routine case"
    );
}

// The config half is a verbatim copy:
// a diff of anything re-serialized would be a diff of gage's formatting,
// not of the change.
#[test]
fn trust_cache_stores_both_halves() {
    let t = TrustVault::new();
    t.bootstrap();

    let cached = std::fs::read(t.known_config_path()).expect("reading the cached copy");
    assert_eq!(cached, t.file(".gage/config.toml"), "not a verbatim copy");
    let cache = t.cache().unwrap();
    assert_eq!(cache.known_config, cached);
    assert_eq!(
        cache.recipients_hash,
        sha256_hex(&t.vault.path.join(".age-recipients"))
    );
    assert!(
        cache.confirmed_at.is_some(),
        "no confirmation time recorded"
    );
}

// A regeneration
// writes under `$GAGE_STATE`, nothing inside the vault, and commits
// nothing.
#[test]
fn trust_cache_lives_under_state_and_is_never_committed() {
    let t = TrustVault::new();
    t.bootstrap();
    commit_routine_change(&t.vault, "phone-1", &new_key());

    let commits = t.commits();
    let p = trusting(&[true]);
    let identity = t.unlock(&p);
    t.insert(&identity, "ProtonMail", &p);
    assert!(
        t.known_config_path()
            .starts_with(t.dirs.state.join(&t.vault.id))
    );
    assert!(t.known_config_path().is_file());

    // Nothing cache-shaped inside the vault, at any depth.
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if name == ".git" {
                continue;
            }
            if name.contains("known-config") || name == "trust.toml" {
                found.push(path.clone());
            }
            if path.is_dir() {
                walk(&path, found);
            }
        }
    }
    let mut found = Vec::new();
    walk(&t.vault.path, &mut found);
    assert!(
        found.is_empty(),
        "trust state written in the vault: {found:?}"
    );

    assert!(
        t.is_clean(),
        "a cache regeneration dirtied the working tree"
    );
    assert_eq!(
        t.commits(),
        commits + 1,
        "the insert's commit and nothing else"
    );
}

// The frontend
// receives a value: the diff, the verification, who moved, and when this
// device last confirmed. Its rendering is gage-cli's, tested there.
#[test]
fn the_recipient_warning_is_a_typed_value_not_printed_text() {
    let t = TrustVault::new();
    t.bootstrap();
    let added = new_key();
    commit_routine_change(&t.vault, "phone-1", &added);

    let p = trusting(&[true]);
    let identity = t.unlock(&p);
    t.insert(&identity, "ProtonMail", &p);
    let asked = p.recipient_changes();
    assert_eq!(asked.len(), 1);
    let w = &asked[0];

    assert!(!w.diff.is_empty(), "the warning shows no change");
    let device_line = config_line_containing(&t.vault, "phone-1");
    for want in [
        "known-config.toml".to_string(),
        ".gage/config.toml".to_string(),
        format!("+{device_line}"),
    ] {
        assert!(w.diff.contains(&want), "missing {want:?}:\n{}", w.diff);
    }
    assert_eq!(w.added.len(), 1, "{:?}", w.added);
    assert_eq!(w.added[0].pubkey, added);
    assert_eq!(w.added[0].device, "phone-1");
    assert!(w.removed.is_empty(), "{:?}", w.removed);
    assert!(w.verification.in_sync);
    assert!(
        w.last_confirmed.is_some(),
        "the diff header's date is missing"
    );
}

/// Changes a `config.toml` field that isn't a recipient and commits it: a
/// real config diff with nothing to count from it.
fn tamper_config_method_only(vault: &Vault) {
    let mut file = vaultconfig::read(&vault.config_path()).unwrap();
    file.method.plugin = Some("yubikey".into());
    vaultconfig::write(&vault.config_path(), &file).unwrap();
    gitrepo::commit_all(&vault.path, "gage: touch a non-recipient config field").unwrap();
}

// A routine change whose
// config diff is real but adds and removes nobody: the summary's fallback
// line, reached through its non-mismatched branch.
#[test]
fn delta_with_no_recipient_change_says_edited_alone() {
    let t = TrustVault::new();
    t.bootstrap();
    tamper_config_method_only(&t.vault);

    let p = trusting(&[true]);
    let identity = t.unlock(&p);
    t.insert(&identity, "ProtonMail", &p);

    let asked = p.recipient_changes();
    assert_eq!(asked.len(), 1, "a config-only edit was not asked about");
    let w = &asked[0];
    assert!(!w.mismatched());
    assert!(w.added.is_empty() && w.removed.is_empty(), "{w:?}");
    assert_eq!(w.delta(), ".age-recipients was edited on its own");
    assert!(
        w.summary().contains("recipient list changed"),
        "{}",
        w.summary()
    );
}

// The
// one edit-script shape the scenario tests never produce alone, and the
// empty endpoints a diff of an empty file needs.
#[test]
fn longest_common_subsequence_handles_pure_insertion_and_empty_input() {
    let a = ["a".to_string()];
    let b = ["a".to_string(), "b".to_string(), "c".to_string()];
    assert_eq!(
        longest_common_subsequence(&a, &b),
        vec![
            DiffOp::Keep("a".into()),
            DiffOp::Add("b".into()),
            DiffOp::Add("c".into()),
        ]
    );
    assert!(split_diff_lines(b"").is_empty());
    assert!(
        split_diff_lines(b"\n").is_empty(),
        "a lone trailing newline is not a blank last line"
    );
    assert_eq!(split_diff_lines(b"x\r\ny\n"), vec!["x", "y"]);
}

// The summary's three shapes, and that a mismatch points at `verify`
// rather than promising a later question.
#[test]
fn summary_says_what_to_do_next() {
    let r = |device: &str| vaultconfig::Recipient {
        device: device.into(),
        pubkey: format!("age1{device}"),
    };
    let mut w = RecipientChangeWarning {
        vault: NAME.into(),
        added: vec![r("a"), r("b")],
        removed: vec![r("c")],
        verification: crate::recipient::RecipientVerification {
            in_sync: true,
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(w.delta(), "2 added, 1 removed");
    assert!(
        w.summary().contains("the next write here will ask"),
        "{}",
        w.summary()
    );
    w.removed.clear();
    assert_eq!(w.delta(), "2 added");
    w.added.clear();
    w.removed.push(r("c"));
    assert_eq!(w.delta(), "1 removed");
    w.verification.in_sync = false;
    assert!(w.mismatched());
    assert_eq!(
        w.summary_remedy(),
        Some(crate::error::Remedy::RecipientVerify),
        "{}",
        w.summary()
    );
}

// A cache whose two files didn't both land reads as no record: the pair is
// the record, and half of it is not one this device can honestly claim to
// have confirmed.
#[test]
fn a_half_written_cache_reads_as_no_record() {
    let t = TrustVault::new();
    t.bootstrap();
    std::fs::remove_file(t.dirs.state.join(&t.vault.id).join("trust.toml")).unwrap();
    assert!(t.cache().is_none());
}

// A cache file that can't be parsed is an error, not a first use: treating
// it as absent would re-bootstrap over whatever was there.
#[test]
fn an_unparseable_trust_record_is_an_error() {
    let t = TrustVault::new();
    t.bootstrap();
    std::fs::write(
        t.dirs.state.join(&t.vault.id).join("trust.toml"),
        "recipients_hash = [",
    )
    .unwrap();
    let err = t.vault.load_trust_cache().unwrap_err();
    assert!(matches!(err, TrustError::Parse { .. }), "{err}");
    assert_eq!(err.kind(), ErrorKind::Internal);
}

// --- the blocking check -----------------------------------------------------

// No Session anywhere:
// each call is what a one-shot command makes, and each must ask on its own.
// `generate` is `insert` with a generated value in both CLIs, so the insert
// case covers it.
#[test]
fn blocking_check_hooks_every_encrypting_vault_method() {
    type Write = fn(&TrustVault, &Identity, &FakePrompter) -> Result<(), WriteError>;
    let cases: [(&str, Write); 3] = [
        ("insert", |t, id, p| {
            t.try_insert(id, "another", p).map(drop)
        }),
        ("update", |t, id, p| {
            let (uuid, mut e) = t.vault.resolve(id, "ProtonMail").unwrap();
            e.value = "rewritten".into();
            t.vault.update(uuid, &e, p)
        }),
        ("rename", |t, id, p| {
            t.vault
                .rename(id, "ProtonMail", "Proton Mail", false, p)
                .map(drop)
        }),
    ];
    for (name, write) in cases {
        let t = TrustVault::new();
        // One entry to edit or rename, written before the list moves, so
        // the change under test is the only reason to ask.
        let p = trusting(&[]);
        let identity = t.unlock(&p);
        t.insert(&identity, "ProtonMail", &p);
        commit_routine_change(&t.vault, "phone-1", &new_key());

        let p = trusting(&[true]);
        write(&t, &identity, &p).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            p.recipient_changes().len(),
            1,
            "{name} encrypted without asking about the change"
        );
    }
}

// `rm` encrypts nothing, so it asks nothing: a question on every write
// would be a tax, not a question about the one thing it asks about.
#[test]
fn removing_an_entry_asks_nothing() {
    let t = TrustVault::new();
    let p = trusting(&[]);
    let identity = t.unlock(&p);
    t.insert(&identity, "ProtonMail", &p);
    commit_routine_change(&t.vault, "phone-1", &new_key());

    let p = trusting(&[]);
    t.vault.remove(&identity, "ProtonMail", &p).unwrap();
    assert!(p.recipient_changes().is_empty());
}

// With `config.toml` untouched
// there is no diff to show at all; the hash half is what catches it.
#[test]
fn recipients_file_edit_alone_still_warns() {
    let t = TrustVault::new();
    t.bootstrap();
    let stray = new_key();
    tamper_recipients_file(&t.vault, &stray);

    let p = trusting(&[true]);
    let identity = t.unlock(&p);
    t.insert(&identity, "ProtonMail", &p);

    let asked = p.recipient_changes();
    assert_eq!(
        asked.len(),
        1,
        "an .age-recipients-only edit went unnoticed"
    );
    assert!(asked[0].diff.is_empty(), "{}", asked[0].diff);
    assert!(asked[0].mismatched());
    assert!(
        asked[0]
            .verification
            .only_in_recipients_file
            .contains(&stray),
        "{:?}",
        asked[0].verification
    );
}

// No commit, no entry,
// the cache as it was, and so the same question next time.
#[test]
fn declining_the_recipient_change_aborts_the_write() {
    let t = TrustVault::new();
    t.bootstrap();
    let before = t.cache().unwrap();
    commit_routine_change(&t.vault, "phone-1", &new_key());
    let (commits, head) = (t.commits(), t.head());

    let p = trusting(&[false]);
    let identity = t.unlock(&p);
    let err = t.try_insert(&identity, "ProtonMail", &p).unwrap_err();
    assert!(
        matches!(err, WriteError::Trust(TrustError::ChangeDeclined { .. })),
        "{err}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);

    assert_eq!(t.commits(), commits, "a declined write committed");
    assert_eq!(t.head(), head);
    assert!(t.vault.entry_ids().unwrap().is_empty());
    let after = t.cache().expect("the cache vanished on a declined write");
    assert_eq!(after, before, "a declined change moved the cache");

    // And it does come back.
    let p = trusting(&[false]);
    let err = t.try_insert(&identity, "ProtonMail", &p).unwrap_err();
    assert!(matches!(
        err,
        WriteError::Trust(TrustError::ChangeDeclined { .. })
    ));
    assert_eq!(p.recipient_changes().len(), 1, "not asked again");
}

// One acknowledgment
// per change, not per command.
#[test]
fn confirming_a_routine_change_regenerates_both_halves() {
    let t = TrustVault::new();
    t.bootstrap();
    commit_routine_change(&t.vault, "phone-1", &new_key());

    let p = trusting(&[true]);
    let identity = t.unlock(&p);
    t.insert(&identity, "ProtonMail", &p);
    assert_eq!(p.recipient_changes().len(), 1);

    let cache = t.cache().unwrap();
    assert_eq!(cache.known_config, t.file(".gage/config.toml"));
    assert_eq!(
        cache.recipients_hash,
        sha256_hex(&t.vault.path.join(".age-recipients")),
        "both halves regenerate together"
    );

    let p = trusting(&[]);
    let identity = t.unlock(&p);
    for title in ["another", "and another"] {
        t.insert(&identity, title, &p);
    }
    assert!(
        p.recipient_changes().is_empty(),
        "asked again after confirming"
    );
    assert!(p.warnings().is_empty(), "{:?}", p.warnings());
}

// The write goes
// through; silence is not bought. `verify` keeps failing and the next
// write asks again.
#[test]
fn confirming_a_mismatched_change_does_not_clear_the_cache() {
    let t = TrustVault::new();
    t.bootstrap();
    let before = t.cache().unwrap();
    let stray = new_key();
    tamper_recipients_file(&t.vault, &stray);

    let p = trusting(&[true]);
    let identity = t.unlock(&p);
    t.insert(&identity, "ProtonMail", &p);

    let after = t.cache().unwrap();
    assert_eq!(after.recipients_hash, before.recipients_hash);
    assert_eq!(after.known_config, before.known_config);
    assert_eq!(
        after.mismatch_seen,
        Some(sha256_hex(&t.vault.path.join(".age-recipients"))),
        "the acknowledged mismatch was not recorded"
    );

    let verdict = t.vault.verify_recipients().unwrap();
    assert!(!verdict.in_sync);
    assert!(verdict.only_in_recipients_file.contains(&stray));

    let p = trusting(&[true]);
    t.insert(&identity, "another", &p);
    assert_eq!(
        p.recipient_changes().len(),
        1,
        "a still-mismatched vault went quiet"
    );
}

// A frontend with
// nobody to ask implements nothing, and the trait's default refuses: an
// unapproved list is exactly what this check won't encrypt to.
#[test]
fn confirm_recipient_trust_with_no_prompter_refuses() {
    struct Mute;
    impl crate::prompter::Prompter for Mute {
        fn unlock(
            &self,
            _: &crate::prompter::UnlockRequest,
        ) -> Result<crate::prompter::UnlockResponse, crate::prompter::PromptError> {
            unreachable!()
        }
        fn confirm(&self, _: &str) -> Result<bool, crate::prompter::PromptError> {
            unreachable!()
        }
        fn confirm_default_yes(&self, _: &str) -> Result<bool, crate::prompter::PromptError> {
            unreachable!()
        }
        fn value(
            &self,
            _: &str,
        ) -> Result<zeroize::Zeroizing<String>, crate::prompter::PromptError> {
            unreachable!()
        }
        fn choose(
            &self,
            _: &crate::entry::CandidateList,
        ) -> Result<Uuid, crate::prompter::PromptError> {
            unreachable!()
        }
        fn warn(&self, _: &crate::prompter::Warning) {}
    }

    let t = TrustVault::new();
    t.bootstrap();
    commit_routine_change(&t.vault, "other-device", &new_key());
    let err = t.vault.confirm_recipient_trust(&Mute).unwrap_err();
    assert!(
        matches!(
            err,
            TrustError::Unconfirmable {
                source: crate::prompter::PromptError::NotInteractive,
                ..
            }
        ),
        "{err}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
}

// --- the opportunistic checks -----------------------------------------------

// A plain read warns
// and asks nothing, and doesn't launder the unreviewed list into the cache.
#[test]
fn unlock_warns_opportunistically_without_blocking() {
    let t = TrustVault::new();
    let p = trusting(&[]);
    let identity = t.unlock(&p);
    t.insert(&identity, "ProtonMail", &p);
    drop(identity);
    commit_routine_change(&t.vault, "phone-1", &new_key());

    // Unlock, list, resolve: nothing here encrypts.
    let p = trusting(&[]);
    let identity = t.unlock(&p);
    let warnings = p.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("recipient list changed"),
        "{warnings:?}"
    );
    assert!(warnings[0].contains("1 added"), "{warnings:?}");
    assert!(
        p.recipient_changes().is_empty(),
        "an unlock asked a question"
    );

    t.vault.list(&identity).unwrap();
    t.vault.resolve(&identity, "ProtonMail").unwrap();
    let cache = t.cache().unwrap();
    assert!(
        !String::from_utf8_lossy(&cache.known_config).contains("phone-1"),
        "a read regenerated the cache"
    );
}

// A wrong passphrase is not a review: a failed unlock must not bootstrap
// an approval of whatever list happens to be on disk (the plan's "cache
// written on a failed unlock" mutation).
#[test]
fn a_failed_unlock_does_not_bootstrap_the_cache() {
    let t = TrustVault::new();
    let p = FakePrompter::new()
        .with_passphrase("wrong")
        .with_unlock_error(crate::prompter::PromptError::TooManyAttempts);
    t.vault.unlock(&p).unwrap_err();
    assert!(t.cache().is_none(), "a failed unlock wrote the cache");
    assert!(!t.dirs.state.join(&t.vault.id).exists());
}

// A fast-forward unlocks nothing, so
// the warning can't be riding on an unlock: the sync has its own hook.
#[test]
fn sync_warns_on_a_clean_fast_forward() {
    use crate::remotesync::tests::SyncVault;
    let s = SyncVault::new();
    drop(s.unlock()); // the first use

    // Another device commits a recipient change and publishes it. The local
    // side has nothing of its own, so this is a clean fast-forward.
    let other = s.other();
    let keys = other.read(".age-recipients");
    other.write_commit_push(
        ".age-recipients",
        &format!("{keys}{}\n", new_key()),
        "another device edits the recipient list",
    );

    let p = FakePrompter::new();
    let report = s
        .vault
        .sync_resolving(&p, &mut || {
            panic!("a clean fast-forward unlocked an identity")
        })
        .unwrap();
    assert!(report.pulled, "{report:?}");
    let warnings = p.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("recipient files now disagree"),
        "{warnings:?}"
    );
    assert!(p.recipient_changes().is_empty(), "a sync asked a question");
}

// --- recipient verbs: refusal, trust and regeneration -----------------------

/// What origin's branch points at now, read through a fresh clone.
fn origin_head(s: &crate::remotesync::tests::SyncVault) -> String {
    crate::gittest::Device::clone(s.remote.path()).head()
}

/// `vault` with a counter its re-encryption bumps per entry written.
fn counting(vault: &Vault) -> (Vault, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let n = std::sync::Arc::new(AtomicUsize::new(0));
    let seen = std::sync::Arc::clone(&n);
    let v = vault
        .clone()
        .with_reencrypt_hook(move |done| seen.store(done, Ordering::SeqCst));
    (v, n)
}

// Both verbs: the
// rebuild that would erase the stray key is the shared tail both commit
// through. The refusal names the difference, and leaves both files
// byte-identical, nothing committed or pushed, and the cache as it was:
// retrying the add can't launder a mismatch.
#[test]
fn recipient_add_and_remove_refuse_over_a_failing_verify() {
    use crate::recipient::RecipientError;
    use crate::remotesync::tests::SyncVault;
    for verb in ["add", "remove"] {
        let s = SyncVault::new();
        let phone = new_key();
        let p = trusting(&[]);
        let identity = s.vault.unlock(&p).unwrap();
        s.vault
            .insert(&identity, sample("ProtonMail"), true, &p)
            .unwrap();
        // remove needs someone to remove; add needs the name free.
        if verb == "remove" {
            s.vault
                .add_recipient("phone-1", &phone, &identity, &p)
                .unwrap();
        }
        let stray = new_key();
        tamper_recipients_file(&s.vault, &stray);

        let read = |rel: &str| std::fs::read(s.vault.path.join(rel)).unwrap();
        let (keys_before, config_before) = (read(".age-recipients"), read(".gage/config.toml"));
        let commits = gitrepo::commit_count(&s.vault.path).unwrap();
        let origin = origin_head(&s);
        let cache_before = s.vault.load_trust_cache().unwrap();

        let p = trusting(&[true]);
        let err = match verb {
            "add" => s.vault.add_recipient("phone-1", &phone, &identity, &p),
            _ => s.vault.remove_recipient("phone-1", true, &identity, &p),
        }
        .unwrap_err();
        assert!(
            matches!(err, RecipientError::Trust(TrustError::OutOfSync { .. })),
            "{verb}: {err}"
        );
        assert_eq!(err.kind(), ErrorKind::Conflict);
        assert!(err.to_string().contains(&stray), "{verb}: {err}");
        assert_eq!(
            err.remedy(),
            Some(crate::error::Remedy::RecipientRepair),
            "{verb}: {err}"
        );
        // Refused before anyone was asked to approve it.
        assert!(p.recipient_changes().is_empty(), "{verb} asked first");
        assert!(p.confirm_prompts().is_empty(), "{verb} asked first");

        assert_eq!(read(".age-recipients"), keys_before, "{verb} rewrote it");
        assert_eq!(read(".gage/config.toml"), config_before, "{verb}");
        let verdict = s.vault.verify_recipients().unwrap();
        assert!(!verdict.in_sync && verdict.only_in_recipients_file.contains(&stray));
        assert_eq!(gitrepo::commit_count(&s.vault.path).unwrap(), commits);
        assert_eq!(origin_head(&s), origin, "{verb} pushed");
        assert_eq!(s.vault.load_trust_cache().unwrap(), cache_before);
    }
}

// The in-sync
// check sits ahead of the re-encryption pass, not beside the
// recipient-file write: aborting after rewriting entries would be its own
// half-migrated state.
#[test]
fn reencrypt_refusal_happens_before_any_entry_is_rewritten() {
    let t = TrustVault::new();
    let p = trusting(&[]);
    let identity = t.unlock(&p);
    for title in ["one", "two"] {
        t.insert(&identity, title, &p);
    }
    tamper_recipients_file(&t.vault, &new_key());

    let (vault, reencrypted) = counting(&t.vault);
    let p = trusting(&[true]);
    let err = vault
        .add_recipient("phone-1", &new_key(), &identity, &p)
        .unwrap_err();
    assert!(err.to_string().contains("disagree"), "{err}");
    assert_eq!(
        reencrypted.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "entries were re-encrypted before the refusal"
    );
    assert!(t.is_clean());
}

// A consistent
// change pulled from another device passes `verify`, so only the trust
// question stands between it and being blessed by a local add's
// regeneration, after every entry was rewritten to it. Declined: asked
// once, about the pulled key, and nothing re-encrypted, committed or
// cached.
#[test]
fn recipient_add_does_not_launder_someone_elses_change() {
    use crate::remotesync::tests::SyncVault;
    let s = SyncVault::new();
    let p = trusting(&[]);
    let identity = s.vault.unlock(&p).unwrap();
    s.vault
        .insert(&identity, sample("ProtonMail"), true, &p)
        .unwrap();
    drop(identity);

    // Another device's change, arriving the way a pull delivers one: both
    // files, committed together and published.
    let stranger = new_key();
    let other = s.other();
    other.pull();
    commit_routine_change_at(&other, "stranger", &stranger);
    other.push();

    // This device's unlock pulls it in, and only warns.
    let p = trusting(&[false]);
    let identity = s.vault.unlock(&p).unwrap();
    assert_eq!(p.warnings().len(), 1, "{:?}", p.warnings());
    let commits = gitrepo::commit_count(&s.vault.path).unwrap();
    let cache_before = s.vault.load_trust_cache().unwrap().unwrap();

    let (vault, reencrypted) = counting(&s.vault);
    let phone = new_key();
    let err = vault
        .add_recipient("phone-1", &phone, &identity, &p)
        .unwrap_err();
    assert!(
        matches!(
            err,
            crate::recipient::RecipientError::Trust(TrustError::ChangeDeclined { .. })
        ),
        "{err}"
    );
    let asked = p.recipient_changes();
    assert_eq!(asked.len(), 1, "asked {} times", asked.len());
    assert!(asked[0].diff.contains("stranger"), "{}", asked[0].diff);
    assert_eq!(reencrypted.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(gitrepo::commit_count(&s.vault.path).unwrap(), commits);
    let keys = recipients::read(&s.vault.path.join(".age-recipients")).unwrap();
    assert!(!keys.contains(&phone), "added despite the refusal");

    let cache_after = s.vault.load_trust_cache().unwrap().unwrap();
    assert_eq!(cache_after, cache_before, "a declined add moved the cache");
    assert!(
        !String::from_utf8_lossy(&cache_after.known_config).contains(&stranger),
        "the pulled key was laundered into the cache"
    );
}

/// [`commit_routine_change`] on a raw git clone: another device's
/// legitimate `recipient add`, committed there.
fn commit_routine_change_at(d: &crate::gittest::Device, device: &str, pubkey: &str) {
    let keys = d.read(".age-recipients");
    d.write(".age-recipients", &format!("{keys}{pubkey}\n"));
    let config = d.path().join(".gage").join("config.toml");
    let mut file = vaultconfig::read(&config).unwrap();
    file.recipients.push(vaultconfig::Recipient {
        device: device.into(),
        pubkey: pubkey.into(),
    });
    vaultconfig::write(&config, &file).unwrap();
    d.commit(&format!("gage: recipient add {device}"));
}

// The pulled change
// is reviewed once, the operator's own add lands, and the cache covers
// both, so the next write asks nothing.
#[test]
fn confirming_lets_the_add_through_and_regenerates_once() {
    let t = TrustVault::new();
    t.bootstrap();
    let stranger = new_key();
    commit_routine_change(&t.vault, "stranger", &stranger);

    let p = trusting(&[true]);
    let identity = t.unlock(&p);
    let phone = new_key();
    t.vault
        .add_recipient("phone-1", &phone, &identity, &p)
        .unwrap();
    assert_eq!(p.recipient_changes().len(), 1);

    let cache = t.cache().unwrap();
    let known = String::from_utf8_lossy(&cache.known_config);
    assert!(known.contains(&stranger), "the reviewed key is missing");
    assert!(known.contains(&phone), "the operator's own key is missing");
    assert_eq!(cache.known_config, t.file(".gage/config.toml"));

    let p = trusting(&[]);
    t.insert(&identity, "ProtonMail", &p);
    assert!(
        p.recipient_changes().is_empty(),
        "asked again after confirming"
    );
}

// A change the operator made
// here needs no second review: warning about their own add at their next
// write is the fastest way to teach them to stop reading the warning.
// `remove` regenerates too.
#[test]
fn a_local_recipient_add_regenerates_the_cache() {
    let t = TrustVault::new();
    let p = trusting(&[]);
    let identity = t.unlock(&p);
    t.vault
        .add_recipient("phone-1", &new_key(), &identity, &p)
        .unwrap();
    t.insert(&identity, "ProtonMail", &p);
    t.vault
        .remove_recipient("phone-1", true, &identity, &p)
        .unwrap();
    t.insert(&identity, "Bank", &p);
    assert!(
        p.recipient_changes().is_empty(),
        "the operator was asked to review their own change: {:?}",
        p.recipient_changes()
    );
    assert_eq!(t.cache().unwrap().known_config, t.file(".gage/config.toml"));
}

// `remove` asks about an unreviewed change too, and before its own
// self-removal question: the order inside the verbs is in-sync check,
// trust question, then everything else.
#[test]
fn recipient_remove_asks_about_an_unreviewed_change_before_anything_else() {
    let t = TrustVault::new();
    let p = trusting(&[]);
    let identity = t.unlock(&p);
    t.vault
        .add_recipient("phone-1", &new_key(), &identity, &p)
        .unwrap();
    commit_routine_change(&t.vault, "stranger", &new_key());
    let commits = t.commits();

    // Removing this device's own key would ask `confirm`; the declined
    // trust question has to come first, so `confirm` is never reached.
    let p = trusting(&[false]);
    let err = t
        .vault
        .remove_recipient(DEVICE, true, &identity, &p)
        .unwrap_err();
    assert!(err.to_string().contains("was not confirmed"), "{err}");
    assert_eq!(p.recipient_changes().len(), 1);
    assert!(p.confirm_prompts().is_empty(), "{:?}", p.confirm_prompts());
    assert_eq!(t.commits(), commits);
}

// --- verify --repair --------------------------------------------------------

// The refusal's exit: a
// deliberate, visible act, confirmed first, naming what it drops.
#[test]
fn repair_rewrites_the_recipients_file_from_config() {
    let t = TrustVault::new();
    t.bootstrap();
    let stray = new_key();
    tamper_recipients_file(&t.vault, &stray);
    let commits = t.commits();

    let p = FakePrompter::new().with_confirm(true);
    let repair = t.vault.repair_recipients(&p).unwrap();
    assert_eq!(repair.dropped, vec![stray.clone()]);
    assert!(repair.added.is_empty(), "{:?}", repair.added);
    assert!(repair.commit.is_some(), "no commit reported");

    let asked = p.confirm_prompts();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(asked[0].contains(&stray), "{}", asked[0]);
    assert!(asked[0].contains("drop"), "{}", asked[0]);

    assert!(t.vault.verify_recipients().unwrap().in_sync);
    assert!(!t.recipients_file().contains(&stray));
    assert_eq!(t.commits(), commits + 1);
    assert_eq!(
        gitrepo::head_commit(&t.vault.path).unwrap().message,
        "gage: recipient repair"
    );
    assert!(t.is_clean(), "the rewrite was left uncommitted");
}

// It has to have asked to have been
// declined, and the divergence stays exactly where it was.
#[test]
fn repair_declined_writes_nothing() {
    let t = TrustVault::new();
    t.bootstrap();
    tamper_recipients_file(&t.vault, &new_key());
    let (commits, before) = (t.commits(), t.file(".age-recipients"));

    let p = FakePrompter::new().with_confirm(false);
    let err = t.vault.repair_recipients(&p).unwrap_err();
    assert!(
        matches!(err, crate::recipient::RecipientError::RepairDeclined { .. }),
        "{err}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(p.confirm_prompts().len(), 1);
    assert_eq!(t.file(".age-recipients"), before);
    assert_eq!(t.commits(), commits);
}

// Nobody to confirm the drop with
// means no repair, never a silent one.
#[test]
fn repair_with_no_prompter_refuses() {
    let t = TrustVault::new();
    t.bootstrap();
    tamper_recipients_file(&t.vault, &new_key());
    let commits = t.commits();

    let p = FakePrompter::new().with_confirm_error(crate::prompter::PromptError::NotInteractive);
    let err = t.vault.repair_recipients(&p).unwrap_err();
    assert!(
        matches!(
            err,
            crate::recipient::RecipientError::RepairUnconfirmed { .. }
        ),
        "{err}"
    );
    assert!(err.to_string().contains("disagree"), "{err}");
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(t.commits(), commits);
}

// No question nobody
// needs to answer, and no empty commit.
#[test]
fn repair_on_an_already_in_sync_vault_does_nothing() {
    let t = TrustVault::new();
    t.bootstrap();
    let commits = t.commits();
    let p = FakePrompter::new();
    let repair = t.vault.repair_recipients(&p).unwrap();
    assert_eq!(repair.commit, None);
    assert!(repair.dropped.is_empty() && repair.added.is_empty());
    assert!(p.confirm_prompts().is_empty());
    assert_eq!(t.commits(), commits);
}

// Repair settles the split
// between the two files, not whether the list is one this device
// approves: regenerating here would launder an unreviewed key the way the
// old rebuilding add did.
#[test]
fn repair_does_not_regenerate_the_trust_cache() {
    let t = TrustVault::new();
    t.bootstrap();
    let before = t.cache().unwrap();
    // A change to both files (so it survives the repair) this device never
    // reviewed, plus a stray key to make `verify` fail.
    commit_routine_change(&t.vault, "phone-1", &new_key());
    tamper_recipients_file(&t.vault, &new_key());

    t.vault
        .repair_recipients(&FakePrompter::new().with_confirm(true))
        .unwrap();
    assert_eq!(t.cache().unwrap(), before, "the repair moved the cache");

    let p = trusting(&[true]);
    let identity = t.unlock(&p);
    t.insert(&identity, "ProtonMail", &p);
    let asked = p.recipient_changes();
    assert_eq!(asked.len(), 1, "the next encrypt asked nothing");
    assert!(
        asked[0].verification.in_sync,
        "still reported as disagreeing"
    );
}

/// Appends a recipient to `config.toml` alone and commits: a key declared
/// with nothing yet encrypted to it.
fn tamper_config_file(vault: &Vault, device: &str, pubkey: &str) {
    let mut file = vaultconfig::read(&vault.config_path()).unwrap();
    file.recipients.push(vaultconfig::Recipient {
        device: device.into(),
        pubkey: pubkey.into(),
    });
    vaultconfig::write(&vault.config_path(), &file).unwrap();
    gitrepo::commit_all(&vault.path, "gage: config-only recipient edit").unwrap();
}

// The other direction: a key
// only `config.toml` names. Repair adds it, and says so.
#[test]
fn verify_and_repair_report_only_in_config() {
    let t = TrustVault::new();
    t.bootstrap();
    let extra = new_key();
    tamper_config_file(&t.vault, "phone-1", &extra);

    let verdict = t.vault.verify_recipients().unwrap();
    assert!(!verdict.in_sync);
    assert_eq!(verdict.only_in_config, vec![extra.clone()]);
    assert!(verdict.only_in_recipients_file.is_empty());

    let p = FakePrompter::new().with_confirm(true);
    let repair = t.vault.repair_recipients(&p).unwrap();
    assert_eq!(repair.added, vec![extra.clone()]);
    let asked = &p.confirm_prompts()[0];
    assert!(asked.contains("add") && asked.contains(&extra), "{asked}");
    assert!(t.recipients_file().contains(&extra));
}

// Clippy's `result_large_err` limit is 128 bytes, and only Windows CI
// crossed it (its `PathBuf` is 8 bytes larger), so it went unnoticed on
// Linux and macOS. `TrustError` rides inside every write's error type, so
// it stays small here on every platform, with room to spare.
#[test]
fn trust_error_stays_small_enough_to_carry() {
    use std::mem::size_of;
    assert!(size_of::<TrustError>() <= 96, "{}", size_of::<TrustError>());
    assert!(size_of::<WriteError>() <= 96, "{}", size_of::<WriteError>());
}
