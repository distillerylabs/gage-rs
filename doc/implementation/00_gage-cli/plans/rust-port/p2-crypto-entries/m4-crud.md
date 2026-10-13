# M4 — One-shot CRUD

[← M3](m3-entry-format.md) · [P2 plan](index.md) · next: P3 (planned when it starts)

> **Recommended model: Sonnet.** Straightforward CRUD with the judgment
> calls (commit message confidentiality, commit author, `ls` format,
> interim resolution) settled below. **Run an Opus review pass over the
> multi-process lock tests** (task 2): this is the first milestone where
> two `gage` processes could corrupt each other, and a lock test that runs
> threads instead of processes passes and proves nothing.

## Goal

`insert`, `cat`, `rm` and `ls`: the first end-to-end usable slice. Every
command unlocks, does one thing, and drops the `Identity` (one-shot mode;
nothing is cached). Every write is one git commit whose message is the
entry's UUID alone, authored `gage <gage@localhost>`, under the vault
lock for the whole read-modify-commit sequence. No push (P4), no
substring resolution (P3 · M5), no session (P3 · M6). Ends with the P2
user docs.

## Depends on

- **M3**: entries read/write/list.
- **M2**: `Vault::unlock`, `Identity`, the terminal prompter.
- **P1**: `vaultlock`, `gitrepo` (fixed author, explicit branch, isolated
  from user git config), the registry, global config and `current`.

## Design references

- Design doc "Entry CRUD" (the command surface, `insert`'s mutually
  exclusive input modes), "Sync model" ("commit is local, instant, and
  always happens"), "Session model" (one-shot unlock-use-close),
  "Concurrent processes and the vault lock", "Addressing entries & the
  metadata index" (why title before UUID).
- [Q-COMPAT](../open-questions.md#q-compat): flag names and behavior from
  `tests/fixtures/go/help/{insert,cat,rm,ls}.txt`; help layout is clap's.

## Go sources

- Go plan `gage-cli-design/m4-crud.md`, including #58 (`insert --field`).
- Parity ledger rows: §3 M4; §2 `a1572d5` (`ls` shows `updated_by`),
  `a4b989e` (#58 `--field`); §4 help fixtures `insert`, `cat`, `rm`, `ls`;
  §6 `entries.md`.
- Go sources at the pinned SHA: `internal/gage/entry.go` (`Insert`,
  `Remove`, `Resolve`, `titleExists`, `decryptAll`), `internal/gage/vault.go`
  (`withWriteLock`), `internal/gage/search.go` (`List`),
  `internal/gage/index.go` (`ListEntry`), `cmd/gage/entry.go` (`insert`,
  `cat`, `rm`, `ls`, `withUnlockedVault`, `parseFieldFlags`).
- Test inventory: `entry_crud` (13), `cmd/gage/entry_test.go` (26 of 33),
  `insert_field` (14 of 15), `insert_multiline_pty` (1): 54 rows. Plus the
  rows P1 moved here (task 6).

## Decisions to make first

Recommended answers; confirm them when this plan is reviewed.

- **Commit message is the entry's UUID and nothing else** (Go's
  confidentiality decision: no title, no verb). Author and committer are
  `gage <gage@localhost>` (P1's `gitrepo` constants). A new
  `gitrepo::commit_all(dir, message)` stages every change, deletions
  included, and makes one commit.
- **Write lock**: `vaultlock` held from before the duplicate-title check
  to after the commit; timeout 10 s (Go's `vaultLockTimeout`). Reads take
  no lock.
- **Resolution (interim)**: exact title, then exact UUID (any spelling
  `uuid` parses: hyphenated, simple, braced, URN). More than one match at
  a stage is `Ambiguous` carrying a typed `CandidateList` (query, then
  `(id, title)` pairs sorted by title then id); the one-shot CLI prints the
  candidates to stderr and exits Ambiguous. P3 · M5 inserts substring
  title and substring UUID as stages 2 and 4. The stage logic is a pure
  function over `id → title`, as Go's `resolveTitleAndUUID`, so M7's index
  can reuse it.
- **Unlock before asking for the value**, as Go: a wrong passphrase never
  makes the user type the secret for nothing. Flag validation (`--field`,
  mode exclusivity) happens before both.
- **`insert`'s value**: masked prompt through a new `Prompter::value`
  (Go's `Value` reads like a passphrase); `--value-stdin` reads stdin to
  EOF and trims exactly one trailing `\n` or `\r\n`; `-m` reads stdin to
  EOF verbatim. `-e/--edit` is not registered until P3 · M5.
- **`--field NAME=VALUE`** (repeatable): split on the first `=`; every
  whitespace character stripped from `NAME`, `VALUE` kept verbatim;
  missing `=`, an empty `NAME` after stripping, or a repeated `NAME` is a
  Usage error before any unlock.
- **`ls` output** is Go's current format, since a one-shot `ls` decrypts
  every entry anyway (M7 adds only the cache): title padded to the widest
  (counted in chars), short id (first 8), created and updated as UTC
  `YYYY-MM-DD`, `updated_by`; `-` for a missing value; trailing spaces
  trimmed; sorted by title then id; an empty vault prints nothing.
  `-H/--header` is P6 · M12.
- **`cat` output** is the entry's YAML exactly as M3 writes it; the id
  line and alignment are P6 · M12.
- **Messages**: `gage: inserted "<title>" (<id>)` and
  `gage: removed <id>` on stdout, as Go.
- **The one-shot handler** is one function in `gage-cli`: resolve `--use`
  (or `current`) from global config, fail on an unregistered name before
  any prompt, build the `Vault`, unlock, run the command, and let the
  `Identity` drop on every path. Its "drops on the error path" test is a
  unit test of that function with a counting `Locker` (through M2's
  `Vault::with_locker`), not a new seam in `Io`.
- **`Vault::list(&Identity) -> Vec<ListEntry>`** (id, title, description,
  created, updated, updated_by), so `ls` renders from one type and M7's
  index can serve the same rows.

## Tasks

Tests first, at task granularity.

### 1. Library CRUD — `entry_crud` (13 rows)

- [x] `TestInsertRoundTripsThroughReadEntry`,
      `TestInsertCommitsExactlyOnceWithUUIDMessageAndFixedAuthor`,
      `TestInsertDuplicateTitleRejectedWithoutForce`,
      `TestInsertDuplicateTitleAllowedWithForce`.
- [x] `TestRemoveDeletesFileAndCommits`, `TestRemoveUnknownQueryFails`.
- [x] Resolution: `TestResolveByExactTitle`, `TestResolveByUUID`,
      `TestResolveAmbiguousTitleFails` (typed candidate list),
      `TestResolveUnknownQueryFails`.
- [x] Lock ⚑ (task 2 writes these as process-level tests):
      `TestInsertBlocksOnAConcurrentlyHeldWriteLock`,
      `TestReadDoesNotBlockOnAHeldWriteLock`,
      `TestWriteLockSerializesConcurrentInsertsAndLosesNoCommit`.
- [x] `gitrepo::commit_all`, with its own test that deletions are staged.

### 2. Concurrency, process-level ⚑ — 2 CLI rows and the 3 library lock rows

- [x] The library lock rows above, with the lock held by a child process
      (the test binary re-executed, as `vaultlock`'s tests do), never by a
      thread.
- [x] `TestConcurrentInsertsThroughTheCLIBothSucceed` (two real `gage`
      processes, two commits, both entries readable),
      `TestConcurrentReadsDoNotBlockEachOther`.

### 3. The one-shot handler and `--use` — 4 CLI rows

- [x] `TestNoUseFlagOperatesAgainstCurrentVault`,
      `TestUseFlagTargetsNamedVaultNotCurrent`,
      `TestUseUnregisteredVaultFailsBeforePromptingForAPassphrase`,
      `TestOneShotHandlerClosesIdentityOnEveryExitPathIncludingError`.

### 4. `insert`, `cat`, `rm`, `ls` — 20 CLI rows and `insert_multiline_pty` (1)

- [x] `insert`: `TestInsertThenCatRoundTripsValueThroughTheCLI`,
      `TestInsertDefaultPromptsForValueViaPrompter`,
      `TestInsertValueStdinTrimsExactlyOneNewline`,
      `TestInsertValueStdinTrimsOneTrailingNewlineAndRoundTrips`,
      `TestValueStdinAndPromptPathsStoreIdenticalEntries`,
      `TestTrimOneTrailingNewline`,
      `TestInsertMutuallyExclusiveValueFlagsRejectedBeforeAnyIO`,
      `TestInsertDescriptionStoredAndShownByCat`,
      `TestInsertDuplicateTitleRejectedUnlessForced`,
      `TestInsertSetsTimestampsAndUpdatedBy` (`updated_by` is the device
      name from M2), `TestInsertProducesExactlyOneCommit`,
      `TestInsertCommitMessageIsUUIDOnly`,
      `TestCommitAuthorIgnoresTheUsersGitConfig` (a child process with a
      hostile `HOME`, as M1's git-defaults test).
- [x] `cat` and `rm`: `TestCatUnknownFailsWithNotFound`,
      `TestRmDeletesFileAndCommitsThenCatAndLsNoLongerShowIt`.
- [x] `ls`: `TestLsListsInsertedTitleAlongsidePartialUUID`,
      `TestLsEmptyVaultNoOutput`, `TestLsPlaceholdersForMissingColumns`,
      `TestShortEntryIDLeavesShortStringsAlone`, and the `ls` half of
      `TestLsAndSearchPrintNothingWhenThereIsNothing` (the `search` half is
      P3 · M7).
- [x] An ambiguous `cat`/`rm` (two forced duplicate titles) prints the
      candidates to stderr and exits Ambiguous.
- [x] Register `insert`, `cat`, `rm`, `ls` (one-shot and session) with
      Go's descriptions and the flags listed in the fixtures, less `-e`
      (P3) and `-H` (P6); add their error paths to the `errorprefix`
      table.
- [x] Real terminal (Unix only, as Go): `TestInsertMultilineCapturesFromARealTerminalUntilEOF`
      (`insert -m` reading until Ctrl-D on a `portable-pty` terminal,
      byte for byte).
- Moved: `TestLsRendersIdenticallyInBothModes` (P3 · M7);
  `TestAlignCatOutputLeavesNonMappingInputAlone`, `TestPadKeyLinesWithNoKeysIsANoOp`,
  `TestLsHeaderEmptyVaultNoOutput`, `TestLsHeaderPrintsLabelledPipeDelimitedTable`,
  `TestLsHeaderRendersIdenticallyInBothModes`,
  `TestLsDefaultOutputUnchangedByHeaderFeature` (P6 · M12).

### 5. `insert --field` (#58) — `insert_field` (14 of 15 rows)

- [x] Parsing, all before any unlock:
      `TestInsertFieldMissingEqualsIsUsageErrorBeforeUnlock`,
      `TestInsertFieldEmptyNameIsUsageErrorBeforeUnlock`,
      `TestInsertFieldWhitespaceOnlyNameIsUsageErrorBeforeUnlock`,
      `TestInsertDuplicateFieldNameIsUsageErrorBeforeUnlock`,
      `TestInsertFieldNamesEqualAfterStrippingAreDuplicates`,
      `TestInsertFieldNameWhitespaceIsStripped`,
      `TestInsertFieldValueWhitespaceIsPreserved`,
      `TestInsertFieldValueContainingEqualsIsPreservedVerbatim`,
      `TestInsertFieldEmptyValueIsStored`.
- [x] With value modes: `TestInsertFieldWithMultilineSetsBothValueAndFields`,
      `TestInsertFieldWithValueStdinSetsBothValueAndFields`,
      `TestInsertFieldWithoutValueModeStillPromptsOnceAndStoresFields`.
- [x] Partial: `TestInsertFieldRoundTripsThroughShow` and
      `TestInsertMultipleFieldFlagsEachIndependentlyReadable` are verified
      through the stored entry (`Entry::field` on what `read_entry`
      returns); **the `show --field` halves move to P3 · M5**.
- [x] Moved: `TestInsertFieldWithEditFlagRejectedAsMutuallyExclusive`
      (P3 · M5, with `-e`).

### 6. Rows from P1

- [x] `vaultid_keying` `TestTwoRegistrationsOfOneRepositoryShareOneLock`:
      two registrations of one repository contend on one lock through
      `insert` (child processes).
- [x] The `insert` half of `TestAVaultWithoutAnIDFailsLoudlyOnEveryDerivedPath`
      (a registration with an empty id fails `insert` before any path is
      built); **the `show` half moves to P3 · M5**, the `identity list`
      half stays with P5.

### 7. User docs and ledger

- [x] Port Go's `doc/cli/entries.md` for the shipped surface: `insert`
      (value modes, `--field`, `--description`, `-f`), `cat`, `rm`, `ls`,
      exact-match addressing (noting that substring matching arrives with
      P3), and the commit-per-write behavior. Link it from
      `doc/cli/README.md`.
- [x] Parity ledger: §3 M4, §2 `a1572d5` and `a4b989e`, §4 (`insert`,
      `cat`, `rm`, `ls`), §6 `entries.md`; P2 rows of §5.
- [x] P2 index status column and the port plan's phase table.

## Implementation notes

What the implementation settled or changed, beyond the decisions above.

- **`ls` columns** are separated by two spaces (the plan fixed everything
  but the separator).
- **The ambiguity error** prints each candidate as `  <full id>  <title>`
  on stderr: full ids, so the next attempt can name one exactly.
- **`-m` on a terminal** prints `gage: enter the value; finish with Ctrl-D
  on an empty line` to stderr first. Off a terminal it prints nothing.
- **The terminal prompter no longer reads ahead.** Found by running the real
  binary: with one piped stdin (`printf 'pass\nvalue\n' | gage insert x
  --value-stdin`) the prompter's buffered reader consumed the value along
  with the passphrase line, and `insert` stored an empty secret and exited
  0. In-process tests can't see it (they use fakes), so the prompter now
  reads a byte at a time and stops at the newline, and a real-binary test
  (`value_stdin_after_a_piped_passphrase_stores_the_value`) guards it. This
  touches M2's prompter, which is why it is recorded here.
- **New library surface for tests**, all behind `test-support` and never in
  the shipped binary: `Vault::with_locker` became public (the one-shot
  handler's "closes on every path" test needs it from `gage-cli`), and
  `gitrepo::head_commit` / `commit_log` read commit metadata without giving
  `gage-cli` a `git2` dependency.
- **Where the tests live.** The library rows are in `entry/crud.rs`
  (the lock rows with a real child process: the test binary re-executed).
  The in-process CLI rows are `crates/gage-cli/src/entry_tests.rs`. The rows
  that need a real process (two `gage` processes inserting, readers while a
  writer holds the lock, `insert -m` on a pty) are
  `crates/gage-cli/tests/entries.rs`, and pay the shipped binary's real
  scrypt cost (about five seconds in all).
- **Lock tests, mutation-checked.** With the lock removed from
  `with_write_lock`, `insert_blocks_on_a_concurrently_held_write_lock` and
  `write_lock_serializes_concurrent_inserts_and_loses_no_commit` fail. The
  Opus review the plan asks for has not been done.
- **`TestUseUnregisteredVaultFailsBeforePromptingForAPassphrase`** is
  checked for all four commands, with a prompter that panics on any prompt.
- **Clap rejects `-m` with `--value-stdin`** at parse time (`conflicts_with`),
  which is before any I/O; the usage error carries the command's help.
- **Deferred flags**: `insert -e` (P3 · M5) and `ls -H` (P6 · M12) are not
  registered, so the `--field` help text omits its "mutually exclusive with
  -e" clause until then.

## Definition of done

Every box above checked; all 54 claimed rows ported, or partially ported
with the remainder named in this doc, or moved; the rows from P1 in task
6 done; the Opus review of the lock tests done; `make lint` and
`make test` clean locally and on Linux, macOS and Windows CI. `gage` is a
usable, minimal single-device secrets manager: insert, list, read and
delete, all committed, all locked against concurrent writers.
