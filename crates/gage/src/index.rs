//! A session's decrypt-once cache of one vault's metadata: title,
//! description, dates and `updated_by`. Never a value, never a field, and
//! never written anywhere. It belongs to `Session` (never `Vault`, which
//! stays stateless) and is discarded with the `Identity` that built it.
//! See "Addressing entries & the metadata index" in the design doc and
//! ADR-0029.
//!
//! Titles and descriptions live in one page-locked arena per index rather
//! than in `String`s, which the allocator can copy and free without
//! wiping. One arena rather than one `LockedBuf` per entry because each
//! `LockedBuf` takes at least a page of its own. Spans point into it; a
//! span that stops being live (a removed entry, a renamed title) is zeroed
//! at once, and growing copies only live spans.

use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;

use crate::entry::{Entry, ListEntry, Timestamp};
use crate::locker::Locker;
use crate::memlock::LockedBuf;
use crate::search::{SearchResult, matches_text};

/// The smallest arena allocated: small enough that a near-empty vault
/// doesn't reserve pages for nothing, large enough that most vaults never
/// grow past it.
pub(crate) const ARENA_MIN_SIZE: usize = 4096;

/// A byte range in the arena. An empty string is the empty span and takes
/// no arena bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Span {
    off: usize,
    len: usize,
}

struct IndexEntry {
    title: Span,
    description: Span,
    created: Option<Timestamp>,
    updated: Option<Timestamp>,
    /// A device name: public in the vault's own config, so ordinary memory.
    updated_by: String,
}

pub(crate) struct Index {
    entries: HashMap<Uuid, IndexEntry>,
    arena: Option<LockedBuf>,
    used: usize,
    /// Whether the current arena's lock took.
    locked: bool,
    /// The vault's identity's own lock result when this index was built.
    /// If the key couldn't be page-locked, the same limit applies here, so
    /// the arena doesn't try, and doesn't warn about a failure the unlock
    /// already reported.
    lock_enabled: bool,
    locker: Arc<dyn Locker>,
}

impl Index {
    pub(crate) fn new(locker: Arc<dyn Locker>, lock_enabled: bool) -> Index {
        Index {
            entries: HashMap::new(),
            arena: None,
            used: 0,
            locked: false,
            lock_enabled,
            locker,
        }
    }

    /// Caches `id`'s metadata, replacing (and zeroing) whatever was cached
    /// for it before.
    pub(crate) fn put(&mut self, id: Uuid, e: &Entry) {
        self.remove(id);
        // Room for both first: a growth compacts only the spans already in
        // `entries`, so growing between the two stores would strand the
        // title stored first.
        self.ensure_capacity(e.title.len() + e.description.len());
        let title = self.store(e.title.as_bytes());
        let description = self.store(e.description.as_bytes());
        self.entries.insert(
            id,
            IndexEntry {
                title,
                description,
                created: e.created,
                updated: e.updated,
                updated_by: e.updated_by.clone(),
            },
        );
    }

    /// Drops `id`, zeroing its bytes.
    pub(crate) fn remove(&mut self, id: Uuid) {
        if let Some(old) = self.entries.remove(&id) {
            self.wipe(old.title);
            self.wipe(old.description);
        }
    }

    /// Every cached `(id, title)`, for the resolver. Transient copies made
    /// for one resolution: the arena is what the rule binds, not a
    /// match's own working set.
    pub(crate) fn titles(&self) -> Vec<(Uuid, String)> {
        self.entries
            .iter()
            .map(|(id, e)| (*id, self.text(e.title)))
            .collect()
    }

    /// Every cached entry as `ls` rows, sorted as `Vault::list` sorts.
    pub(crate) fn list(&self) -> Vec<ListEntry> {
        let mut rows: Vec<ListEntry> = self
            .entries
            .iter()
            .map(|(id, e)| ListEntry {
                id: *id,
                title: self.text(e.title),
                description: self.text(e.description),
                created: e.created,
                updated: e.updated,
                updated_by: e.updated_by.clone(),
            })
            .collect();
        crate::entry::sort_list(&mut rows);
        rows
    }

    /// The index-served half of `search`: entries whose title or
    /// description contains `lower_pattern`.
    pub(crate) fn match_text(&self, lower_pattern: &str) -> HashMap<Uuid, SearchResult> {
        self.entries
            .iter()
            .filter_map(|(id, e)| {
                let (title, description) = (self.text(e.title), self.text(e.description));
                matches_text(&title, &description, lower_pattern).then_some((
                    *id,
                    SearchResult {
                        id: *id,
                        title,
                        description,
                        matched_body: false,
                    },
                ))
            })
            .collect()
    }

    /// False only when locking the arena was attempted and failed: the one
    /// case worth a warning.
    pub(crate) fn lock_ok(&self) -> bool {
        !self.lock_enabled || self.arena.is_none() || self.locked
    }

    /// Releases the arena's page lock and drops it (which zeroes it).
    /// Idempotent; also what dropping the index does.
    pub(crate) fn discard(&mut self) {
        self.entries.clear();
        self.release_arena();
        self.used = 0;
    }

    /// A copy of the arena's bytes, for tests that look for what must not
    /// be there.
    #[cfg(test)]
    pub(crate) fn arena_bytes(&self) -> Vec<u8> {
        self.arena
            .as_ref()
            .map(|a| a.as_slice().to_vec())
            .unwrap_or_default()
    }

    /// A copy made for output: the arena's bytes stay in the arena.
    fn text(&self, span: Span) -> String {
        if span.len == 0 {
            return String::new();
        }
        let arena = self.arena.as_ref().expect("a non-empty span has an arena");
        String::from_utf8_lossy(&arena.as_slice()[span.off..span.off + span.len]).into_owned()
    }

    /// Appends `bytes` into room the caller already made.
    fn store(&mut self, bytes: &[u8]) -> Span {
        if bytes.is_empty() {
            return Span::default();
        }
        let arena = self.arena.as_mut().expect("put made room");
        let off = self.used;
        arena.as_mut_slice()[off..off + bytes.len()].copy_from_slice(bytes);
        self.used += bytes.len();
        Span {
            off,
            len: bytes.len(),
        }
    }

    fn wipe(&mut self, span: Span) {
        if let (Some(arena), true) = (self.arena.as_mut(), span.len > 0) {
            arena.as_mut_slice()[span.off..span.off + span.len].fill(0);
        }
    }

    /// Makes room for `n` more bytes: a new, larger arena, locked, with the
    /// live spans copied in compactly; then the old one unlocked and
    /// dropped (zeroed). Never decrypts anything.
    fn ensure_capacity(&mut self, n: usize) {
        let capacity = self.arena.as_ref().map_or(0, LockedBuf::len);
        if n == 0 || self.used + n <= capacity {
            return;
        }
        let live: usize = self
            .entries
            .values()
            .map(|e| e.title.len + e.description.len)
            .sum();
        let mut size = capacity.max(ARENA_MIN_SIZE);
        while size < live + n {
            size *= 2;
        }
        let mut grown = LockedBuf::zeroed(size);
        let mut used = 0;
        if let Some(old) = &self.arena {
            let (from, to) = (old.as_slice(), grown.as_mut_slice());
            for e in self.entries.values_mut() {
                for span in [&mut e.title, &mut e.description] {
                    if span.len > 0 {
                        to[used..used + span.len]
                            .copy_from_slice(&from[span.off..span.off + span.len]);
                        span.off = used;
                        used += span.len;
                    }
                }
            }
        }
        let locked = self.lock_enabled && self.locker.lock(grown.as_slice()).is_ok();
        self.release_arena();
        self.arena = Some(grown);
        self.locked = locked;
        self.used = used;
    }

    /// Zeroes the arena, then unlocks its pages if they were locked, then
    /// drops it (whose `Drop` wipes the page slack too and frees). Zeroing
    /// first, as `Identity` does, means the bytes are never readable from
    /// a page that has already lost its lock.
    fn release_arena(&mut self) {
        if let Some(mut arena) = self.arena.take() {
            arena.as_mut_slice().fill(0);
            if self.locked {
                let _ = self.locker.unlock(arena.as_slice());
            }
            drop(arena);
        }
        self.locked = false;
    }
}

impl Drop for Index {
    fn drop(&mut self) {
        self.discard();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io;
    use std::sync::Mutex;

    use super::*;
    use crate::entry::resolve_query;

    /// Counts lock and unlock calls, keeping a copy of what each unlock
    /// was handed (so a test can see what an arena held as it was let go).
    #[derive(Default)]
    pub(crate) struct Counting {
        pub(crate) locks: Mutex<usize>,
        pub(crate) unlocked: Mutex<Vec<Vec<u8>>>,
    }

    impl Counting {
        pub(crate) fn locks(&self) -> usize {
            *self.locks.lock().unwrap()
        }

        pub(crate) fn unlocks(&self) -> usize {
            self.unlocked.lock().unwrap().len()
        }
    }

    impl Locker for Counting {
        fn lock(&self, _: &[u8]) -> io::Result<()> {
            *self.locks.lock().unwrap() += 1;
            Ok(())
        }

        fn unlock(&self, buf: &[u8]) -> io::Result<()> {
            self.unlocked.lock().unwrap().push(buf.to_vec());
            Ok(())
        }
    }

    fn entry(title: &str, description: &str) -> Entry {
        Entry {
            title: title.into(),
            description: description.into(),
            updated_by: "laptop-1".into(),
            ..Entry::default()
        }
    }

    fn id(i: u32) -> Uuid {
        Uuid::parse_str(&format!("{i:08x}-0000-4000-8000-000000000000")).unwrap()
    }

    // Several
    // doublings, each replacing an arena that must have been released, so
    // unlocks trail locks by exactly one until discard squares them.
    #[test]
    fn index_arena_growth_preserves_every_entry_and_its_page_locks() {
        let counter = Arc::new(Counting::default());
        let mut idx = Index::new(counter.clone(), true);
        const N: u32 = 400;
        let mut want = HashMap::new();
        for i in 0..N {
            let e = entry(
                &format!("title-{i:03}-{}", "t".repeat(20)),
                &format!("desc-{i:03}-{}", "d".repeat(30)),
            );
            idx.put(id(i), &e);
            want.insert(id(i), e);
        }

        assert!(idx.used > ARENA_MIN_SIZE, "never outgrew the first arena");
        assert!(counter.locks() >= 3, "grew {} times", counter.locks());
        assert_eq!(
            counter.unlocks(),
            counter.locks() - 1,
            "every arena but the current one was released"
        );
        assert!(idx.locked);

        let rows = idx.list();
        assert_eq!(rows.len(), N as usize);
        for row in &rows {
            let w = &want[&row.id];
            assert_eq!((&row.title, &row.description), (&w.title, &w.description));
        }
        // Resolution reads the spans through another path.
        let target = id(0x63);
        assert_eq!(
            resolve_query(&want[&target].title, &idx.titles()).unwrap(),
            target
        );

        idx.discard();
        assert_eq!(
            counter.unlocks(),
            counter.locks(),
            "the live arena's lock leaked"
        );
    }

    // Growth copies only live spans: a removed entry's bytes don't travel
    // into the next arena.
    #[test]
    fn growth_compacts_away_removed_entries() {
        let counter = Arc::new(Counting::default());
        let mut idx = Index::new(counter.clone(), true);
        idx.put(
            id(1),
            &entry("gone-title-marker", "gone-description-marker"),
        );
        idx.remove(id(1));
        idx.put(id(2), &entry(&"x".repeat(ARENA_MIN_SIZE), ""));
        assert_eq!(counter.locks(), 2, "the big title forced a growth");
        let arena = String::from_utf8_lossy(&idx.arena_bytes()).into_owned();
        assert!(!arena.contains("gone"), "a dead span was copied forward");
        assert_eq!(idx.used, ARENA_MIN_SIZE);
    }

    // An entry whose title fits but whose description forces a growth: the
    // compaction copies only spans already indexed, so the two must be
    // stored after one reservation, or the title is stranded in the old
    // layout (the bug the growth test above first caught).
    #[test]
    fn a_growth_between_title_and_description_keeps_the_title() {
        let mut idx = Index::new(Arc::new(Counting::default()), true);
        idx.put(id(1), &entry(&"a".repeat(ARENA_MIN_SIZE - 100), ""));
        idx.put(id(2), &entry("short title", &"d".repeat(200)));
        let rows = idx.list();
        let row = rows.iter().find(|r| r.id == id(2)).unwrap();
        assert_eq!(row.title, "short title");
        assert_eq!(row.description, "d".repeat(200));
    }

    // Replacing or removing an entry zeroes its old bytes at once, rather
    // than leaving stale plaintext until discard.
    #[test]
    fn orphaned_spans_are_zeroed_at_once() {
        let mut idx = Index::new(Arc::new(Counting::default()), true);
        idx.put(id(1), &entry("Old Title", "old description"));
        idx.put(id(1), &entry("New Title", ""));
        idx.put(id(2), &entry("Removed Title", ""));
        idx.remove(id(2));
        let arena = String::from_utf8_lossy(&idx.arena_bytes()).into_owned();
        assert!(arena.contains("New Title"), "vacuous");
        for gone in ["Old Title", "old description", "Removed Title"] {
            assert!(!arena.contains(gone), "{gone:?} is still in the arena");
        }
        assert_eq!(idx.list().len(), 1);
    }

    // The arena is zeroed
    // before its lock comes off (the unlock is handed nothing but zeros),
    // and a second discard unlocks nothing.
    #[test]
    fn index_discard_zeroes_arena_and_is_idempotent() {
        let counter = Arc::new(Counting::default());
        let mut idx = Index::new(counter.clone(), true);
        idx.put(Uuid::new_v4(), &entry("hello", "world"));
        assert!(idx.locked);
        assert!(
            String::from_utf8_lossy(&idx.arena_bytes()).contains("hello"),
            "vacuous"
        );

        idx.discard();
        assert!(idx.arena.is_none());
        assert!(idx.list().is_empty());
        let released = counter.unlocked.lock().unwrap()[0].clone();
        assert!(!released.is_empty());
        assert!(
            released.iter().all(|b| *b == 0),
            "the arena still held bytes when its lock came off"
        );
        idx.discard();
        assert_eq!(counter.unlocks(), 1, "unlocked once across two discards");
        // Dropping the index discards too, without a second unlock.
        drop(idx);
        assert_eq!(counter.unlocks(), 1);
    }

    // With the identity's own lock failed, the arena isn't locked at all
    // and reports nothing to warn about.
    #[test]
    fn an_index_whose_identity_failed_to_lock_never_tries() {
        let counter = Arc::new(Counting::default());
        let mut idx = Index::new(counter.clone(), false);
        idx.put(id(1), &entry("t", "d"));
        assert_eq!(counter.locks(), 0);
        assert!(idx.lock_ok());
        drop(idx);
        assert_eq!(counter.unlocks(), 0);
    }
}
