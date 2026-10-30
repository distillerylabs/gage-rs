//! The N-recipient form of the recipient write, and the two options
//! its commit gained (paths to delete, the failed-push clause). The push
//! clause rows live with the other push tests in `remotesync_tests`.
//!
//! Nothing here goes through a command: the form is crate-private and
//! enrollment's approval is its only other caller.

use uuid::Uuid;

use crate::crypto::identity::Identity;
use crate::entry::{NewEntry, WRITE_LOCK_TIMEOUT};
use crate::prompter::FakePrompter;
use crate::storage::vaultconfig::Recipient;
use crate::sync::gitrepo;
use crate::trust::recipient::recipient_tests::{
    DEVICE, Fixture, Untouched, commit_paths, commit_recipient_files, counting, new_key,
};
use crate::trust::recipient::{RecipientError, RecipientWrite};
use crate::trust::trustcache::TrustError;
use crate::vault::Vault;
use crate::vault::reencrypt_tests::{crashed, crashing_after};

const MESSAGE: &str = "gage: test batch (reencrypt)";

/// A new device: the list entry, and the key that proves it can read.
fn member(device: &str) -> (Recipient, age::x25519::Identity) {
    let key = new_key();
    let r = Recipient {
        device: device.into(),
        pubkey: key.to_public().to_string(),
    };
    (r, key)
}

/// The batch form under the lock and the dirty-tree reset, as enrollment's
/// approval calls it.
fn batch(
    vault: &Vault,
    adds: &[Recipient],
    write: &RecipientWrite<'_>,
    identity: &Identity,
    prompter: &FakePrompter,
) -> Result<(usize, String), RecipientError> {
    vault.with_vault_write(WRITE_LOCK_TIMEOUT, prompter, || {
        vault.add_recipient_locked(adds, write, identity, prompter)
    })
}

fn plain(
    vault: &Vault,
    adds: &[Recipient],
    f: &Fixture,
) -> Result<(usize, String), RecipientError> {
    batch(
        vault,
        adds,
        &RecipientWrite::message(MESSAGE),
        &f.identity(),
        &FakePrompter::new(),
    )
}

/// An ordinary tracked file, standing in for a pending request.
fn track(f: &Fixture, rel: &str) {
    let full = f.vault.path.join(rel);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(&full, b"pending").unwrap();
    gitrepo::commit_all(&f.vault.path, "test: track a file").unwrap();
}

fn message_of(f: &Fixture, hash: &str) -> String {
    let repo = git2::Repository::open(&f.vault.path).unwrap();
    let commit = repo
        .find_commit(git2::Oid::from_str(hash).unwrap())
        .unwrap();
    commit.message().unwrap().to_string()
}

/// Leaves the list changed under this device (a foreign key appended and
/// committed, as a pull would): the next encrypting write must ask.
fn unreviewed_list(f: &Fixture) {
    let mut list = f.vault.recipients().unwrap();
    list.push(member("foreign").0);
    commit_recipient_files(&f.vault, &list);
}

fn insert_one(f: &Fixture, p: &FakePrompter) -> Result<Uuid, crate::entry::WriteError> {
    f.vault.insert(
        &f.identity(),
        NewEntry {
            title: "next write".into(),
            value: "v".into(),
            ..NewEntry::default()
        },
        true,
        p,
    )
}

// --- the N-recipient form ------------------------------------------

#[test]
fn several_recipients_land_in_one_commit() {
    let f = Fixture::new();
    let ids = f.seed(3);
    let (a, a_key) = member("phone-1");
    let (b, b_key) = member("phone-2");
    let before = f.commits();

    let (reencrypted, commit) = plain(&f.vault, &[a.clone(), b.clone()], &f).unwrap();

    assert_eq!(reencrypted, 3);
    assert_eq!(f.commits(), before + 1, "one commit for the whole batch");
    let paths = commit_paths(&f.vault.path, &commit);
    assert!(paths.contains(&".age-recipients".to_string()), "{paths:?}");
    assert!(
        paths.contains(&".gage/config.toml".to_string()),
        "{paths:?}"
    );
    assert_eq!(paths.len(), 2 + 3, "{paths:?}");
    for key in [&a_key, &b_key] {
        let who = Identity::for_tests(key);
        for &id in &ids {
            f.read(id, &who).unwrap();
        }
    }
    for r in [&a, &b] {
        assert!(f.recipients_file().contains(&r.pubkey));
        assert!(f.config_keys().contains(&r.pubkey));
    }
    assert!(f.is_clean());
}

// Counted through the hook: the
// number of entry rewrites, not wall-clock time.
#[test]
fn one_reencryption_pass_for_the_batch() {
    let f = Fixture::new();
    f.seed(3);
    // Every entry write, summed: `counting` keeps the last running count,
    // which a second pass would leave unchanged.
    let writes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = writes.clone();
    let v = f.vault.clone().with_reencrypt_hook(move |_| {
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });

    plain(&v, &[member("phone-1").0, member("phone-2").0], &f).unwrap();

    assert_eq!(writes.load(std::sync::atomic::Ordering::SeqCst), 3);
}

#[test]
fn one_trust_question_for_the_batch() {
    let f = Fixture::new();
    f.seed(2);
    unreviewed_list(&f);
    let p = FakePrompter::new().with_recipient_answers(&[true]);

    batch(
        &f.vault,
        &[member("phone-1").0, member("phone-2").0],
        &RecipientWrite::message(MESSAGE),
        &f.identity(),
        &p,
    )
    .unwrap();

    assert_eq!(p.recipient_changes().len(), 1, "asked once for the batch");
}

#[test]
fn batch_takes_one_lock_and_one_dirty_tree_reset() {
    let f = Fixture::new();
    let ids = f.seed(2);
    f.dirty_entry(ids[0]);
    let p = FakePrompter::new();

    batch(
        &f.vault,
        &[member("phone-1").0, member("phone-2").0],
        &RecipientWrite::message(MESSAGE),
        &f.identity(),
        &p,
    )
    .unwrap();

    let discards: Vec<_> = p
        .warnings()
        .into_iter()
        .filter(|w| w.contains("discarded uncommitted changes"))
        .collect();
    assert_eq!(discards.len(), 1, "{discards:?}");
}

#[test]
fn a_duplicate_within_the_batch_is_refused() {
    let f = Fixture::new();
    f.seed(2);
    let (a, _) = member("phone-1");
    let same_key = Recipient {
        device: "phone-2".into(),
        pubkey: a.pubkey.clone(),
    };
    let (b, _) = member("phone-1");
    let same_name = [a.clone(), b];

    for (case, adds) in [
        ("key", vec![a.clone(), same_key]),
        ("name", same_name.to_vec()),
    ] {
        let still = Untouched::of(&f);
        let err = plain(&f.vault, &adds, &f).unwrap_err();
        assert!(
            matches!(err, RecipientError::RecipientExists { .. }),
            "{case}: {err:?}"
        );
        still.assert_still(&f);
        assert!(f.is_clean(), "{case}: something was written");
    }
}

#[test]
fn a_batch_member_already_in_the_vault_is_refused() {
    let f = Fixture::new();
    f.seed(1);
    let existing = Recipient {
        device: "other-name".into(),
        pubkey: f.pubkey(),
    };
    let still = Untouched::of(&f);

    let err = plain(&f.vault, &[member("phone-1").0, existing], &f).unwrap_err();

    match err {
        RecipientError::RecipientExists { device, pubkey } => {
            assert_eq!(device, DEVICE);
            assert_eq!(pubkey, Some(f.pubkey()));
        }
        other => panic!("{other:?}"),
    }
    still.assert_still(&f);
}

#[test]
fn add_recipients_locked_revalidates_every_batch_member() {
    let f = Fixture::new();
    f.seed(1);
    let still = Untouched::of(&f);
    let good = member("phone-1").0;

    let bad_name = Recipient {
        device: "../escape".into(),
        pubkey: member("x").0.pubkey,
    };
    let err = plain(&f.vault, &[good.clone(), bad_name], &f).unwrap_err();
    assert!(
        matches!(err, RecipientError::InvalidDeviceName(_)),
        "{err:?}"
    );

    let bad_key = Recipient {
        device: "phone-2".into(),
        pubkey: "not-a-key".into(),
    };
    let err = plain(&f.vault, &[good, bad_key], &f).unwrap_err();
    assert!(
        matches!(err, RecipientError::InvalidRecipient(_)),
        "{err:?}"
    );

    still.assert_still(&f);
    assert!(f.is_clean());
}

#[test]
fn add_recipient_is_the_one_recipient_caller() {
    let f = Fixture::new();
    f.seed(2);
    unreviewed_list(&f);
    let (r, _) = member("phone-1");
    let p = FakePrompter::new().with_recipient_answers(&[true]);

    let change = f
        .vault
        .add_recipient("phone-1", &r.pubkey, &f.identity(), &p)
        .unwrap();

    assert_eq!(
        message_of(&f, &change.commit),
        "gage: recipient add phone-1 (reencrypt)"
    );
    assert_eq!(p.recipient_changes().len(), 1);
    assert_eq!(change.reencrypted, 2);
    // The cache was regenerated: the operator's next write is not asked
    // about their own add (an unscripted fake would fail the question).
    insert_one(&f, &FakePrompter::new()).unwrap();
}

#[test]
fn extra_paths_are_deleted_in_the_same_commit() {
    let f = Fixture::new();
    f.seed(1);
    track(&f, "notes/pending.txt");
    let before = f.commits();
    let delete = ["notes/pending.txt".to_string()];

    let (_, commit) = batch(
        &f.vault,
        &[member("phone-1").0],
        &RecipientWrite {
            message: MESSAGE,
            delete_paths: &delete,
            push_clause: None,
        },
        &f.identity(),
        &FakePrompter::new(),
    )
    .unwrap();

    assert_eq!(f.commits(), before + 1);
    assert!(commit_paths(&f.vault.path, &commit).contains(&"notes/pending.txt".to_string()));
    assert!(!f.vault.path.join("notes/pending.txt").exists());
    assert!(f.is_clean());
}

#[test]
fn an_unsafe_delete_path_is_refused_before_anything_is_written() {
    let f = Fixture::new();
    f.seed(1);
    let still = Untouched::of(&f);
    let delete = ["../outside".to_string()];

    let err = batch(
        &f.vault,
        &[member("phone-1").0],
        &RecipientWrite {
            message: MESSAGE,
            delete_paths: &delete,
            push_clause: None,
        },
        &f.identity(),
        &FakePrompter::new(),
    )
    .unwrap_err();

    assert!(matches!(err, RecipientError::Git(_)), "{err:?}");
    still.assert_still(&f);
    assert!(f.is_clean(), "something was written before the refusal");
}

// --- atomicity ------------------------------------------------------

#[test]
fn interrupted_batch_leaves_head_untouched() {
    let f = Fixture::new();
    f.seed(3);
    track(&f, "notes/pending.txt");
    let still = Untouched::of(&f);
    let (a, _) = member("phone-1");
    let (b, _) = member("phone-2");
    let delete = ["notes/pending.txt".to_string()];
    let v = crashing_after(&f.vault, 2);

    assert!(
        crashed(|| {
            let _ = batch(
                &v,
                &[a.clone(), b.clone()],
                &RecipientWrite {
                    message: MESSAGE,
                    delete_paths: &delete,
                    push_clause: None,
                },
                &f.identity(),
                &FakePrompter::new(),
            );
        }),
        "the crash seam never fired"
    );

    still.assert_still(&f);
    for r in [&a, &b] {
        assert!(!f.recipients_file().contains(&r.pubkey));
    }
    assert!(
        f.vault.path.join("notes/pending.txt").exists(),
        "deleted before the entries were done"
    );
    assert!(!gitrepo::dirty_paths(&f.vault.path).unwrap().is_empty());
    insert_one(&f, &FakePrompter::new()).unwrap();
    assert!(f.is_clean());
}

// The second path is a
// directory, which cannot be removed as a file: the first deletion has
// already happened when this one fails.
#[test]
fn a_batch_whose_deletion_fails_commits_nothing() {
    let f = Fixture::new();
    f.seed(2);
    track(&f, "notes/pending.txt");
    std::fs::create_dir_all(f.vault.path.join("notes/dir")).unwrap();
    let (head, commits) = (f.head(), f.commits());
    let (a, _) = member("phone-1");
    let delete = ["notes/pending.txt".to_string(), "notes/dir".to_string()];

    let err = batch(
        &f.vault,
        std::slice::from_ref(&a),
        &RecipientWrite {
            message: MESSAGE,
            delete_paths: &delete,
            push_clause: None,
        },
        &f.identity(),
        &FakePrompter::new(),
    )
    .unwrap_err();

    // The removal itself failed, not the path check up front.
    assert!(
        matches!(
            err,
            RecipientError::Git(gitrepo::Error::Io { op: "removing", .. })
        ),
        "{err:?}"
    );
    assert_eq!(f.head(), head, "HEAD moved");
    assert_eq!(f.commits(), commits, "something was committed");
    assert!(!f.is_clean(), "the failed batch left no dirty surface");

    // The next write's reset discards all of it: the list is as HEAD had
    // it, and the deleted file is back.
    insert_one(&f, &FakePrompter::new()).unwrap();
    assert!(!f.recipients_file().contains(&a.pubkey));
    assert!(!f.config_keys().contains(&a.pubkey));
    assert!(f.vault.path.join("notes/pending.txt").exists());
    assert!(f.is_clean());
}

// The deletion is
// uncommitted, not undone; the next write's reset restores the path rather
// than folding the deletion into that write's commit.
#[test]
fn a_batch_that_fails_after_deleting_commits_nothing() {
    let f = Fixture::new();
    f.seed(2);
    track(&f, "notes/pending.txt");
    let (head, commits) = (f.head(), f.commits());
    let (a, _) = member("phone-1");
    let delete = ["notes/pending.txt".to_string()];
    let v = f.vault.clone().with_delete_hook(|done| {
        if done >= 1 {
            std::panic::panic_any(crate::vault::reencrypt_tests::SimulatedCrash);
        }
    });

    assert!(
        crashed(|| {
            let _ = batch(
                &v,
                std::slice::from_ref(&a),
                &RecipientWrite {
                    message: MESSAGE,
                    delete_paths: &delete,
                    push_clause: None,
                },
                &f.identity(),
                &FakePrompter::new(),
            );
        }),
        "the delete seam never fired"
    );

    assert_eq!(f.head(), head, "HEAD moved");
    assert_eq!(f.commits(), commits, "something was committed");
    assert!(
        !f.vault.path.join("notes/pending.txt").exists(),
        "the deletion was undone, not left uncommitted"
    );
    // The recipient files are written last, after the deletions: a crash
    // here has not yet put a key no committed entry is encrypted to into
    // either of them.
    let dirty = gitrepo::dirty_paths(&f.vault.path).unwrap();
    for file in [".age-recipients", ".gage/config.toml"] {
        assert!(
            !dirty.iter().any(|d| d == file),
            "{file} written: {dirty:?}"
        );
    }

    let p = FakePrompter::new();
    let id = insert_one(&f, &p).unwrap();
    assert!(
        f.vault.path.join("notes/pending.txt").exists(),
        "not restored"
    );
    assert!(f.is_clean());
    assert!(!f.recipients_file().contains(&a.pubkey));
    let paths = commit_paths(&f.vault.path, &f.head());
    assert_eq!(paths, vec![format!("entries/{id}.age")], "{paths:?}");
}

#[test]
fn a_declined_trust_question_aborts_the_whole_batch() {
    let f = Fixture::new();
    f.seed(2);
    unreviewed_list(&f);
    let still = Untouched::of(&f);
    let (counted, entries_written) = counting(&f.vault);
    let p = FakePrompter::new().with_recipient_answers(&[false]);

    let err = batch(
        &counted,
        &[member("phone-1").0, member("phone-2").0],
        &RecipientWrite::message(MESSAGE),
        &f.identity(),
        &p,
    )
    .unwrap_err();

    assert!(
        matches!(
            err,
            RecipientError::Trust(TrustError::ChangeDeclined { .. })
        ),
        "{err:?}"
    );
    still.assert_still(&f);
    assert_eq!(
        entries_written.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "entries were rewritten"
    );
    assert!(f.is_clean());
    // The cache wasn't regenerated: the list is still unreviewed.
    let again = FakePrompter::new().with_recipient_answers(&[false]);
    assert!(insert_one(&f, &again).is_err());
    assert_eq!(again.recipient_changes().len(), 1);
}
