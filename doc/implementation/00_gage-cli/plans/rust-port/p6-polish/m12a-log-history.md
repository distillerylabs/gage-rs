# M12a — `log` / `history --decrypt`

[← P6 plan](index.md) · [#53](https://github.com/distillerylabs/gage-rs/issues/53) · next: [M12b](m12b-display-cleanup.md)

> **Recommended model: Sonnet ⚑.** Additive and fully specified by Go's
> code. Run an **Opus review of the merge-walk tests** (task 1) before
> calling it done. Go's first `Log` diffed each commit against its
> neighbour in the walk instead of its parents. On any vault that had
> synced, that invented changes and deletions, and the merge rows are
> what catch it.

## Goal

Two commands over one entry's git history, at two danger levels:

- **`gage log [QUERY]`** prints when an entry changed, newest first: one
  line per commit with the local time and the short commit hash. With no
  query, it prints every entry's commits as one timeline, with each
  line's short entry id. A deleting commit is marked `(deleted)`.
  **Nothing decrypted reaches the output**, and commit messages are never
  printed (they are UUIDs and would only repeat the id). It still unlocks,
  because resolving a title means decrypting every entry.
- **`gage history --decrypt <query>`** decrypts every past revision of one
  entry, oldest first, and prints each one as a diff against the
  next-older revision. `--decrypt` and the query are both mandatory.
  There is no confirmation prompt and no bulk form (Go's M12 decision). A
  revision this identity can't decrypt is an error, never skipped.
- Both accept a **removed entry by its full UUID**, but only after
  ordinary resolution finds nothing. That keeps title-first resolution
  intact, and a substring never matches a removed entry.
- An entry with no commits says so on stderr and exits 0.

## Depends on

- **P1 · M0b:** `gitrepo` (the `git2` wrapper) and `gittest`.
- **P2 · M4:** one commit per write, with the entry's UUID as the message;
  `entries/<uuid>.age` paths.
- **P3 · M5:** the resolver and `NotFound`. **M6/M7:** sessions, where
  both commands also run.
- **P4 · M8b:** conflict resolution, which creates the merge commits the
  merge rows walk (taking one side; both sides of an unrelated pair).

## Design references

- Design doc, "Sync (vault-generic, git-implemented today)": `log` versus
  `history --decrypt`.
- Design doc, "`history --decrypt` is still the one meaningfully more
  dangerous tier".
- Design doc, "Addressing entries": why `log` must unlock.

## Go sources

- Go plan `gage-cli-design/m12-polish.md`: the `history --decrypt`
  decision (no prompt, no TTY gate, mandatory query) and the `log` tests.
- Parity ledger: §3 M12 (`log`, `history`), §4 the `log` and `history`
  fixtures.
- Go sources at the pinned SHA:
  - `internal/gage/gitrepo/history.go`: `Log`, `blob`, `blobAt`
  - `internal/gage/history.go`: `LogEntry`, `Revision`, `Vault.Log`,
    `Vault.History`, `entryRevisions`
  - `cmd/gage/log.go`: `newLogCommand`, `logTargets`,
    `resolveHistoricalQuery`, `writeLog`, `newHistoryCommand`,
    `writeHistory`, `diffEntryLines`
- Test inventory, 24 rows:
  - `internal/gage/gitrepo/history_test.go` (6)
  - `internal/gage/history_test.go` (6)
  - `cmd/gage/log_test.go` (12)

## Decisions to make first

Settled while implementing, and recorded as
[Q-LOG-WALK](../open-questions.md#q-log-walk): the walk's sort order, the
scope of a vault-wide `log`, `history`'s order and print format, and the
`jiff` time zone dependency in `gage-cli`.

- **The walk on `git2`.** `Revwalk` from HEAD, sorted `Sort::TIME` (Go
  uses committer-time order). A commit is a change when its blob at the
  path differs from *every* parent's. A root commit is a change exactly
  when the path is present. "Absent" and "empty" are different states, so
  presence is compared as well as bytes. Blobs are memoised per commit id.
  The revision's time is the **author** time, as Go's `c.Author.When`. An
  unborn HEAD returns an empty list, not an error. Confirm the `git2` calls
  (`Tree::get_path` returning `NotFound` for an absent path, `Commit::parents`)
  against the vendored libgit2 before writing the walk.
- **Types.** `gitrepo::Revision { id, when, message, content: Option<Vec<u8>> }`
  (`None` = deleted). In `crates/gage`: `LogEntry { hash, when, deleted }`
  with no message field (Go's reasoning: printing it would only make `log`
  look like it shows message text), and
  `Revision { hash, when, deleted, entry: Option<Entry> }`. Decrypted
  entries zeroize the way M3's `Entry` already does; check nothing new
  holds plaintext outside one.
- **Signatures.** `Vault::log(&self, id: Uuid)` takes no identity: a
  caller holding an id shouldn't have to unlock. `Vault::history(&self,
  id, &Identity)`. Neither takes the vault lock; they are reads.
- **Errors.** An undecryptable revision is a typed error naming the short
  commit hash and the entry id (Go: `decrypting revision <hash> of <id>`).
  Walk failures map to `Internal`.
- **The deleted-entry fallback** lives in `gage-cli`, as Go's
  `resolveHistoricalQuery` does: resolve; on `NotFound` only, accept the
  query if it parses as a full UUID. Any other resolver error (ambiguous,
  cancelled) is returned as is.
- **Rendering** stays in `gage-cli`. Times are local with the offset
  (`%Y-%m-%d %H:%M:%S %z`, Go's `logTimeFormat`) through `jiff`'s system
  zone. A vault-wide log is sorted on the timestamps, not the rendered
  strings, so a daylight-saving change doesn't misorder it; it is a stable
  sort. Tests that assert times pin the zone through whatever `jiff` reads
  (`TZ`) or compare against the same formatter, never the host's zone.

## Tasks

Write each task's tests first, at task granularity.

### 1. `gitrepo::log` — 6 rows (5 + one half)

- [x] `gitrepo::log(dir, path) -> Result<Vec<Revision>, Error>`, newest
      first, as decided above.
- [x] Rows:
  - `TestLogReportsEachRealChangeOnce`
  - `TestLogReportsADeletionOnce`: the deleting commit appears once, with
    no content.
  - `TestLogOnAPathGitHasNeverSeenIsEmpty`: empty, not an error.
  - `TestLogAcrossAMergeOfUnrelatedEdits` ⚑: an edit on each side to
    *different* paths. The merge is not a change for either path, and no
    commit from one side is reported against the other side's path.
  - `TestLogAcrossAConflictResolvedByTakingASide` ⚑: the merge matches one
    parent, so it is not a change.
  - The `Log` half of `TestLogAndHeadCommitterTimeOnARepoWithNoCommitsYet`:
    an unborn HEAD gives an empty log. The `HeadCommitterTime` half is
    P7's (see the [P6 plan](index.md#inventory-accounting)).
- [x] Beyond Go's rows: a merge that resolved a real conflict with a blob
      equal to *neither* parent is a change. It is the case the
      every-parent rule exists to keep, and none of Go's rows pins it.

### 2. `Vault::log` and `Vault::history` — 6 rows

- [x] Both over one shared walk of `entries/<uuid>.age`, built with
      forward slashes on every platform (Go's `entryFilePath`; a
      `Path::join` matches nothing on Windows).
- [x] Rows:
  - `TestLogReportsOneRevisionPerWrite`
  - `TestLogMarksTheDeletingCommit`
  - `TestLogOnAnEntryGitHasNeverSeenIsEmptyNotAnError`
  - `TestHistoryDecryptsEveryPastRevision`
  - `TestHistoryOnADeletedEntryStillWalksIt`
  - `TestHistoryFailsRatherThanSkippingAnUndecryptableRevision`: a
    revision encrypted to a recipient set without this device. Build it
    with M9's recipient writes and the two-device fixture, not by hand.

### 3. `gage log` — 4 rows + 2 shared with task 4

- [x] Register `log` (group and description from the fixture, both
      modes, `[QUERY]`, `-u/--use`).
- [x] Rows:
  - `TestLogShowsTimestampsAndNoDecryptedContent`: neither the value,
    the title nor any field appears on stdout or stderr.
  - `TestLogRevealsNoTitlesFromCommitMessages`
  - `TestLogShowsADeletedEntrysEnd`: by its full UUID after `rm`.
  - `TestLogOnAQueryThatIsNeitherATitleNorAUUID`: `NotFound`, and a
    substring of a removed entry's id doesn't match it.
  - `TestLogAndHistoryOnAnEntryWithNoHistorySayNothingIsThere` (the `log`
    half; the `history` half is in task 4).
  - `TestLogAndHistoryWorkInASession` (the `log` half).
- [x] Beyond Go's rows: a vault-wide `log` (no query) interleaves two
      entries newest first and tags each line with its short entry id.

### 4. `gage history --decrypt` — 6 rows + 2 shared with task 3

- [x] Register `history` (`--decrypt <query>`, `--decrypt`, `-u/--use`).
      Without `--decrypt` it is a Usage error naming `gage log`. A missing
      query is clap's usage error.
- [x] `diffEntryLines` ported: `+`, `-` and unchanged markers for `title`,
      `description` (only if either side has one), `value`, `updated_by`,
      and `fields.<key>` over the union of both sides' keys, sorted.
- [x] Rows:
  - `TestHistoryRequiresDecrypt`
  - `TestHistoryWithNoQueryIsAUsageError`
  - `TestHistoryDecryptWalksRevisionsAndDiffs`
  - `TestHistoryDecryptMarksADescriptionChange`
  - `TestHistoryDecryptShowsAFieldThatWasRemoved`
  - `TestHistoryDecryptShowsADeletedRevision`
- [x] The `history` halves of
      `TestLogAndHistoryOnAnEntryWithNoHistorySayNothingIsThere` and
      `TestLogAndHistoryWorkInASession`.
- [x] Beyond Go's rows: nothing `history` decrypts reaches the session
      history file, which records only the line as typed (Go's M12
      decision names this as testable).

(Rows: 6 + 6 + 12 = 24. The 12 `log_test` rows are 4 in task 3, 6 in
task 4, and 2 that each task ports half of.)

### 5. Parity, review, ledger

- [x] A `p6_log_and_history_have_their_go_fixture_flags_and_descriptions`
      test over `tests/fixtures/go/help/log.txt` and `history.txt`.
- [x] **Opus review (⚑)** of task 1's merge rows and the extra row. For
      each, name the mutation it kills and record the table here:
  - comparing against the previous commit in the walk instead of the
    parents
  - comparing only bytes, not presence
  - "changed if it differs from *any* parent" instead of every parent

      Opus review: each mutation was written as a minimal, faithful edit
      to `gitrepo::log` (everything else unchanged), then
      `cargo test -p gage history` and `cargo test -p gage-cli log` were
      run and the file restored. Every mutant was killed. This table
      replaces the implementing session's, which credited the linear rows
      against the neighbour mutation and three extra rows against the
      bytes-only one. Those kills came from cruder mutations that also
      broke the root rule; they don't hold for the faithful forms.

      | Mutation | Killed by |
      |---|---|
      | Compare against the next-older commit in the walk, not the parents (the walk's last commit is a change when the path is present) | `log_across_a_merge_of_unrelated_edits`, `log_across_a_conflict_resolved_by_taking_a_side`. No linear row fails. |
      | Compare bytes only, so absent equals empty (root rule kept, or changed to "non-empty") | `log_tells_an_empty_file_from_an_absent_one` only |
      | "Changed if it differs from *any* parent" | `log_across_a_merge_of_unrelated_edits`, `log_across_a_conflict_resolved_by_taking_a_side` |
      | Beyond the list: compare against the first parent only | `log_across_a_merge_of_unrelated_edits`, `log_across_a_conflict_resolved_by_taking_a_side` |
      | Beyond the list: a merge is never a change | `log_reports_a_merge_that_resolved_to_a_blob_neither_parent_has` only |
      | Beyond the list: `Sort::TIME` without `TOPOLOGICAL` | `log_reports_each_real_change_once`, `log_reports_a_deletion_once`, `log_across_a_conflict_resolved_by_taking_a_side`, and three `Vault` rows (same-second commits tie) |
      | Beyond the list: a root commit is never / always a change | five `gitrepo` rows / nine `gage` rows and seven `gage-cli` rows |

      Kills that don't depend on how ties sort:
      `log_across_a_merge_of_unrelated_edits` asserts both sides' files,
      so the neighbour mutation fails it whichever side the walk puts
      next to the merge. The same test catches any-parent and
      first-parent through the merge's other parent, whatever the
      timestamps. The taking-a-side row's neighbour kill does depend on
      walk order: it relies on our commit sorting ahead of theirs, which
      only `TIME | TOPOLOGICAL` and commit order give. Nothing rests on
      that alone.

      `log_reports_a_merge_that_resolved_to_a_blob_neither_parent_has`
      is the only test that kills "a merge is never a change", which is
      the case the every-parent rule exists to keep. Presence-versus-bytes
      also rests on a single row (`log_tells_an_empty_file_from_an_absent_one`).
      That is enough, since gage never writes an empty `.age` file.

      Not covered at this level: author time versus committer time.
      `commit_file` signs both with the same time, so swapping
      `commit.author()` for `commit.committer()` survives. That is a
      field choice taken from Go, not part of the walk rule, so it is not
      a gap in the merge rows.
- [x] Parity ledger: §3 M12 (`log`/`history`) and §4 `log`, `history`.
- [ ] Mark M12a `[x]` in the [P6 index](index.md) after CI.

## Definition of done

Every box above is checked. All 24 claimed rows are ported (one by its
`Log` half, with the other half recorded as P7's), and the review table is
recorded. `make lint` and `make test` are clean locally and on Linux,
macOS and Windows CI.

The milestone is done when:
- `log` on a vault that has synced and merged reports each real change
  once, and nothing it prints was ever encrypted
- `history --decrypt` either shows every past revision or fails, never a
  silent subset
