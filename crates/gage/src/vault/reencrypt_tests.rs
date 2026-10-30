//! Atomicity and dirty-tree tests for re-encryption.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use uuid::Uuid;

use std::panic::{AssertUnwindSafe, catch_unwind, panic_any, resume_unwind};
use std::sync::Mutex;

use crate::crypto::identity::Identity;
use crate::entry::{Entry, NewEntry};
use crate::prompter::FakePrompter;
use crate::storage::vaultconfig::Recipient;
use crate::sync::gitrepo;
use crate::sync::remotesync::tests::lock_is_held_elsewhere;
use crate::trust::recipient::recipient_tests::{Fixture, commit_paths, new_key};
use crate::vault::Vault;

// --- the crash seam ---------------------------------------------------------

/// The panic payload the crash seam raises. A panic rather than an error,
/// and that is the point: an error would run the write's own
/// error path, and a test would then prove that cleanup works rather than
/// that an interrupted run is invisible. Unwinding runs only `Drop`s, the
/// lock's included, which is what a killed process leaves behind.
pub(crate) struct SimulatedCrash;

/// `vault`, dying once `n` entries have been re-encrypted into the working
/// tree.
pub(crate) fn crashing_after(vault: &Vault, n: usize) -> Vault {
    vault.clone().with_reencrypt_hook(move |done| {
        if done >= n {
            panic_any(SimulatedCrash);
        }
    })
}

/// Runs `f` and reports whether it died on the simulated crash. Any other
/// panic is re-raised, so a real bug is not swallowed as "the crash we
/// asked for".
pub(crate) fn crashed(f: impl FnOnce()) -> bool {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(()) => false,
        Err(payload) if payload.is::<SimulatedCrash>() => true,
        Err(other) => resume_unwind(other),
    }
}

fn phone() -> (age::x25519::Identity, String) {
    let key = new_key();
    let public = key.to_public().to_string();
    (key, public)
}

fn insert(f: &Fixture, title: &str, p: &FakePrompter) -> Uuid {
    f.vault
        .insert(
            &f.identity(),
            NewEntry {
                title: title.into(),
                value: "v".into(),
                ..NewEntry::default()
            },
            true,
            p,
        )
        .unwrap()
}

// --- the dirty-tree reset -----------------------------------------

/// One write verb, driven over a vault whose `existing` entry it may touch.
type Write = fn(&Fixture, &Identity, Uuid, &FakePrompter);

fn write_verbs() -> Vec<(&'static str, Write)> {
    vec![
        ("insert", |f, id, _, p| {
            f.vault
                .insert(
                    id,
                    NewEntry {
                        title: "new".into(),
                        value: "v".into(),
                        ..NewEntry::default()
                    },
                    true,
                    p,
                )
                .unwrap();
        }),
        ("edit", |f, id, existing, p| {
            let e = Entry {
                title: "edited".into(),
                ..f.read(existing, id).unwrap()
            };
            f.vault.update(existing, &e, p).unwrap();
        }),
        ("rename", |f, id, existing, p| {
            f.vault
                .rename(id, &existing.to_string(), "renamed", true, p)
                .unwrap();
        }),
        ("remove", |f, id, existing, p| {
            f.vault.remove(id, &existing.to_string(), p).unwrap();
        }),
    ]
}

// Every write verb,
// since the requirement is that the precondition runs on all of them.
// `generate` writes through `insert`. The dirtied entry is one the write
// never touches, so a surviving change can only be the stale one folded in.
#[test]
fn dirty_work_tree_is_reset_before_any_subsequent_write() {
    for (name, write) in write_verbs() {
        let f = Fixture::new();
        let id = f.identity();
        let ids = f.seed(2);
        let stale = f.dirty_entry(ids[1]);
        let before = f.commits();

        write(&f, &id, ids[0], &FakePrompter::new());

        assert_eq!(f.commits(), before + 1, "{name}: the reset made a commit");
        let paths = commit_paths(&f.vault.path, &f.head());
        assert!(
            !paths.contains(&stale),
            "{name}: its commit folded in the stale {stale}: {paths:?}"
        );
        assert!(f.is_clean(), "{name}: the tree is still dirty");
        // Restored from HEAD, not deleted.
        f.read(ids[1], &id)
            .unwrap_or_else(|e| panic!("{name}: the dirtied entry no longer decrypts: {e}"));
    }
}

// One line,
// through the prompter, naming the path; a clean tree stays silent.
#[test]
fn dirty_work_tree_reset_warns_naming_the_discarded_paths() {
    let f = Fixture::new();
    let ids = f.seed(2);
    let stale = f.dirty_entry(ids[1]);
    // An orphan an interrupted insert leaves: untracked, and named too.
    let orphan = format!("entries/{}.age", crate::entry::new_id());
    std::fs::write(f.vault.path.join(&orphan), b"x").unwrap();

    let p = FakePrompter::new();
    write_verbs()[0].1(&f, &f.identity(), ids[0], &p);

    let warnings = p.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    for want in [&stale, &orphan] {
        assert!(warnings[0].contains(want.as_str()), "{warnings:?}");
    }
    assert!(!warnings[0].contains('\n'), "{warnings:?}");
    assert!(!f.vault.path.join(&orphan).exists());

    let p = FakePrompter::new();
    write_verbs()[0].1(&f, &f.identity(), ids[0], &p);
    assert!(
        p.warnings().is_empty(),
        "a clean tree warned: {:?}",
        p.warnings()
    );
}

// A
// reset that discarded something moved the working tree, so it raises the
// observer, and a write over a clean tree doesn't.
#[test]
fn a_reset_that_discards_something_tells_the_pull_observer() {
    let f = Fixture::new();
    let ids = f.seed(1);
    let flag = Arc::new(AtomicBool::new(false));
    let vault = f.vault.clone().with_pull_observer(Arc::clone(&flag));
    let rename = |title: &str| {
        vault
            .rename(
                &f.identity(),
                &ids[0].to_string(),
                title,
                true,
                &FakePrompter::new(),
            )
            .unwrap();
    };

    rename("clean");
    assert!(!flag.load(Ordering::SeqCst), "a clean write raised it");

    f.dirty_entry(ids[0]);
    rename("after a reset");
    assert!(flag.load(Ordering::SeqCst));
}

// --- re-encryption atomicity ---------------------------------------

// HEAD right after the
// crash (a commit id is a hash of its content, so equal ids are
// byte-identical commits), and the tree after the next write, which is
// where the leftover goes away.
#[test]
fn interrupted_reencrypt_leaves_head_untouched() {
    let f = Fixture::new();
    f.seed(4);
    let (_, phone) = phone();
    let (head, commits) = (f.head(), f.commits());

    let v = crashing_after(&f.vault, 2);
    assert!(
        crashed(|| {
            let _ = v.add_recipient("phone-1", &phone, &f.identity(), &FakePrompter::new());
        }),
        "the crash seam never fired"
    );

    assert_eq!(f.head(), head);
    assert_eq!(f.commits(), commits);
    assert!(
        !f.recipients_file().contains(&phone),
        "the recipient files must be written last"
    );
    // The leftover that makes the next write's reset load-bearing.
    assert!(!gitrepo::dirty_paths(&f.vault.path).unwrap().is_empty());

    insert(&f, "next write", &FakePrompter::new());
    assert!(f.is_clean(), "the next write left entries/ dirty");
}

// The retry starts
// from scratch, and the abandoned attempt contributed nothing to history.
#[test]
fn retrying_reencrypt_after_an_interruption_succeeds() {
    let f = Fixture::new();
    let ids = f.seed(4);
    let (key, phone) = phone();
    let base = f.commits();

    let v = crashing_after(&f.vault, 2);
    assert!(crashed(|| {
        let _ = v.add_recipient("phone-1", &phone, &f.identity(), &FakePrompter::new());
    }));

    let change = f
        .vault
        .add_recipient("phone-1", &phone, &f.identity(), &FakePrompter::new())
        .unwrap();
    assert_eq!(change.reencrypted, ids.len());
    assert_eq!(f.commits(), base + 1);
    let as_phone = Identity::for_tests(&key);
    for id in ids {
        f.read(id, &as_phone).unwrap();
    }
}

// Produced
// honestly: one entry's ciphertext replaced and committed, so the reset
// can't undo the setup. The error names it, nothing is committed, and no
// intact entry is half-migrated to the never-added key.
#[test]
fn reencrypt_aborts_whole_operation_when_one_entry_fails() {
    let f = Fixture::new();
    let ids = f.seed(3);
    let broken = ids[1];
    std::fs::write(
        f.vault.path.join(format!("entries/{broken}.age")),
        b"not an age file at all",
    )
    .unwrap();
    gitrepo::commit_all(&f.vault.path, "test: corrupt one entry").unwrap();
    let (key, phone) = phone();
    let (head, commits) = (f.head(), f.commits());

    let err = f
        .vault
        .add_recipient("phone-1", &phone, &f.identity(), &FakePrompter::new())
        .unwrap_err();

    assert!(err.to_string().contains(&broken.to_string()), "{err}");
    assert_eq!(f.head(), head);
    assert_eq!(f.commits(), commits);
    assert!(!f.recipients_file().contains(&phone));
    assert!(!f.config_keys().contains(&phone));
    assert!(f.is_clean(), "a decrypt failure wrote something");
    let as_phone = Identity::for_tests(&key);
    for id in ids.into_iter().filter(|id| *id != broken) {
        assert!(
            matches!(
                f.read(id, &as_phone),
                Err(crate::entry::Error::Crypt(
                    crate::crypto::crypt::Error::NotARecipient
                ))
            ),
            "{id} was migrated"
        );
    }
}

#[test]
fn reencrypt_lands_everything_in_exactly_one_commit() {
    let f = Fixture::new();
    let ids = f.seed(3);
    let before = f.commits();
    f.vault
        .add_recipient("phone-1", &phone().1, &f.identity(), &FakePrompter::new())
        .unwrap();

    assert_eq!(f.commits(), before + 1);
    let paths = commit_paths(&f.vault.path, &f.head());
    for want in [".age-recipients", ".gage/config.toml"] {
        assert!(paths.contains(&want.to_string()), "{paths:?}");
    }
    for id in ids {
        assert!(paths.contains(&format!("entries/{id}.age")), "{paths:?}");
    }
    assert!(f.is_clean());
}

// From inside the pass, at
// every point an interleaving implementation would have let go, another
// process finds the lock taken.
#[test]
fn reencrypt_holds_the_vault_lock_throughout() {
    let f = Fixture::new();
    f.seed(3);
    let lock = f.vault.lock_file_path();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    let probe = lock.clone();
    let v = f.vault.clone().with_reencrypt_hook(move |done| {
        record
            .lock()
            .unwrap()
            .push((done, lock_is_held_elsewhere(&probe)));
    });

    v.add_recipient("phone-1", &phone().1, &f.identity(), &FakePrompter::new())
        .unwrap();

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3, "{seen:?}");
    assert!(seen.iter().all(|(_, held)| *held), "{seen:?}");
    // And the probe can tell: once the add is done the lock is free.
    assert!(!lock_is_held_elsewhere(&lock));
}

// The window
// the entry hook can't reach: after the recipient files are written, before
// the commit. Reproduced through the real `write_recipient_files`. Both
// shapes, since they fail differently: a re-encryption's leaves entries
// dirty too; a plain add's leaves only the pair, which a reset keyed off
// entries/ would skip entirely.
#[test]
fn interrupted_recipient_write_leaves_no_half_migrated_state() {
    for dirty_an_entry in [true, false] {
        let f = Fixture::new();
        let ids = f.seed(2);
        let (key, phone) = phone();
        let mut must_not_commit = vec![".age-recipients".to_string(), ".gage/config.toml".into()];
        if dirty_an_entry {
            must_not_commit.push(f.dirty_entry(ids[1]));
        }
        let mut half_migrated = f.vault.recipients().unwrap();
        half_migrated.push(Recipient {
            device: "phone-1".into(),
            pubkey: phone.clone(),
        });
        f.vault.write_recipient_files(&half_migrated).unwrap();
        // The state under test exists, or everything below is vacuous.
        assert!(f.recipients_file().contains(&phone));
        assert!(f.config_keys().contains(&phone));

        let p = FakePrompter::new();
        let before = f.commits();
        let new = insert(&f, "after the crash", &p);

        let shape = if dirty_an_entry {
            "reencrypt"
        } else {
            "plain add"
        };
        assert!(!f.recipients_file().contains(&phone), "{shape}");
        assert!(!f.config_keys().contains(&phone), "{shape}");
        assert_eq!(f.commits(), before + 1, "{shape}");
        let paths = commit_paths(&f.vault.path, &f.head());
        for unwanted in &must_not_commit {
            assert!(
                !paths.contains(unwanted),
                "{shape}: {unwanted} in {paths:?}"
            );
        }
        // The reset ran before the write encrypted, not merely before it
        // committed: otherwise the never-added device could read this.
        assert!(
            matches!(
                f.read(new, &Identity::for_tests(&key)),
                Err(crate::entry::Error::Crypt(
                    crate::crypto::crypt::Error::NotARecipient
                ))
            ),
            "{shape}: the new entry is readable by a device never added"
        );
        let warned = p.warnings().join("\n");
        for want in [".age-recipients", ".gage/config.toml"] {
            assert!(warned.contains(want), "{shape}: {warned:?}");
        }
        assert!(f.is_clean(), "{shape}");
    }
}

// The next re-encryption
// cleans up after the last: the dirtied entry is in its commit, as every
// entry is, but freshly re-encrypted, not as the half-written bytes.
#[test]
fn dirty_work_tree_reset_is_also_run_by_reencrypt() {
    let f = Fixture::new();
    let ids = f.seed(2);
    let stale = f.dirty_entry(ids[1]);
    let before = f.commits();
    let p = FakePrompter::new();

    f.vault
        .add_recipient("phone-1", &phone().1, &f.identity(), &p)
        .unwrap();

    assert_eq!(f.commits(), before + 1);
    let e = f.read(ids[1], &f.identity()).unwrap();
    assert_eq!(e.title, "entry-b", "the half-written bytes survived");
    assert!(
        p.warnings().iter().any(|w| w.contains(&stale)),
        "{:?}",
        p.warnings()
    );
}
