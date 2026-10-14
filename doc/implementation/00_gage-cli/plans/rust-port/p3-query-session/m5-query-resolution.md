# M5 — Query resolution and the entry commands

[P3 plan](index.md) · [#22](https://github.com/distillerylabs/gage-rs/issues/22) · next: [M6 — Session mode](m6-session-mode.md)

> **Recommended model: Sonnet.** A resolver extension, four commands, and
> the `$EDITOR` round trip, all well specified by Go. **Run an Opus review
> pass over the scratch-file tests** (task 4): it is the one place P3 puts
> a decrypted entry on disk, and a cleanup test that misses an exit path
> passes while leaving plaintext behind.

## Goal

Upgrade addressing from M4's exact title / exact UUID to the full order:
**exact title → substring title (case-insensitive) → exact UUID →
substring UUID → ambiguous**. Move every query-taking command onto it
(`cat` and `rm` along with the new `show`, `edit`, `rename`), so none is
left on exact-only matching. Add `generate` (with `-l` and
`--no-symbols`), `show --field`, and `insert -e`, which shares `edit`'s
`$EDITOR` scratch-file helper. One-shot only: an ambiguous query fails
with its candidates, as in M4; the session picker is M6's.

## Depends on

- **P2 · M4**: `resolve_query` (the pure stage function over
  `(id, title)`), `CandidateList`, `Vault::insert`/`remove`, the one-shot
  handler, `with_write_lock`, commit-per-write.
- **P2 · M3**: `Entry`, `Entry::field`, `yaml::marshal`/`unmarshal`.

## Design references

- Design doc "Addressing entries & the metadata index" (the order, and
  why title comes before UUID), "Entry CRUD" (`show` vs `cat`, `insert -e`
  abort conditions), "A few decisions worth calling out" (the scratch
  file's tmpfs-where-available posture), "What's deliberately CLI-only"
  (`$EDITOR`).
- [Q-EDITOR](../open-questions.md#q-editor);
  [Q-COMPAT](../open-questions.md#q-compat): flags and names from
  `tests/fixtures/go/help/{show,edit,rename,generate,insert}.txt`.

## Go sources

- Go plan `gage-cli-design/m5-query-resolution.md`; the post-plan #58
  `-e` exclusivity row.
- Parity ledger rows: §2 `a0dcf7c` (title before UUID), `a4b989e` (the
  `show --field` halves); §3 M5; §4 `show`, `edit`, `rename`, `generate`
  (less `-c`/`-q`), `insert -e`; §6 `addressing-entries.md`, the `show`
  and `edit` parts of `entries.md`.
- Go sources at the pinned SHA: `internal/gage/entry.go`
  (`resolveTitleAndUUID` and its four `match*` stages, `resolveStage`),
  `internal/gage/entry_edit.go` (`Update`, `Rename`),
  `internal/gage/generate.go`, `cmd/gage/entry.go` (`show`, `edit`,
  `rename`, `generate`, `insert -e`), `cmd/gage/editor.go`,
  `cmd/gage/scratchdir_linux.go`, `cmd/gage/scratchdir_other.go`.
- Test inventory: `resolve` (12), `resolve_cli` (6), `show` (8 of 9),
  `edit` (5), `rename` (3), `entry_edit` (6), `generate` library (11) and
  CLI (6 of 9), `insert_edit` (7), `editor` (11), `scratchdir*` (4): 79
  rows. Plus 3 rows deferred by P2 (task 7).

## Decisions to make first

Recommended answers, confirmed in review. Where implementation changed
one, the bullet says so (**Changed in implementation**).

- **Resolver**: extend M4's `resolve_query` in place by inserting the two
  substring stages, keeping it a pure function over `(id, title)` pairs so
  M6's session and M7's index call the same code. Substring title
  compares lowercase forms (`str::to_lowercase`, Go's `strings.ToLower`).
  Substring UUID compares against the canonical hyphenated lowercase id
  with hyphens and case ignored on both sides, so a query may span a
  hyphen. Exact UUID accepts every spelling `Uuid::parse_str` accepts, as
  M4. A non-UUID query simply matches nothing at the UUID stages.
  **Changed in implementation:** `Vault::resolve` returns `(Uuid, Entry)`,
  as Go's `Resolve` does, rather than the id alone: every Go row asserts on
  the resolved entry, and `show`, `edit` and `rename` all need it.
- **Typed results**: not-found and ambiguous stay distinct
  `ResolveError` variants with distinct exit codes (`NotFound`,
  `Ambiguous`), as M4 already has them; the one-shot CLI prints the
  candidates to stderr and never prompts.
- **`show`** prints the value followed by exactly one newline (a value
  that already ends in `\n` gets none added:
  `TestShowMultilineValueHasNoExtraTrailingBlankLine`). `--field NAME`
  prints that field instead, through `Entry::field`; an unknown field, or
  an entry with no fields, is a clear error with Go's exit code.
- **`edit`** seeds the editor with the entry's YAML, and on save keeps
  `created` from the original and re-stamps `updated`/`updated_by`
  regardless of what the file says (`TestEditRestampIsAuthoritativeOverTheEditedFile`).
  As Go, `edit` has **no unchanged abort**: it commits every time the
  file parses. Prints `gage: updated "<title>" (<shortid>)`.
  **Changed in implementation:** the re-stamp is the library's
  `Entry::restamp_after_edit(&original, &Identity)` rather than three
  assignments in the CLI (Go does it in `cmd/gage`), so a GUI editor gets
  the same rule.
- **`rename <query> <new-title>`**: the same duplicate-title check as
  `insert`, with `-f`; renaming to the entry's own title is always
  allowed. Only `title` and `updated`/`updated_by` change. Prints
  `gage: renamed to "<title>" (<shortid>)`.
- **`Vault::update(id, &Entry, &Identity)`** is the shared write path for
  `edit` and `rename`: encrypt, write, commit (message = the UUID), under
  the vault lock, exactly as `insert`. **Changed in implementation:**
  `Vault::update(id, &Entry)` takes no identity. The entry is encrypted to
  the recipients, not the writer, and Go's only other use for it is
  reporting the automatic push, which is P4's; P4 adds it if it needs it.
  `Vault::rename` does its own resolve, check and write under one lock
  hold (as `remove` does) rather than going through `update`, so the
  duplicate check can't race a concurrent write.
- **`generate`**: Go's constants (default length 24, minimum 8, letters +
  digits + `!@#$%^&*-_=+`, `--no-symbols` drops the symbols). Each
  character is drawn uniformly from the OS CSPRNG: `rand_core` 0.6's
  `OsRng` (already in the dependency tree through `age`), with
  `random_string` taking `&mut dyn RngCore` and using `try_fill_bytes`, so
  `TestRandomStringDrawIsStructural` and
  `TestRandomStringPropagatesReaderFailure` can hand it a scripted or
  failing source. Rejection sampling, no modulo bias. Length 0 means the
  default; below 8 is a Usage error before any unlock. Prints
  `gage: generated "<title>" (<id>)`, never the value (`-c`/`-q`, the ways
  to see it at creation, are P6).
- **`$EDITOR` resolution** (`gage-cli`, Q-EDITOR): trim, split on
  whitespace, resolve the first word with `which::which` (handles
  Windows `PATHEXT`), pass the rest as arguments with the scratch path
  last. Unset or blank, or not found, fails with Go's Internal error
  **before** any scratch file is created. **Changed in implementation:**
  `main` reads `$EDITOR` into a new `Io::editor` field rather than the
  helper reading the process environment: in-process tests run in
  parallel against one environment, and `std::env::set_var` is `unsafe`
  (denied workspace-wide).
- **Scratch directory**: on Linux, the first of `$XDG_RUNTIME_DIR` and
  `/dev/shm` that `statfs` reports as tmpfs (`TMPFS_MAGIC`); otherwise,
  and on macOS and Windows, `std::env::temp_dir()`. File created through
  `tempfile::Builder` (exclusive, unpredictable name `gage-edit-*.yaml`),
  mode 0600 on Unix. On every exit path (editor exit 0, non-zero exit,
  parse failure, unchanged) the file is overwritten with zeros at its
  current length, then unlinked; both best-effort. **Added in review:**
  on Unix the handle gage wrote the file through stays open across the
  editor and is zeroed too, so an editor that saves by renaming a new
  file over the path doesn't leave the original inode un-zeroed (Windows
  closes it before the editor runs, as editors there may refuse a shared
  file).
- **`insert -e`** seeds a template with `title` and `--description`
  pre-filled and empty `value`/`fields`; aborts with no entry and no
  commit if the file comes back unchanged, or with `value` and `fields`
  both still empty; uses the edited title for the saved entry and the
  duplicate check. Rejected alongside `-m`, `--value-stdin` or `--field`
  at parse time (clap `conflicts_with`), before any I/O.
- **The fake editor in tests**: `$EDITOR` set to the test binary
  re-executed (`current_exe` + `--exact <helper test> --nocapture`, as
  the vault-lock and core-dump tests already do), with the action chosen
  by an environment variable (`set-value`, `set-title`, `fail`,
  `write-garbage`, ...), mirroring Go's `setFakeEditor`. libtest treats
  the trailing scratch path as one more name filter, which matches
  nothing. If a platform's test-binary path turns out to contain
  whitespace, fall back to a generated one-line launcher script and record
  it here. **Changed in implementation:** the action comes from a config
  file named on the editor's command line (`*.gage-fake-editor`, another
  ignored libtest filter), not an environment variable, for the same
  shared-environment reason as `Io::editor`. `FakeEditor::new` asserts the
  command splits into exactly its seven words, so a path with whitespace
  fails loudly rather than mysteriously.
- **`insert -e` keeps unknown keys** (added in implementation): Go passes
  the edited entry's `Extra` through to `Insert`, so `NewEntry` gains an
  `extra` map.

## Tasks

Tests first, at task granularity.

### 1. Resolver — `resolve` (12 rows)

- [x] `TestResolveExactTitleWinsOverSubstringOfAnotherTitle`,
      `TestResolveUniqueSubstringMatch`,
      `TestResolveSubstringMatchIsCaseInsensitive`.
- [x] `TestResolveByExactUUID`, `TestResolveByNonCanonicalUUIDSpellings`,
      `TestResolveByUUIDPrefix`, `TestResolveByUUIDMidStringSubstring`,
      `TestResolveByUUIDSubstringSpanningAHyphen`.
- [x] `TestResolveExactTitleWinsOverUUIDMatch` (the regression test for
      Go's original UUID-first order).
- [x] `TestResolveAmbiguousSubstringListsCandidatesAsAValue`,
      `TestResolveAmbiguousUUIDSubstringListsCandidates`,
      `TestResolveNotFoundAndAmbiguousAreDistinguishable`.

### 2. Library writes — `entry_edit` (6 rows) and `generate` library (11 rows)

- [x] `Vault::update`: `TestUpdateRewritesEntryAndCommits`.
- [x] `Vault::rename`: `TestRenameChangesOnlyTitleAndBumpsUpdated`,
      `TestRenameProducesExactlyOneCommit`,
      `TestRenameToExistingTitleRejectedUnlessForced`,
      `TestRenameToOwnTitleAlwaysAllowed`, `TestRenameUnknownQueryFails`.
- [x] `generate_value`: `TestGenerateValueLengthAndAlphabet`,
      `TestGenerateValueHonoursLength`,
      `TestGenerateValueNoSymbolsExcludesSymbols`,
      `TestGenerateValueStillDrawsSymbolsByDefault`,
      `TestGenerateValueZeroLengthUsesTheDefault`,
      `TestGenerateValueRejectsTooShortLength`,
      `TestGenerateValueDiffersAcrossCalls`,
      `TestGenerateDrawsFromCryptoRandNotMathRand` (structural: the
      default source is the OS CSPRNG type, not a seeded PRNG).
- [x] `random_string`: `TestRandomStringDrawIsStructural`,
      `TestRandomStringPropagatesReaderFailure`,
      `TestRandomStringRejectsNonPositiveLengthAndEmptyAlphabet`.

### 3. CLI resolution across commands — `resolve_cli` (6 rows)

- [x] Re-wire `cat` and `rm` (already on `resolve_query`; they pick up
      the new stages) and wire `show`, `edit`, `rename`.
- [x] `TestShowCatRmResolveIdenticallyAcrossQueryForms`,
      `TestCatResolvesByExactTitleEvenWhenQueryIsAlsoAUUIDSubstring`,
      `TestAmbiguousQueryListsCandidatesAndFailsWithoutPromptingForEveryCommand`
      (every query-taking command, with a prompter that panics if asked),
      `TestAmbiguousRmAndRenameLeaveEveryCandidateUntouched`,
      `TestAmbiguousUUIDSubstringFailsAtTheCLIWithCandidates`,
      `TestUnknownQueryIsDistinguishableFromAmbiguousAtTheCLI`.

### 4. The `$EDITOR` helper ⚑ — `editor` (11 rows), `scratchdir*` (4 rows)

- [x] Resolution before any file: `TestEditYAMLUnsetEditorFailsBeforeAnyScratchFile`,
      `TestEditYAMLUnusableEditorFailsBeforeAnyScratchFile`.
- [x] Scratch file: `TestEditYAMLScratchFileCreated0600` (Unix only, as
      Go skips it on Windows), `TestEditYAMLScratchFileLandsInScratchDir`.
- [x] Round trip: `TestEditYAMLSetValueReportsChanged`,
      `TestEditYAMLUnchangedReportsUnchangedAndParsesBackIdentically`,
      `TestEditYAMLNonZeroExitFails`, `TestEditYAMLInvalidYAMLFailsToParse`.
- [x] Cleanup: `TestEditYAMLScratchFileRemovedOnEveryExitPath`,
      `TestOverwriteFileZeroesContentsInPlace`,
      `TestOverwriteThenRemoveOverwritesBeforeUnlinking` (through a hard
      link, skipped where hard links are unavailable, as Go).
- [x] Scratch directory: `TestScratchDirIsAWritableDirectory`; Linux
      only: `TestScratchDirPrefersTmpfsOnLinux`,
      `TestIsTmpfsRejectsANonExistentDirectory`; elsewhere:
      `TestScratchDirIsTheOSTempDirOffLinux`.
- [x] From the second review (not Go rows): the rename-over-the-path
      editor, `/proc/self/mounts` as an independent tmpfs oracle,
      `$XDG_RUNTIME_DIR` verified and ordered, scratch file gone after
      every command-level abort. See the review below.

### 5. `show`, `edit`, `rename` — `show` (8 of 9), `edit` (5), `rename` (3)

- [x] `show`: `TestShowPrintsOnlyValue`, `TestCatPrintsFullYAMLDistinctFromShow`,
      `TestShowMultilineValueHasNoExtraTrailingBlankLine`,
      `TestShowUnknownQueryFailsNotFound`,
      `TestShowAmbiguousQueryListsCandidatesAndFails`,
      `TestShowFieldExtractsNamedField`, `TestShowFieldOnEntryWithNoFields`,
      `TestShowUnknownFieldFailsClearly`.
- [x] `edit`: `TestEditChangesValueAndLeavesOtherFieldsUntouched`,
      `TestEditRestampsUpdatedAndUpdatedByAndCommits`,
      `TestEditRestampIsAuthoritativeOverTheEditedFile`,
      `TestEditUnparseableYAMLAbortsWithNoCommitAndOriginalUnchanged`,
      `TestEditAmbiguousQueryListsCandidatesAndFails`.
- [x] `rename`: `TestRenameChangesOnlyTitleThroughTheCLI`,
      `TestRenameToExistingTitleRejectedUnlessForcedThroughTheCLI`,
      `TestRenameAmbiguousQueryListsCandidatesAndFails`.
- [x] Register `show`, `edit`, `rename` (one-shot and session) with Go's
      descriptions and the fixture flags, less `show -c`/`-q`; add their
      error paths to the `errorprefix` table.
- Moved: `TestShowClipCopiesTheNamedFieldNotTheValue` (P6 · M12).

### 6. `generate` and `insert -e` — `generate` CLI (6 of 9), `insert_edit` (7)

- [x] `generate`: `TestGenerateInsertsEntryWithRandomValueOfSaneDefaultLength`,
      `TestGenerateProducesDifferentValuesAcrossEntries`,
      `TestGenerateProducesExactlyOneCommitAndSetsTimestamps`,
      `TestGenerateDuplicateTitleRejectedUnlessForced`,
      `TestGenerateRespectsLengthAndNoSymbols`,
      `TestGenerateRejectsAnUnreasonablySmallLength` (before any unlock).
- [x] `insert -e`: `TestInsertEditOpensAPreFilledTemplateWithEmptyValueAndFields`,
      `TestInsertEditOpensTemplateAndInsertsEditedValueAndFields`,
      `TestInsertEditSetsFieldsAtCreation`,
      `TestInsertEditAbortsWithNoEntryWrittenAndNoCommitIfUnchanged`,
      `TestInsertEditAbortsIfValueAndFieldsBothStillEmpty`,
      `TestInsertEditChangedTitleUsedForSavedEntryAndDuplicateCheck`,
      `TestInsertEditRejectedAlongsideMultilineOrValueStdinBeforeAnyIO`.
- [x] Register `generate` (with `-l`, `--no-symbols`, `--description`,
      `-f`, `-u`; not `-c`/`-q`) and `insert -e`; restore the "mutually
      exclusive with -e/--edit" clause in `--field`'s help.
- Moved: `TestGenerateClipAndQRAreMutuallyExclusive`,
  `TestGenerateClipCopiesTheValueAndPrintsNone`,
  `TestGenerateQRRendersWithoutPrintingTheValue` (P6 · M12).

### 7. Rows deferred by P2

- [x] `insert_field` `TestInsertFieldWithEditFlagRejectedAsMutuallyExclusive`.
- [x] The `show --field` halves of `TestInsertFieldRoundTripsThroughShow`
      and `TestInsertMultipleFieldFlagsEachIndependentlyReadable`.
- [x] The `show` half of `vaultid_keying`
      `TestAVaultWithoutAnIDFailsLoudlyOnEveryDerivedPath`.

### 8. User docs and ledger

- [x] Port Go's `doc/cli/addressing-entries.md`; extend
      `doc/cli/entries.md` with `show` (and `--field`), `edit`,
      `rename`, `generate`, `insert -e`, replacing M4's "substring
      matching arrives with P3" note.
- [x] Parity ledger: §2 `a0dcf7c`, `a4b989e` (`show --field` halves);
      §4 `show`, `edit`, `rename`, `generate`; §6 `addressing-entries.md`.
- [x] P3 index status column.

## Where the rows landed

Library rows are in `crates/gage/src/entry/crud.rs` (`resolve`,
`entry_edit`) and `crates/gage/src/generate.rs`; the `$EDITOR` helper's in
`crates/gage-cli/src/editor.rs`; every CLI row in
`crates/gage-cli/src/query_tests.rs`. Each test names its Go row above
it. Rows ported with a difference:

- `TestGenerateValueRejectsTooShortLength` and
  `TestRandomStringRejectsNonPositiveLengthAndEmptyAlphabet`: Go's
  negative lengths have no `usize` spelling. The CLI rejects `-l -5` at
  parse time (also Usage), which `generate_rejects_an_unreasonably_small_length`
  checks.
- `TestGenerateDrawsFromCryptoRandNotMathRand` reads the source as Go's
  does: the production half of `generate.rs` must call
  `random_string(&mut OsRng, ...)` and name no seedable or thread-local
  generator. Go's `rand.Int` check (no modulo bias) is replaced by a
  behavioral test, `random_string_rejects_bytes_that_would_bias_the_draw`.
- `TestAmbiguousQueryListsCandidatesAndFailsWithoutPromptingForEveryCommand`:
  there is no `Prompter::choose` until M6, so "never prompts" is the
  `FakePrompter` scripted with the unlock alone (any other prompt panics),
  plus a fake editor that records if it was ever run.
- `TestInsertEditRejectedAlongsideMultilineOrValueStdinBeforeAnyIO` and
  `TestInsertFieldWithEditFlagRejectedAsMutuallyExclusive`: clap's
  conflict message says "cannot be used with" where Go says "mutually
  exclusive"; wording is not behavior (Q-COMPAT).
- Help parity: `m5_commands_have_their_go_fixture_flags_and_descriptions`
  compares each M5 command's description, positionals and flags with its
  `tests/fixtures/go/help/` file, `-c`/`-q` excepted.

## Opus review of the scratch-file tests ⚑

Done in implementation, against every way out of `edit_yaml_in` once the
file exists (editor exit 0 unchanged or changed, non-zero exit, parse
failure, read-back failure) and before it (unset, blank, not on `PATH`).
Two gaps found and closed with tests that aren't Go rows:

- Nothing proved the *real* flow overwrites before unlinking; Go's
  hard-link test covers the helper alone, so a `Drop` reduced to a bare
  unlink passed every test. `edit_yaml_overwrites_the_scratch_file_before_unlinking_it`
  has the fake editor hard-link the file to a witness and checks the
  witness is zeros afterwards; it was confirmed to fail against exactly
  that regression.
- The read-back failure path (the editor deletes or moves the file) was
  untested: `edit_yaml_fails_cleanly_when_the_editor_deletes_the_file`,
  and a `delete` case in the every-exit-path test.

"Fails before any file exists" is checked in a private scratch directory
that must stay empty, not by a record the fake editor would never have
written anyway (Go's check can't fail for the unset case).

### Second review pass

A second, independent Opus pass found the first one's fixes sound and
closed these:

- **A broken `is_tmpfs` could not fail any test.** `TestScratchDirPrefersTmpfsOnLinux`
  decided whether to skip by calling `is_tmpfs` itself, and nothing
  checked a positive answer, so an `is_tmpfs` that always said no sent
  plaintext to a disk-backed `/tmp` with the suite green. The Linux tests
  now take tmpfs-ness from `/proc/self/mounts` (`mount_fstype`), and
  `is_tmpfs_agrees_with_proc_mounts` checks `is_tmpfs` against it both
  ways on every candidate the host has.
- **"Verified, not assumed" was untested.** On a CI host whose
  `$XDG_RUNTIME_DIR` is tmpfs or unset, trusting the variable passed.
  `scratch_dir` now delegates to `pick_scratch_dir(xdg)`, so tests pass
  the value without `set_var`:
  `pick_scratch_dir_passes_over_an_xdg_dir_that_is_not_tmpfs` (a
  disk-backed or missing directory is passed over) and
  `pick_scratch_dir_prefers_xdg_over_dev_shm` (order; unset and empty
  fall through).
- **Skips that hid failures.** The Linux precondition skips now fail when
  `CI` is set (`skip_unless_ci`): GitHub's Linux runners have a tmpfs
  `/dev/shm` and a disk-backed root, so a skip there means a broken check.
  The two hard-link witness tests fail rather than skip when the link
  fails, since every CI platform has hard links in its temp dir (Go
  skips; a test policy, not behavior).
- **Editors that save by rename** (write a new file, rename it over the
  path; vim with some `backupcopy` settings, among others) unlink the
  original inode themselves, so zeroing the path reached only the
  replacement and the original plaintext was freed un-zeroed. Closed on
  Unix by keeping gage's own handle open across the editor and zeroing
  through it in `Drop` (Q-EDITOR).
  `edit_yaml_zeroes_the_original_when_the_editor_replaces_the_file`
  (the fake editor's `Replace` mode) was confirmed to fail with the
  handle's zeroing removed. `replace` also joined the every-exit-path
  test.
- **Command-level aborts didn't check the file was gone.** Cleanup is
  structural today (the guard dies inside `edit_yaml_in`, before `edit`
  or `insert -e` decide anything), but M6's session mode could extend its
  lifetime. `FakeEditor::assert_scratch_gone` now runs after `edit`
  succeeding and failing to parse, and after `insert -e` aborting for an
  unchanged file, nothing to save, and a duplicate title.
- **One exit path was missing from the list above:** spawning `$EDITOR`
  failing after `which` found it. It isn't honestly reachable in a test:
  an executable file of garbage comes back as exit 126 (the non-zero
  branch, already covered), so only a race between `which` and the spawn
  takes the error branch. The guard exists before the spawn, so it's
  covered structurally.

### Exposures left open

Shared with Go unless noted, recorded here for P9's review:

- A signal that kills gage while the editor runs (Ctrl-C reaches the whole
  foreground process group; most editors ignore it, gage doesn't) skips
  the cleanup and leaves the plaintext scratch file behind. Closing it
  means ignoring SIGINT for the editor's lifetime.
- Editor side files (vim's `.swp`, `~` backups) are written next to the
  scratch file and are the editor's to clean up, not gage's.
- On Windows, a rename-saving editor still leaves the original un-zeroed:
  the handle is closed before the editor runs (closed on Unix, above).

## Definition of done

Every box above checked; all 79 claimed rows ported, or moved with the
destination named here; the 3 P2 rows in task 7 done; the Opus review of
the scratch-file tests done and recorded; `make lint` and `make test`
clean locally and on Linux, macOS and Windows CI. No query-taking command
is left on exact-only matching.
