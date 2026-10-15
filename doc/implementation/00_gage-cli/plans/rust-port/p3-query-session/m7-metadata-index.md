# M7 — Metadata index

[← M6](m6-session-mode.md) · [P3 plan](index.md) · [#24](https://github.com/distillerylabs/gage-rs/issues/24) · next: [M13 — Tab completion & abbreviation](m13-tab-completion.md)

> **Recommended model: Sonnet.** A caching layer whose correctness proof
> is that M5's and M6's tests pass unmodified. **Run an Opus review pass
> over the index tests** (tasks 1–2): the arena keeps titles and
> descriptions resident for a whole session, and the properties that
> matter (no secret value in it, zeroed on discard, locks preserved
> across growth) are exactly the ones a passing suite is least likely to
> notice being wrong.

## Goal

A decrypt-once metadata index per vault, owned by `Session` and **never
by `Vault`**, serving `ls`, resolution and the title/description half of
`search`, updated incrementally on `insert`/`edit`/`rename`/`rm`/
`generate`, and discarded with the vault's `Identity`. It holds `title`,
`description`, `created`, `updated` and `updated_by`; never `value`,
never `fields`; never written to disk. Adds `search` (alias `grep`),
which also matches body text by decrypting on demand and dropping the
plaintext, and `reindex`. One-shot mode builds no index and keeps
decrypting fresh.

## Depends on

- **M6**: `Session` and `HeldVault`, the lock/idle-timeout path the index
  is discarded through.
- **M5**: the resolver the index feeds (`resolve_query` over `(id, title)`).
- **P2 · M2**: `LockedBuf`, the `Locker` seam and `Vault::with_locker`.
- **P2 · M4**: `Vault::list`, `ListEntry`, `render_ls`.

## Design references

- Design doc "Addressing entries & the metadata index" (why `ls` can't be
  cheap, what the index holds), "Library architecture" (the index belongs
  to `Session`), "A few decisions worth calling out" (in-memory,
  session-scoped, never on disk).
- [Q-INDEX-ARENA](../open-questions.md#q-index-arena).

## Go sources

- Go plan `gage-cli-design/m7-metadata-index.md`.
- Parity ledger rows: §3 M7; §4 `search`, `reindex`; §6 the index and
  `search` parts of `session-mode.md` and `entries.md`.
- Go sources at the pinned SHA: `internal/gage/index.go`,
  `internal/gage/search.go`, `internal/gage/session.go` (`List`,
  `Search`, `Reindex`, `NoteEntry`, `ForgetEntry`, `ensureIndex`,
  `buildIndexIfMissing`, `warnIndexLockOnce`, `discardIndex`),
  `cmd/gage/entry.go` (`search`, `reindex`, `writeSearchResults`,
  `noteIndexEntry`).
- Test inventory: `index` (20), `internal/gage/search_test.go` (2),
  `cmd/gage/search_test.go` (4): 26 rows. Plus 2 rows deferred by P2
  (task 4).

## Decisions to make first

Recommended answers; confirm them when this plan is reviewed.

- **Arena** (Q-INDEX-ARENA): one `LockedBuf` per vault index holding
  packed UTF-8 title and description bytes; each index entry is
  `{ id, title: Span, description: Span, created, updated, updated_by }`
  where `Span` is an offset and length into the arena. `updated_by` (a
  device name, public in the vault's config) and timestamps live in
  ordinary memory, as Go's. Strings handed to callers (`ListEntry`,
  candidate lists) are transient copies made for output, out of scope as
  in Go.
- **Growth**: when an insert doesn't fit, allocate a `LockedBuf` of
  double the needed capacity, lock it, copy the live spans in, then drop
  the old one (which zeroes it) after unlocking it through the `Locker`.
  A growth never decrypts. Removed or renamed spans are dead bytes until
  the next growth compacts them (Go's behavior); they are zeroed when
  orphaned, not left as stale plaintext.
- **Page locking** goes through the vault's `Arc<dyn Locker>`, the same
  one its `Identity` uses. A lock failure warns once through
  `Prompter::warn` and proceeds. No second warning if this vault's unlock
  already warned (the `Identity` records whether its lock took); the
  "warned" flag resets when the index is discarded.
- **Discard** is idempotent and driven by the same function that drops
  the `Identity` (lock, idle timeout, `Session` drop): unlock the pages,
  then drop the `LockedBuf` (zeroize, free).
- **Decrypt counting for tests**: a `#[cfg(any(test, feature = "test-support"))]`
  counter on `Vault`, bumped in the one decrypt path (`read_entry`),
  mirroring Go's `onDecrypt` hook. Not a seam, nothing to fake: it
  observes the real path.
- **Build trigger**: the first `list`, `resolve` or `search` against a
  vault builds the index in one full decrypt pass; a first `search`
  builds it from that same pass rather than decrypting twice. Mutations
  on a vault with no index leave it unbuilt.
- **Incremental update**: the CLI calls `Session::note_entry(vault, id, &Entry)`
  after `insert`/`edit`/`rename`/`generate` and
  `Session::forget_entry(vault, id)` after `rm`, as Go; a no-op when that
  vault has no index.
- **One render path for `ls`**: `Vault::list` and `Session::list` return
  the same `Vec<ListEntry>`, sorted the same way, through the one
  `render_ls`.
- **`search`**: case-insensitive substring over title and description
  (index-served in a session), then over `value` and every field value
  for entries not already matched (always decrypted fresh, never cached).
  A `SearchResult` records `matched_body` but never the matched text.
  Output is `title  shortid` per match, sorted by title then id; nothing
  printed for no matches.
- **`reindex`** in a session discards and rebuilds; one-shot it confirms
  the vault is reachable and says there is no cache to rebuild
  (`TestReindexOneShotHasNothingToRebuild`).

### Settled at the start of implementation

- **Orphaned spans are zeroed at once, and growth copies only live
  spans.** Go leaves a removed or renamed entry's old title in the arena
  until discard, and its growth copies dead bytes too. The plan above
  already chose the stronger rule; this records that it is a deliberate
  difference from Go, not a port of it.
- **Arena sizing**: Go's 4096-byte minimum, doubled until the new bytes
  fit. The arena is one `LockedBuf`; its page-rounded slack is wiped on
  drop with the rest.
- **The decrypt counter** is a `DecryptCounter` field on `Vault`: an
  `Arc<AtomicUsize>` under `cfg(any(test, feature = "test-support"))`,
  shared by a vault's clones (a session's opener hands out clones), and
  a zero-sized no-op otherwise. Bumped in `read_entry`, the one decrypt
  path.
- **Index updates after a write.** `edit` has the entry it wrote and
  calls `Session::note_entry`. `insert`, `generate` and `rename` get only
  an id back from the library, so they call `Session::refresh_entry(vault,
  id)`, which reads that one entry only if the vault is indexed: no
  decrypt otherwise, and the cached dates are the ones written, not a
  guess. `rm` calls `forget_entry`. All are no-ops one-shot.
- **Test access to internals**: `Session` gets `#[cfg(test)]` accessors
  for whether a vault has an index and a copy of its arena bytes; the M6
  test fixture (`Device`) becomes `pub(crate)` so the index tests share
  it. Visibility only: no M6 test changes.

## Tasks

Tests first, at task granularity.

### 1. The index and its arena ⚑ — 9 `index` rows

- [x] `TestIndexArenaNeverHoldsASecretValue` (a sentinel value appears
      nowhere in the arena after `ls`, `show` and `search`).
- [x] `TestIndexArenaLocksThroughTheSameLockerAsTheIdentity`,
      `TestIndexPageLockFailureWarnsOnceAndProceeds`,
      `TestIndexDoesNotWarnAgainWhenUnlockAlreadyWarned`.
- [x] `TestIndexArenaGrowthPreservesEveryEntryAndItsPageLocks`.
- [x] `TestIndexDiscardZeroesArenaAndIsIdempotent`, `TestLockDiscardsIndex`,
      `TestIdleTimeoutDiscardsIndexLikeAnExplicitLock`.
- [x] `TestIndexNeverWrittenToDisk` (no new file under the config, data
      and state roots or the vault during a session that builds one).
- [x] Beyond Go's rows: growth compacts away removed entries; orphaned
      spans are zeroed at once; an index whose identity failed to lock
      never tries; a growth between a title and its description keeps the
      title (a bug the growth row caught in the compaction this port added,
      pinned and mutation-checked); the warning resets with the index;
      closing a session releases every index.

### 2. Session integration ⚑ — 11 `index` rows

- [x] Build once: `TestFirstListBuildsIndexWithOneDecryptPass`,
      `TestSearchFirstCallBuildsIndexWithoutADoubleDecrypt`.
- [x] Incremental: `TestNoteEntryUpdatesIndexWithoutFullRebuild`,
      `TestForgetEntryRemovesFromIndex`,
      `TestSessionResolveSeesARenameAfterNoteEntry`.
- [x] `TestMultiVaultSessionsHaveSeparateIndexes`,
      `TestReindexForcesFullRebuildAndPicksUpOutOfBandChanges`.
- [x] Agreement with one-shot: `TestListAgreesBetweenOneShotAndSession`,
      `TestSearchAgreesBetweenOneShotAndSession`.
- [x] Search: `TestSearchMatchesTitleDescriptionAndBody`,
      `TestSearchBodyHalfRedecryptsButTitleHalfDoesNot`.
- [x] Beyond Go's rows: a first resolve builds the index and writes
      don't; `refresh_entry` reads one entry only when indexed, and drops
      the index rather than keep it stale when it can't.

### ⚑ Opus review of the index tests

Done (this milestone ran on Opus). The properties the plan names, and
what proves each:

- **No secret value in the arena**: `index_arena_never_holds_a_secret_value`
  after `ls`, `show` and `search`, value and field both; the arena's
  orphaned-span and compaction tests cover a renamed or removed title.
- **Zeroed on discard**: the arena unit test asserts the unlock is handed
  only zeros (the arena is now zeroed *before* its lock comes off, as
  `Identity` does). The review added `every_page_a_session_releases_was_zeroed_first`,
  the same property through a session for the key and the arena alike.
  Mutation-checked: removing the zeroing fails both.
- **Locks preserved across growth**: the growth unit test counts locks
  and unlocks. The review added `growth_through_a_session_keeps_the_arena_locked`,
  which checks the grown arena's pages are locked and the old arena's
  released on the OS-like ledger, rather than counting calls.
- Out of scope, as in Go: transient copies handed to callers (`ListEntry`,
  candidate lists, search results) are ordinary strings.

### 3. `search` library and CLI — 2 + 4 rows

- [x] Library: `TestSearchMatchesFieldValueAndReportsMatchedBody`,
      `TestSortSearchResultsTiesByID`.
- [x] CLI: `TestSearchOneShotMatchesTitleDescriptionAndBody`,
      `TestReindexOneShotHasNothingToRebuild`,
      `TestSessionLsSearchReindexSeeInsertRenameAndRm`,
      `TestSessionEditUpdatesIndexWithoutReindex`.
- [x] Register `search` (alias `grep`) and `reindex` (one-shot and
      session) with Go's descriptions and the fixture flags
      (`m7_commands_have_their_go_fixture_flags_and_descriptions`); add
      their error paths to the `errorprefix` table
      (`no_search_or_reindex_failure_doubles_gages_own_prefix`).
- [x] Beyond Go's rows: `generate` and `--use` keep their indexes
      current in a session; `search` renders identically in both modes
      through the CLI.

### 4. Rows deferred by P2

- [x] `entry` (CLI) `TestLsRendersIdenticallyInBothModes`.
- [x] The `search` half of `TestLsAndSearchPrintNothingWhenThereIsNothing`.

### 5. Unmodified earlier tests, docs and ledger

- [x] M5's and M6's tests pass with no edits beyond what this milestone's
      new `Session` methods require of their setup; any edit is listed
      here with its reason. No assertion changed. Setup edits only:
      `session_tests`' fixture (`Device`, `Cfg`, `Clock`, helpers) and
      `repl_tests`' session harness became `pub(crate)` so the M7 tests
      share them; the harness passes `Env`'s `$EDITOR` setting into the
      session (`None` unless a test sets one, so no M6 test sees a
      change); `query_tests`' `set_editor` became `pub(crate)`; one P2
      comment in `entry_tests` points at the search half's new home.
- [x] Extend `doc/cli/session-mode.md` (the index, `reindex`) and
      `doc/cli/entries.md` (`search`/`grep`).
- [x] Parity ledger: §3 M7; §4 `search`, `reindex`.
- [x] P3 index status column.

## Definition of done

Every box above checked; all 26 claimed rows ported; the 2 deferred P2
rows done; the Opus review of the index tests done and recorded; M5's and
M6's tests still pass unmodified; `make lint` and `make test` clean
locally and on Linux, macOS and Windows CI. A session decrypts a vault
once for metadata, and no secret value is ever held in the index.
