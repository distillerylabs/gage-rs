//! Library tests for `mv` and `cp` between vaults.

use std::panic::panic_any;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::entry::{Entry, NewEntry, Timestamp};
use crate::error::{ErrorKind, HasKind};
use crate::movecopy::MoveError;
use crate::prompter::FakePrompter;
use crate::recipient_tests::Fixture;
use crate::reencrypt_tests::{SimulatedCrash, crashed};
use crate::remotesync::tests::{Holder, lock_is_held_elsewhere};
use crate::trustcache::TrustError;
use crate::trustcache_tests::{TrustVault, commit_routine_change, new_key, trusting, unlocking};
use crate::vault::Vault;

fn put(f: &Fixture, title: &str) -> Uuid {
    f.vault
        .insert(
            &f.identity(),
            NewEntry {
                title: title.into(),
                description: "the description".into(),
                value: "hunter2".into(),
                fields: [("user".to_string(), "me".to_string())].into(),
                ..NewEntry::default()
            },
            true,
            &FakePrompter::new(),
        )
        .unwrap()
}

fn titles(f: &Fixture) -> Vec<String> {
    f.vault
        .list(&f.identity())
        .unwrap()
        .into_iter()
        .map(|r| r.title)
        .collect()
}

// The destination
// is never unlocked: nothing in it is decrypted by the move, and a key
// that is only on the destination's recipient list reads the copy.
#[test]
fn move_removes_from_source_and_decrypts_at_destination() {
    let (src, dst) = (Fixture::new(), Fixture::new());
    let id = put(&src, "GitHub");

    let r = src
        .vault
        .move_to("GitHub", &dst.vault, &src.identity(), &FakePrompter::new())
        .unwrap();

    assert_eq!(r.source_id, id);
    assert!(matches!(
        src.read(id, &src.identity()),
        Err(crate::entry::Error::EntryNotFound { .. })
    ));
    assert!(titles(&src).is_empty());
    assert!(src.vault.entry_ids().unwrap().is_empty());
    assert_eq!(dst.vault.decrypt_count(), 0, "the destination was unlocked");
    let got = dst.read(r.dest_id, &dst.identity()).unwrap();
    assert_eq!(got.title, "GitHub");
    assert_eq!(got.value, "hunter2");
    assert_eq!(got.description, "the description");
    assert_eq!(got.fields.get("user").map(String::as_str), Some("me"));
    assert!(
        dst.read(r.dest_id, &src.identity()).is_err(),
        "the copy is encrypted to the destination's list, not the source's"
    );
    assert!(src.is_clean() && dst.is_clean());
}

#[test]
fn copy_leaves_source_and_adds_decryptable_copy() {
    let (src, dst) = (Fixture::new(), Fixture::new());
    let id = put(&src, "GitHub");

    let r = src
        .vault
        .copy_to("GitHub", &dst.vault, &src.identity(), &FakePrompter::new())
        .unwrap();

    assert_eq!(src.vault.entry_ids().unwrap(), vec![id]);
    let original = src.read(id, &src.identity()).unwrap();
    assert_eq!(
        (original.title.as_str(), original.value.as_str()),
        ("GitHub", "hunter2")
    );
    assert_eq!(titles(&dst), vec!["GitHub".to_string()]);
    assert_eq!(
        dst.read(r.dest_id, &dst.identity()).unwrap().value,
        "hunter2"
    );
    assert_ne!(r.dest_id, id);
}

// Each named by that vault's
// entry id, like insert and remove.
#[test]
fn move_produces_one_commit_in_each_vault() {
    let (src, dst) = (Fixture::new(), Fixture::new());
    put(&src, "GitHub");
    let (before_src, before_dst) = (src.commits(), dst.commits());

    let r = src
        .vault
        .move_to("GitHub", &dst.vault, &src.identity(), &FakePrompter::new())
        .unwrap();

    assert_eq!(src.commits(), before_src + 1);
    assert_eq!(dst.commits(), before_dst + 1);
    assert_eq!(head_message(&src), r.source_id.to_string());
    assert_eq!(head_message(&dst), r.dest_id.to_string());
}

fn head_message(f: &Fixture) -> String {
    let repo = git2::Repository::open(&f.vault.path).unwrap();
    let commit = repo.head().unwrap().peel_to_commit().unwrap();
    commit.message().unwrap().trim().to_string()
}

// A new entry: new id,
// new dates, this device as the last writer; the content carries over.
#[test]
fn move_gives_the_destination_copy_fresh_provenance() {
    let (src, dst) = (Fixture::new(), Fixture::new());
    let id = put(&src, "GitHub");
    let old = Timestamp::from_unix(1_000_000).unwrap();
    let aged = Entry {
        created: Some(old),
        updated: Some(old),
        updated_by: "someone-else".into(),
        ..src.read(id, &src.identity()).unwrap()
    };
    src.vault.update(id, &aged, &FakePrompter::new()).unwrap();

    let r = src
        .vault
        .copy_to("GitHub", &dst.vault, &src.identity(), &FakePrompter::new())
        .unwrap();

    let got = dst.read(r.dest_id, &dst.identity()).unwrap();
    assert_ne!(r.dest_id, id);
    assert!(got.created.unwrap() > old);
    assert!(got.updated.unwrap() > old);
    assert_eq!(got.updated_by, src.identity().device());
    assert_eq!(got.description, "the description");
    assert_eq!(got.value, "hunter2");
    assert_eq!(got.fields.get("user").map(String::as_str), Some("me"));
    assert_eq!(r.entry.created, got.created);
}

#[test]
fn move_allows_a_title_collision_in_the_destination() {
    let (src, dst) = (Fixture::new(), Fixture::new());
    put(&src, "GitHub");
    put(&dst, "GitHub");

    src.vault
        .move_to("GitHub", &dst.vault, &src.identity(), &FakePrompter::new())
        .unwrap();

    assert_eq!(
        titles(&dst),
        vec!["GitHub".to_string(), "GitHub".to_string()]
    );
}

#[test]
fn move_rejects_the_source_vault_as_its_own_destination() {
    let f = Fixture::new();
    let id = put(&f, "GitHub");
    let before = f.commits();

    let err = f
        .vault
        .move_to("GitHub", &f.vault, &f.identity(), &FakePrompter::new())
        .unwrap_err();

    assert!(matches!(err, MoveError::SameVault { .. }), "{err}");
    assert_eq!(err.kind(), ErrorKind::Usage);
    assert_eq!(f.vault.entry_ids().unwrap(), vec![id]);
    assert_eq!(f.commits(), before);
}

// One id under two names is one vault, and one lock: taking it twice
// would wait out the whole timeout, so it is refused up front.
#[test]
fn move_rejects_two_registrations_of_one_vault() {
    let f = Fixture::new();
    put(&f, "GitHub");
    let mut alias = f.vault.clone();
    alias.name = "alias".into();

    let started = Instant::now();
    let err = f
        .vault
        .copy_to("GitHub", &alias, &f.identity(), &FakePrompter::new())
        .unwrap_err();

    assert!(matches!(err, MoveError::SameVault { .. }), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "refused only after waiting for a lock"
    );
}

// --- trust and locks --------------------------------------------------------

fn with_name(v: &Vault, name: &str) -> Vault {
    let mut v = v.clone();
    v.name = name.into();
    v
}

// The source's
// list has an unreviewed change and isn't asked about; the destination's
// does and is.
#[test]
fn move_checks_the_destinations_trust_cache_not_the_sources() {
    let (s, d) = (TrustVault::new(), TrustVault::new());
    s.bootstrap();
    d.bootstrap();
    let id = s.unlock(&unlocking());
    s.insert(&id, "GitHub", &unlocking());

    // The source's list moved; the destination's did not. A prompter with
    // no recipient answers fails any question it is asked.
    commit_routine_change(&s.vault, "phone-1", &new_key());
    let p = unlocking();
    s.vault.copy_to("GitHub", &d.vault, &id, &p).unwrap();
    assert!(p.recipient_changes().is_empty(), "asked about the source");

    // Now the destination's moved: that is the one asked about.
    let shared = with_name(&d.vault, "shared");
    commit_routine_change(&shared, "phone-2", &new_key());
    let p = trusting(&[true]);
    s.vault.copy_to("GitHub", &shared, &id, &p).unwrap();
    let asked = p.recipient_changes();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].vault, "shared");
}

// Move checks the destination's trust cache, not the source's (first half):
// a destination this device has never looked at has nothing to compare
// against, so the first move into it asks nothing.
#[test]
fn the_first_move_into_an_unseen_destination_asks_nothing() {
    let (s, d) = (TrustVault::new(), TrustVault::new());
    let id = s.unlock(&unlocking());
    s.insert(&id, "GitHub", &unlocking());
    assert!(d.cache().is_none());

    // No recipient answers scripted: a question would fail the test.
    let p = unlocking();
    s.vault.copy_to("GitHub", &d.vault, &id, &p).unwrap();

    assert!(p.recipient_changes().is_empty());
    assert_eq!(d.vault.entry_ids().unwrap().len(), 1);
}

// The source
// is untouched and nothing is written to the destination.
#[test]
fn move_declining_the_trust_check_aborts_the_whole_operation() {
    let (s, d) = (TrustVault::new(), TrustVault::new());
    s.bootstrap();
    d.bootstrap();
    let id = s.unlock(&unlocking());
    let entry = s.insert(&id, "GitHub", &unlocking());
    commit_routine_change(&d.vault, "phone-1", &new_key());
    let (s_head, d_head, d_cache) = (s.head(), d.head(), d.cache());

    let p = trusting(&[false]);
    let err = s.vault.move_to("GitHub", &d.vault, &id, &p).unwrap_err();

    assert!(
        matches!(err, MoveError::Trust(TrustError::ChangeDeclined { .. })),
        "{err}"
    );
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert_eq!(s.vault.entry_ids().unwrap(), vec![entry]);
    assert_eq!(s.head(), s_head);
    assert_eq!(d.head(), d_head);
    assert!(d.vault.entry_ids().unwrap().is_empty());
    assert_eq!(d.cache(), d_cache, "a declined change moved the cache");
}

// The higher-id
// lock is held by another process, so whichever vault is taken first is
// the one found held while the call waits. It must be the lower id, in
// either argument order: that is what kills "order by argument".
#[test]
fn two_vault_locks_are_acquired_in_one_deterministic_order() {
    let (x, y) = (Fixture::new(), Fixture::new());
    let (lo, hi) = if x.vault.id < y.vault.id {
        (&x, &y)
    } else {
        (&y, &x)
    };
    let lo_path = lo.vault.prepare_lock_file().unwrap();
    let hi_holder = Holder::spawn(&hi.vault.prepare_lock_file().unwrap());

    for (a, b) in [(&lo.vault, &hi.vault), (&hi.vault, &lo.vault)] {
        std::thread::scope(|s| {
            let call = s.spawn(|| {
                Vault::with_two_vault_locks(
                    a,
                    b,
                    Duration::from_secs(3),
                    &FakePrompter::new(),
                    || Ok::<(), MoveError>(()),
                )
            });
            let deadline = Instant::now() + Duration::from_secs(2);
            while !lock_is_held_elsewhere(&lo_path) && Instant::now() < deadline {}
            assert!(
                lock_is_held_elsewhere(&lo_path),
                "the lower id wasn't taken first while waiting on the higher"
            );
            let err = call.join().unwrap().unwrap_err();
            assert!(matches!(err, MoveError::Lock(_)), "{err}");
        });
        assert!(
            !lock_is_held_elsewhere(&lo_path),
            "a failed acquire left the first lock held"
        );
    }
    drop(hi_holder);
}

// Two-vault locks are acquired in one deterministic order (ADR-0047), a second angle: opposite-direction callers racing. Each
// `flock` is on its own open file description, so threads contend as
// processes do. A pair that took the locks in argument order would each
// hold one and wait on the other until the (short) timeout, and the
// timeout is an error here.
#[test]
fn opposite_direction_callers_never_deadlock() {
    let (x, y) = (Fixture::new(), Fixture::new());
    std::thread::scope(|s| {
        for _ in 0..25 {
            let forward = s.spawn(|| {
                Vault::with_two_vault_locks(
                    &x.vault,
                    &y.vault,
                    Duration::from_secs(5),
                    &FakePrompter::new(),
                    || Ok::<(), MoveError>(()),
                )
            });
            let backward = s.spawn(|| {
                Vault::with_two_vault_locks(
                    &y.vault,
                    &x.vault,
                    Duration::from_secs(5),
                    &FakePrompter::new(),
                    || Ok::<(), MoveError>(()),
                )
            });
            forward.join().unwrap().unwrap();
            backward.join().unwrap().unwrap();
        }
    });
}

// From the window
// between the two writes, another process finds both locks taken: that is
// what kills releasing the first lock early.
#[test]
fn two_vault_locks_are_both_held_for_the_whole_operation() {
    let (src, dst) = (Fixture::new(), Fixture::new());
    put(&src, "GitHub");
    let (src_lock, dst_lock) = (src.vault.lock_file_path(), dst.vault.lock_file_path());
    let seen = Arc::new(Mutex::new(None));
    let record = Arc::clone(&seen);
    let (a, b) = (src_lock.clone(), dst_lock.clone());
    let v = src.vault.clone().with_move_hook(move || {
        *record.lock().unwrap() = Some((lock_is_held_elsewhere(&a), lock_is_held_elsewhere(&b)));
    });

    v.move_to("GitHub", &dst.vault, &src.identity(), &FakePrompter::new())
        .unwrap();

    assert_eq!(*seen.lock().unwrap(), Some((true, true)));
    assert!(!lock_is_held_elsewhere(&src_lock));
    assert!(!lock_is_held_elsewhere(&dst_lock));
}

// --- crash and session ------------------------------------------------------

// Dying
// right after the destination's commit leaves the entry in both: a
// duplicate to clean up, never a loss. Kills "remove from the source
// before committing the destination".
#[test]
fn move_crash_between_the_two_writes_leaves_a_recoverable_duplicate() {
    let (src, dst) = (Fixture::new(), Fixture::new());
    let id = put(&src, "GitHub");
    let (src_head, src_commits) = (src.head(), src.commits());
    let v = src
        .vault
        .clone()
        .with_move_hook(|| panic_any(SimulatedCrash));

    let died = crashed(|| {
        let _ = v.move_to("GitHub", &dst.vault, &src.identity(), &FakePrompter::new());
    });

    assert!(died, "the crash seam never fired");
    assert_eq!(src.vault.entry_ids().unwrap(), vec![id]);
    assert_eq!(src.head(), src_head);
    assert_eq!(src.commits(), src_commits);
    let copies = dst.vault.entry_ids().unwrap();
    assert_eq!(copies.len(), 1, "exactly the duplicate");
    let dup = dst.read(copies[0], &dst.identity()).unwrap();
    assert_eq!(
        (dup.title.as_str(), dup.value.as_str()),
        ("GitHub", "hunter2")
    );
    assert!(dst.is_clean() && src.is_clean());
    // The locks died with the unwind: a retry is not blocked.
    assert!(!lock_is_held_elsewhere(&src.vault.lock_file_path()));
    assert!(!lock_is_held_elsewhere(&dst.vault.lock_file_path()));
}

// A write error on the destination leaves the source alone, too.
#[test]
fn a_move_that_cannot_resolve_its_query_touches_nothing() {
    let (src, dst) = (Fixture::new(), Fixture::new());
    put(&src, "GitHub");
    let (s, d) = (src.commits(), dst.commits());

    let err = src
        .vault
        .move_to("nope", &dst.vault, &src.identity(), &FakePrompter::new())
        .unwrap_err();

    assert!(matches!(err, MoveError::Resolve(_)), "{err}");
    assert_eq!((src.commits(), dst.commits()), (s, d));
}
