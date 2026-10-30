//! The metadata index through `Session`, against real vaults: built once,
//! updated incrementally, discarded with the key, never holding a secret
//! and never written anywhere. These go
//! through a session; the arena's own rows (growth, discard) are in
//! `index`. Decrypts are counted on the real
//! path (`Vault::decrypt_count`).

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use uuid::Uuid;

use crate::crypto::locker::{Locker, MemLocker};
use crate::entry::{Entry, NewEntry, ResolveError};
use crate::prompter::FakePrompter;
use crate::session::{Session, SessionConfig, SessionError, SystemClock};
use crate::session_tests::{Cfg, Device, StepClock, current_personal, unlocks};

impl Device {
    fn decrypts(&self, name: &str) -> usize {
        self.vaults[name].decrypt_count()
    }

    /// A session whose vaults page-lock through `locker` instead.
    fn session_with_locker<'a>(
        &'a self,
        prompter: &'a FakePrompter,
        locker: Arc<dyn Locker>,
    ) -> Session<'a> {
        Session::new(SessionConfig {
            open: Box::new(move |name| self.open(name).map(|v| v.with_locker(locker.clone()))),
            prompter,
            idle_timeout: None,
            current: Some("personal".into()),
            clock: &SystemClock,
        })
    }
}

/// Inserts `title` (with `description` and `value`) through the session's
/// own key, telling the index nothing: an out-of-band write.
fn insert(s: &mut Session, title: &str, description: &str, value: &str) -> Uuid {
    let (v, ident) = s.vault(None).unwrap();
    v.insert(
        ident,
        NewEntry {
            title: title.into(),
            description: description.into(),
            value: value.into(),
            ..NewEntry::default()
        },
        true,
        &FakePrompter::new(),
    )
    .unwrap()
}

// --- build once -----------------------------------------------------------------

// The first list decrypts
// every entry once; a second decrypts nothing; a resolve decrypts only
// the entry it returns.
#[test]
fn first_list_builds_index_with_one_decrypt_pass() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS", "GitHub", "ProtonMail"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());

    let base = d.decrypts("personal");
    s.list(None).unwrap();
    assert_eq!(d.decrypts("personal") - base, 3, "one pass");
    s.list(None).unwrap();
    assert_eq!(
        d.decrypts("personal") - base,
        3,
        "the second list reused it"
    );
    let (_, e) = s.resolve(None, "AWS").unwrap();
    assert_eq!(e.title, "AWS");
    assert_eq!(
        d.decrypts("personal") - base,
        4,
        "3 for the index, 1 for the match"
    );
}

// A first resolve builds the index too, and the mutation commands don't:
// a vault that's only written to stays unindexed.
#[test]
fn a_first_resolve_builds_the_index_and_writes_do_not() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS", "GitHub"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());

    let id = insert(&mut s, "New", "", "v");
    s.refresh_entry(None, id);
    assert!(!s.has_index("personal"), "a write built an index");

    let base = d.decrypts("personal");
    s.resolve(None, "GitHub").unwrap();
    assert!(s.has_index("personal"));
    assert_eq!(
        d.decrypts("personal") - base,
        4,
        "3 for the index, 1 for the match"
    );
}

// The index is
// built from the same pass the body check needs.
#[test]
fn search_first_call_builds_index_without_a_double_decrypt() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS", "GitHub"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());

    let base = d.decrypts("personal");
    s.search(None, "aws").unwrap();
    assert_eq!(d.decrypts("personal") - base, 2);
    assert!(s.has_index("personal"));
}

// Body text is never
// cached: a second search decrypts again, a list in between doesn't.
#[test]
fn search_body_half_redecrypts_but_title_half_does_not() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS", "GitHub"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());

    let base = d.decrypts("personal");
    s.search(None, "aws").unwrap();
    assert_eq!(d.decrypts("personal") - base, 2);
    s.list(None).unwrap();
    assert_eq!(d.decrypts("personal") - base, 2, "list hit the index");
    s.search(None, "aws").unwrap();
    assert_eq!(d.decrypts("personal") - base, 4, "a fresh body pass");
}

#[test]
fn multi_vault_sessions_have_separate_indexes() {
    let d = Device::new(&["personal", "work"]);
    d.seed("personal", &["Personal Entry"]);
    d.seed("work", &["Work Entry"]);
    let p = unlocks(2);
    let mut s = d.session(&p, Cfg::default());
    s.vault(Some("personal")).unwrap();
    s.vault(Some("work")).unwrap();

    let (bp, bw) = (d.decrypts("personal"), d.decrypts("work"));
    assert_eq!(s.list(Some("personal")).unwrap()[0].title, "Personal Entry");
    assert_eq!(
        (d.decrypts("personal") - bp, d.decrypts("work") - bw),
        (1, 0)
    );
    assert_eq!(s.list(Some("work")).unwrap()[0].title, "Work Entry");
    assert_eq!(
        (d.decrypts("personal") - bp, d.decrypts("work") - bw),
        (1, 1)
    );
}

// --- incremental updates ------------------------------------------------------------

// Insert pays its own
// decrypts (the duplicate-title check); noting the entry and listing
// after it add none.
#[test]
fn note_entry_updates_index_without_full_rebuild() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS", "GitHub"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());
    s.list(None).unwrap();

    let id = insert(&mut s, "ProtonMail", "", "v3");
    let after_insert = d.decrypts("personal");
    let stored = {
        let (v, ident) = s.vault(None).unwrap();
        v.read_entry(id, ident).unwrap()
    };
    let after_read = d.decrypts("personal");
    assert_eq!(after_read - after_insert, 1);

    s.note_entry(None, id, &stored);
    let rows = s.list(None).unwrap();
    assert_eq!(d.decrypts("personal"), after_read, "no rebuild");
    assert_eq!(rows.len(), 3);
    let row = rows
        .iter()
        .find(|r| r.id == id)
        .expect("noted entry listed");
    assert_eq!(row.created, stored.created, "the stored dates, not a guess");
}

// refresh_entry is note_entry for a write that returned only an id: one
// decrypt when the vault is indexed, none when it isn't, and an entry it
// can't read drops the index rather than leaving it stale.
#[test]
fn refresh_entry_reads_one_entry_only_when_indexed() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());

    let id = insert(&mut s, "Unindexed", "", "v");
    let base = d.decrypts("personal");
    s.refresh_entry(None, id);
    assert_eq!(d.decrypts("personal"), base, "no index, nothing to read");

    s.list(None).unwrap();
    let id = insert(&mut s, "Indexed", "", "v");
    let base = d.decrypts("personal");
    s.refresh_entry(None, id);
    assert_eq!(d.decrypts("personal") - base, 1);
    assert!(s.list(None).unwrap().iter().any(|r| r.title == "Indexed"));
    assert_eq!(d.decrypts("personal") - base, 1, "and no rebuild");

    s.refresh_entry(None, Uuid::new_v4());
    assert!(
        !s.has_index("personal"),
        "an unreadable entry left a stale index"
    );
    assert_eq!(s.list(None).unwrap().len(), 3, "rebuilt from the vault");
}

#[test]
fn session_resolve_sees_a_rename_after_note_entry() {
    let d = Device::new(&["personal"]);
    let ids = d.seed("personal", &["Old Title"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());
    s.list(None).unwrap();

    let renamed = {
        let (v, ident) = s.vault(None).unwrap();
        let id = v
            .rename(
                ident,
                &ids[0].to_string(),
                "New Title",
                false,
                &FakePrompter::new(),
            )
            .unwrap();
        v.read_entry(id, ident).unwrap()
    };
    s.note_entry(None, ids[0], &renamed);

    let (id, e) = s.resolve(None, "New Title").unwrap();
    assert_eq!((id, e.title.as_str()), (ids[0], "New Title"));
    let err = s.resolve(None, "Old Title").unwrap_err();
    assert!(
        matches!(err, SessionError::Resolve(ResolveError::NotFound { .. })),
        "{err}"
    );
}

#[test]
fn forget_entry_removes_from_index() {
    let d = Device::new(&["personal"]);
    let ids = d.seed("personal", &["AWS", "GitHub"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());
    s.list(None).unwrap();

    {
        let (v, ident) = s.vault(None).unwrap();
        v.remove(ident, &ids[0].to_string(), &FakePrompter::new())
            .unwrap();
    }
    s.forget_entry(None, ids[0]);

    assert!(s.list(None).unwrap().iter().all(|r| r.id != ids[0]));
    let err = s.resolve(None, &ids[0].to_string()).unwrap_err();
    assert!(
        matches!(err, SessionError::Resolve(ResolveError::NotFound { .. })),
        "{err}"
    );
}

// A change the
// session wasn't told about (a manual `git pull`, here a write with no
// note) is invisible until reindex.
#[test]
fn reindex_forces_full_rebuild_and_picks_up_out_of_band_changes() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());
    s.list(None).unwrap();

    let new = insert(&mut s, "GitHub", "", "v");
    assert_eq!(s.list(None).unwrap().len(), 1, "the index didn't notice");

    let base = d.decrypts("personal");
    s.reindex(None).unwrap();
    assert_eq!(d.decrypts("personal") - base, 2, "a full rebuild");
    let rows = s.list(None).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|r| r.id == new));
}

// --- agreement with one-shot ------------------------------------------------------------

// Two implementations, one
// renderer: the rows must match field for field, ties included.
#[test]
fn list_agrees_between_one_shot_and_session() {
    let d = Device::new(&["personal"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());
    insert(&mut s, "AWS root account", "prod", "x");
    insert(&mut s, "a", "", "y");
    insert(&mut s, "Café", "", "z");
    insert(&mut s, "a", "the duplicate", "w");

    let one_shot = {
        let (v, ident) = s.vault(None).unwrap();
        v.list(ident).unwrap()
    };
    let in_session = s.list(None).unwrap();
    assert_eq!(one_shot, in_session);
    assert_eq!(in_session.len(), 4);
    for r in &in_session {
        assert!(r.created.is_some() && r.updated.is_some(), "{r:?}");
        assert!(!r.updated_by.is_empty(), "{r:?}");
    }
}

fn search_fixture(s: &mut Session) -> [Uuid; 6] {
    [
        insert(s, "AWS root account", "", "x"),
        insert(s, "Cloud creds", "shared aws credentials", "y"),
        insert(s, "Unrelated", "", "the aws secret key is here"),
        // Title and body both match: the classification overlap.
        insert(s, "AWS backup", "", "aws-secret-value"),
        // Description and body both match.
        insert(s, "Ops", "aws notes", "aws again"),
        insert(s, "Nothing", "", "irrelevant"),
    ]
}

#[test]
fn search_matches_title_description_and_body() {
    let d = Device::new(&["personal"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());
    let [title, desc, body, both, _, none] = search_fixture(&mut s);

    let results = s.search(None, "aws").unwrap();
    let get = |id| results.iter().find(|r| r.id == id);
    for id in [title, desc, body, both] {
        assert!(get(id).is_some(), "{id} missing: {results:?}");
    }
    assert!(get(none).is_none());
    assert!(!get(title).unwrap().matched_body);
    assert!(!get(desc).unwrap().matched_body);
    assert!(get(body).unwrap().matched_body);
    assert!(!get(both).unwrap().matched_body, "a title match wins");
    let rendered = format!("{results:?}");
    assert!(!rendered.contains("secret key is here"), "{rendered}");
}

// The overlap entries are
// the ones that once came back classified differently by the two paths.
#[test]
fn search_agrees_between_one_shot_and_session() {
    let d = Device::new(&["personal"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());
    search_fixture(&mut s);

    for query in ["aws", "AWS", "root", "nothing", "no-such-match"] {
        let one_shot = {
            let (v, ident) = s.vault(None).unwrap();
            v.search(ident, query).unwrap()
        };
        assert_eq!(one_shot, s.search(None, query).unwrap(), "{query}");
    }
}

// --- discard ----------------------------------------------------------------------------

// The index doesn't outlive the key; the next
// list after re-unlocking rebuilds from scratch.
#[test]
fn lock_discards_index() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS"]);
    let p = unlocks(2);
    let mut s = d.session(&p, current_personal());
    s.list(None).unwrap();
    assert!(s.has_index("personal"));

    s.lock(Some("personal")).unwrap();
    assert!(!s.has_index("personal"), "lock left the index behind");
    assert!(d.ledger.live().is_empty(), "a page is still locked");

    s.use_vault("personal").unwrap();
    let base = d.decrypts("personal");
    s.list(None).unwrap();
    assert_eq!(d.decrypts("personal") - base, 1, "rebuilt");
}

#[test]
fn idle_timeout_discards_index_like_an_explicit_lock() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS"]);
    let p = unlocks(2);
    let clock = StepClock::new();
    let mut s = d.session(
        &p,
        Cfg {
            idle_timeout: Some(Duration::from_secs(60)),
            current: Some("personal"),
            clock: Some(&clock),
        },
    );
    s.list(None).unwrap();
    assert!(s.has_index("personal"));

    clock.advance(Duration::from_secs(120));
    assert!(!s.status()[0].unlocked);
    assert!(
        !s.has_index("personal"),
        "the key went, the metadata stayed"
    );
    assert!(d.ledger.live().is_empty());

    let base = d.decrypts("personal");
    s.list(None).unwrap();
    assert_eq!(
        d.decrypts("personal") - base,
        1,
        "rebuilt after the timeout"
    );
}

// Closing (and dropping) a session releases every index's pages along
// with every key's.
#[test]
fn closing_a_session_releases_its_indexes() {
    let d = Device::new(&["personal", "work"]);
    d.seed("personal", &["AWS"]);
    d.seed("work", &["Jira"]);
    let p = unlocks(2);
    let mut s = d.session(&p, Cfg::default());
    let base = d.ledger.locks().len();
    s.list(Some("personal")).unwrap();
    s.list(Some("work")).unwrap();
    assert_eq!(d.ledger.locks().len() - base, 4, "two keys, two arenas");

    s.close();
    assert!(!s.has_index("personal") && !s.has_index("work"));
    assert!(d.ledger.live().is_empty());
}

// --- what the arena holds ---------------------------------------------------------------

// After ls, show (the one path
// that decrypts a whole entry to return it) and search, the arena holds
// the title and none of the value or fields.
#[test]
fn index_arena_never_holds_a_secret_value() {
    const SENTINEL: &str = "sentinel-value-should-never-be-cached-93f7";
    let d = Device::new(&["personal"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());
    {
        let (v, ident) = s.vault(None).unwrap();
        v.insert(
            ident,
            NewEntry {
                title: "Sentinel Title".into(),
                description: "Sentinel Description".into(),
                value: SENTINEL.into(),
                fields: [("k".to_string(), format!("{SENTINEL}-field"))].into(),
                ..NewEntry::default()
            },
            false,
            &FakePrompter::new(),
        )
        .unwrap();
    }

    s.list(None).unwrap();
    s.resolve(None, "Sentinel Title").unwrap();
    s.search(None, "sentinel").unwrap();

    let arena = String::from_utf8_lossy(&s.index_arena("personal")).into_owned();
    assert!(arena.contains("Sentinel Title"), "vacuous");
    assert!(arena.contains("Sentinel Description"), "vacuous");
    assert!(
        !arena.contains(SENTINEL),
        "the index arena holds the secret value"
    );
    assert!(
        !arena.contains("-field"),
        "the index arena holds a field value"
    );
}

/// Counts lock calls, failing every one after the first `ok`.
struct FailsAfter {
    ok: usize,
    calls: AtomicUsize,
}

impl FailsAfter {
    fn new(ok: usize) -> Arc<FailsAfter> {
        Arc::new(FailsAfter {
            ok,
            calls: AtomicUsize::new(0),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Locker for FailsAfter {
    fn lock(&self, buf: &[u8]) -> io::Result<()> {
        if self.calls.fetch_add(1, Ordering::SeqCst) < self.ok {
            MemLocker.lock(buf)
        } else {
            Err(io::Error::other("FailsAfter: forced failure"))
        }
    }

    fn unlock(&self, buf: &[u8]) -> io::Result<()> {
        MemLocker.unlock(buf)
    }
}

// The vault's
// own seam, not a second one: building the index locks through it again.
#[test]
fn index_arena_locks_through_the_same_locker_as_the_identity() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS"]);
    let locker = FailsAfter::new(usize::MAX);
    let p = unlocks(1);
    let mut s = d.session_with_locker(&p, locker.clone());

    s.vault(None).unwrap();
    assert_eq!(locker.calls(), 1, "the identity's key");
    s.list(None).unwrap();
    assert_eq!(
        locker.calls(),
        2,
        "and the index's arena, through the same seam"
    );
}

fn index_warnings(p: &FakePrompter) -> Vec<String> {
    p.warnings()
        .into_iter()
        .filter(|w| w.contains("metadata index"))
        .collect()
}

// The key locks, the
// arena doesn't: one warning, however often the index is read or grows.
#[test]
fn index_page_lock_failure_warns_once_and_proceeds() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS"]);
    let p = unlocks(1);
    let mut s = d.session_with_locker(&p, FailsAfter::new(1));

    let (_, ident) = s.vault(None).unwrap();
    assert!(ident.page_locked());
    assert!(p.warnings().is_empty());

    assert_eq!(s.list(None).unwrap().len(), 1, "proceeds unlocked");
    assert_eq!(index_warnings(&p).len(), 1, "{:?}", p.warnings());
    assert!(index_warnings(&p)[0].starts_with("could not lock personal's metadata index"));
    assert!(index_warnings(&p)[0].contains("personal"));

    s.list(None).unwrap();
    // Growth past the first arena retries the lock, and fails again.
    let big = "x".repeat(crate::entry::index::ARENA_MIN_SIZE);
    let id = insert(&mut s, &big, "", "v");
    s.refresh_entry(None, id);
    assert_eq!(index_warnings(&p).len(), 1, "warned again");
}

// The key itself
// couldn't be locked and the unlock said so; the index doesn't try, and
// doesn't say it twice.
#[test]
fn index_does_not_warn_again_when_unlock_already_warned() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS"]);
    let locker = FailsAfter::new(0);
    let p = unlocks(1);
    let mut s = d.session_with_locker(&p, locker.clone());

    let (_, ident) = s.vault(None).unwrap();
    assert!(!ident.page_locked());
    assert_eq!(p.warnings().len(), 1, "the unlock's own warning");

    s.list(None).unwrap();
    assert_eq!(p.warnings().len(), 1, "{:?}", p.warnings());
    assert_eq!(locker.calls(), 1, "the arena never tried");
}

// The warning is once per index, not once per session: a lock and a
// rebuild may meet a different limit, so it can be said again.
#[test]
fn the_index_warning_resets_with_the_index() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS"]);
    let p = unlocks(2);
    // Locks: key ok, arena fails, key ok, arena fails.
    struct Alternating(AtomicUsize);
    impl Locker for Alternating {
        fn lock(&self, buf: &[u8]) -> io::Result<()> {
            if self.0.fetch_add(1, Ordering::SeqCst).is_multiple_of(2) {
                MemLocker.lock(buf)
            } else {
                Err(io::Error::other("arena refused"))
            }
        }
        fn unlock(&self, buf: &[u8]) -> io::Result<()> {
            MemLocker.unlock(buf)
        }
    }
    let mut s = d.session_with_locker(&p, Arc::new(Alternating(AtomicUsize::new(0))));
    s.list(None).unwrap();
    s.lock(None).unwrap();
    s.list(None).unwrap();
    assert_eq!(index_warnings(&p).len(), 2);
}

/// Every file under `root`, sorted.
fn files_under(root: &Path) -> BTreeSet<PathBuf> {
    fn walk(dir: &Path, out: &mut BTreeSet<PathBuf>) {
        for item in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = item.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.insert(path);
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(root, &mut out);
    out
}

// Building and using the index (list,
// search, reindex) creates no file under the config, data or state roots
// or the vault.
#[test]
fn index_never_written_to_disk() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS", "GitHub"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());
    s.vault(None).unwrap();

    let before = files_under(d._root.path());
    assert!(!before.is_empty(), "vacuous");
    s.list(None).unwrap();
    s.search(None, "aws").unwrap();
    s.reindex(None).unwrap();
    let after = files_under(d._root.path());
    assert_eq!(before, after);
}

// Every Entry an index caches came from a decrypt; an unrelated sanity
// check that the counter counts what it claims, so the rows above can't
// pass by counting nothing.
#[test]
fn the_decrypt_counter_counts_reads() {
    let d = Device::new(&["personal"]);
    let ids = d.seed("personal", &["One"]);
    let v = d.open("personal").unwrap();
    let ident = v.unlock(&unlocks(1)).unwrap();
    let base = d.decrypts("personal");
    let _: Entry = v.read_entry(ids[0], &ident).unwrap();
    assert_eq!(d.decrypts("personal") - base, 1);
}

/// Keeps a copy of every buffer it was asked to unlock.
#[derive(Default)]
struct RecordsReleases(std::sync::Mutex<Vec<Vec<u8>>>);

impl Locker for RecordsReleases {
    fn lock(&self, buf: &[u8]) -> io::Result<()> {
        MemLocker.lock(buf)
    }

    fn unlock(&self, buf: &[u8]) -> io::Result<()> {
        self.0.lock().unwrap().push(buf.to_vec());
        MemLocker.unlock(buf)
    }
}

// Opus review: the arena's own test proves discard zeroes before
// unlocking; this proves it through a session, for the key and the arena
// alike. Every page a lock releases holds nothing but zeros by then.
#[test]
fn every_page_a_session_releases_was_zeroed_first() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["Release Title"]);
    let locker = Arc::new(RecordsReleases::default());
    let p = unlocks(1);
    let mut s = d.session_with_locker(&p, locker.clone());
    s.list(None).unwrap();
    assert!(
        String::from_utf8_lossy(&s.index_arena("personal")).contains("Release Title"),
        "vacuous"
    );

    s.lock(None).unwrap();
    let released = locker.0.lock().unwrap().clone();
    assert_eq!(released.len(), 2, "the key and the arena");
    for buf in released {
        assert!(!buf.is_empty());
        assert!(
            buf.iter().all(|b| *b == 0),
            "a page was released holding data"
        );
    }
}

// Opus review: growth mid-session moves the page lock to the new arena and
// releases the old one's, observed on the OS-like ledger rather than by
// counting calls.
#[test]
fn growth_through_a_session_keeps_the_arena_locked() {
    let d = Device::new(&["personal"]);
    d.seed("personal", &["AWS"]);
    let p = unlocks(1);
    let mut s = d.session(&p, current_personal());
    s.vault(None).unwrap();
    let base = d.ledger.locks().len();

    s.list(None).unwrap();
    let id = insert(
        &mut s,
        &"x".repeat(crate::entry::index::ARENA_MIN_SIZE),
        "",
        "v",
    );
    s.refresh_entry(None, id);

    let locks = d.ledger.locks();
    assert_eq!(locks.len() - base, 2, "the first arena, then the grown one");
    let (first, grown) = (&locks[base], &locks[base + 1]);
    let live = d.ledger.live();
    assert!(grown.is_subset(&live), "the grown arena isn't locked");
    assert!(first.is_disjoint(&live), "the old arena's lock leaked");
    assert_eq!(s.list(None).unwrap().len(), 2);
}
