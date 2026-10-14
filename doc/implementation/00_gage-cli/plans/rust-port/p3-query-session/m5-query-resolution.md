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

Recommended answers; confirm them when this plan is reviewed.

- **Resolver**: extend M4's `resolve_query` in place by inserting the two
  substring stages, keeping it a pure function over `(id, title)` pairs so
  M6's session and M7's index call the same code. Substring title
  compares lowercase forms (`str::to_lowercase`, Go's `strings.ToLower`).
  Substring UUID compares against the canonical hyphenated lowercase id
  with hyphens and case ignored on both sides, so a query may span a
  hyphen. Exact UUID accepts every spelling `Uuid::parse_str` accepts, as
  M4. A non-UUID query simply matches nothing at the UUID stages.
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
- **`rename <query> <new-title>`**: the same duplicate-title check as
  `insert`, with `-f`; renaming to the entry's own title is always
  allowed. Only `title` and `updated`/`updated_by` change. Prints
  `gage: renamed to "<title>" (<shortid>)`.
- **`Vault::update(id, &Entry, &Identity)`** is the shared write path for
  `edit` and `rename`: encrypt, write, commit (message = the UUID), under
  the vault lock, exactly as `insert`.
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
  **before** any scratch file is created.
- **Scratch directory**: on Linux, the first of `$XDG_RUNTIME_DIR` and
  `/dev/shm` that `statfs` reports as tmpfs (`TMPFS_MAGIC`); otherwise,
  and on macOS and Windows, `std::env::temp_dir()`. File created through
  `tempfile::Builder` (exclusive, unpredictable name `gage-edit-*.yaml`),
  mode 0600 on Unix. On every exit path (editor exit 0, non-zero exit,
  parse failure, unchanged) the file is overwritten with zeros at its
  current length, then unlinked; both best-effort.
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
  it here.

## Tasks

Tests first, at task granularity.

### 1. Resolver — `resolve` (12 rows)

- [ ] `TestResolveExactTitleWinsOverSubstringOfAnotherTitle`,
      `TestResolveUniqueSubstringMatch`,
      `TestResolveSubstringMatchIsCaseInsensitive`.
- [ ] `TestResolveByExactUUID`, `TestResolveByNonCanonicalUUIDSpellings`,
      `TestResolveByUUIDPrefix`, `TestResolveByUUIDMidStringSubstring`,
      `TestResolveByUUIDSubstringSpanningAHyphen`.
- [ ] `TestResolveExactTitleWinsOverUUIDMatch` (the regression test for
      Go's original UUID-first order).
- [ ] `TestResolveAmbiguousSubstringListsCandidatesAsAValue`,
      `TestResolveAmbiguousUUIDSubstringListsCandidates`,
      `TestResolveNotFoundAndAmbiguousAreDistinguishable`.

### 2. Library writes — `entry_edit` (6 rows) and `generate` library (11 rows)

- [ ] `Vault::update`: `TestUpdateRewritesEntryAndCommits`.
- [ ] `Vault::rename`: `TestRenameChangesOnlyTitleAndBumpsUpdated`,
      `TestRenameProducesExactlyOneCommit`,
      `TestRenameToExistingTitleRejectedUnlessForced`,
      `TestRenameToOwnTitleAlwaysAllowed`, `TestRenameUnknownQueryFails`.
- [ ] `generate_value`: `TestGenerateValueLengthAndAlphabet`,
      `TestGenerateValueHonoursLength`,
      `TestGenerateValueNoSymbolsExcludesSymbols`,
      `TestGenerateValueStillDrawsSymbolsByDefault`,
      `TestGenerateValueZeroLengthUsesTheDefault`,
      `TestGenerateValueRejectsTooShortLength`,
      `TestGenerateValueDiffersAcrossCalls`,
      `TestGenerateDrawsFromCryptoRandNotMathRand` (structural: the
      default source is the OS CSPRNG type, not a seeded PRNG).
- [ ] `random_string`: `TestRandomStringDrawIsStructural`,
      `TestRandomStringPropagatesReaderFailure`,
      `TestRandomStringRejectsNonPositiveLengthAndEmptyAlphabet`.

### 3. CLI resolution across commands — `resolve_cli` (6 rows)

- [ ] Re-wire `cat` and `rm` (already on `resolve_query`; they pick up
      the new stages) and wire `show`, `edit`, `rename`.
- [ ] `TestShowCatRmResolveIdenticallyAcrossQueryForms`,
      `TestCatResolvesByExactTitleEvenWhenQueryIsAlsoAUUIDSubstring`,
      `TestAmbiguousQueryListsCandidatesAndFailsWithoutPromptingForEveryCommand`
      (every query-taking command, with a prompter that panics if asked),
      `TestAmbiguousRmAndRenameLeaveEveryCandidateUntouched`,
      `TestAmbiguousUUIDSubstringFailsAtTheCLIWithCandidates`,
      `TestUnknownQueryIsDistinguishableFromAmbiguousAtTheCLI`.

### 4. The `$EDITOR` helper ⚑ — `editor` (11 rows), `scratchdir*` (4 rows)

- [ ] Resolution before any file: `TestEditYAMLUnsetEditorFailsBeforeAnyScratchFile`,
      `TestEditYAMLUnusableEditorFailsBeforeAnyScratchFile`.
- [ ] Scratch file: `TestEditYAMLScratchFileCreated0600` (Unix only, as
      Go skips it on Windows), `TestEditYAMLScratchFileLandsInScratchDir`.
- [ ] Round trip: `TestEditYAMLSetValueReportsChanged`,
      `TestEditYAMLUnchangedReportsUnchangedAndParsesBackIdentically`,
      `TestEditYAMLNonZeroExitFails`, `TestEditYAMLInvalidYAMLFailsToParse`.
- [ ] Cleanup: `TestEditYAMLScratchFileRemovedOnEveryExitPath`,
      `TestOverwriteFileZeroesContentsInPlace`,
      `TestOverwriteThenRemoveOverwritesBeforeUnlinking` (through a hard
      link, skipped where hard links are unavailable, as Go).
- [ ] Scratch directory: `TestScratchDirIsAWritableDirectory`; Linux
      only: `TestScratchDirPrefersTmpfsOnLinux`,
      `TestIsTmpfsRejectsANonExistentDirectory`; elsewhere:
      `TestScratchDirIsTheOSTempDirOffLinux`.

### 5. `show`, `edit`, `rename` — `show` (8 of 9), `edit` (5), `rename` (3)

- [ ] `show`: `TestShowPrintsOnlyValue`, `TestCatPrintsFullYAMLDistinctFromShow`,
      `TestShowMultilineValueHasNoExtraTrailingBlankLine`,
      `TestShowUnknownQueryFailsNotFound`,
      `TestShowAmbiguousQueryListsCandidatesAndFails`,
      `TestShowFieldExtractsNamedField`, `TestShowFieldOnEntryWithNoFields`,
      `TestShowUnknownFieldFailsClearly`.
- [ ] `edit`: `TestEditChangesValueAndLeavesOtherFieldsUntouched`,
      `TestEditRestampsUpdatedAndUpdatedByAndCommits`,
      `TestEditRestampIsAuthoritativeOverTheEditedFile`,
      `TestEditUnparseableYAMLAbortsWithNoCommitAndOriginalUnchanged`,
      `TestEditAmbiguousQueryListsCandidatesAndFails`.
- [ ] `rename`: `TestRenameChangesOnlyTitleThroughTheCLI`,
      `TestRenameToExistingTitleRejectedUnlessForcedThroughTheCLI`,
      `TestRenameAmbiguousQueryListsCandidatesAndFails`.
- [ ] Register `show`, `edit`, `rename` (one-shot and session) with Go's
      descriptions and the fixture flags, less `show -c`/`-q`; add their
      error paths to the `errorprefix` table.
- Moved: `TestShowClipCopiesTheNamedFieldNotTheValue` (P6 · M12).

### 6. `generate` and `insert -e` — `generate` CLI (6 of 9), `insert_edit` (7)

- [ ] `generate`: `TestGenerateInsertsEntryWithRandomValueOfSaneDefaultLength`,
      `TestGenerateProducesDifferentValuesAcrossEntries`,
      `TestGenerateProducesExactlyOneCommitAndSetsTimestamps`,
      `TestGenerateDuplicateTitleRejectedUnlessForced`,
      `TestGenerateRespectsLengthAndNoSymbols`,
      `TestGenerateRejectsAnUnreasonablySmallLength` (before any unlock).
- [ ] `insert -e`: `TestInsertEditOpensAPreFilledTemplateWithEmptyValueAndFields`,
      `TestInsertEditOpensTemplateAndInsertsEditedValueAndFields`,
      `TestInsertEditSetsFieldsAtCreation`,
      `TestInsertEditAbortsWithNoEntryWrittenAndNoCommitIfUnchanged`,
      `TestInsertEditAbortsIfValueAndFieldsBothStillEmpty`,
      `TestInsertEditChangedTitleUsedForSavedEntryAndDuplicateCheck`,
      `TestInsertEditRejectedAlongsideMultilineOrValueStdinBeforeAnyIO`.
- [ ] Register `generate` (with `-l`, `--no-symbols`, `--description`,
      `-f`, `-u`; not `-c`/`-q`) and `insert -e`; restore the "mutually
      exclusive with -e/--edit" clause in `--field`'s help.
- Moved: `TestGenerateClipAndQRAreMutuallyExclusive`,
  `TestGenerateClipCopiesTheValueAndPrintsNone`,
  `TestGenerateQRRendersWithoutPrintingTheValue` (P6 · M12).

### 7. Rows deferred by P2

- [ ] `insert_field` `TestInsertFieldWithEditFlagRejectedAsMutuallyExclusive`.
- [ ] The `show --field` halves of `TestInsertFieldRoundTripsThroughShow`
      and `TestInsertMultipleFieldFlagsEachIndependentlyReadable`.
- [ ] The `show` half of `vaultid_keying`
      `TestAVaultWithoutAnIDFailsLoudlyOnEveryDerivedPath`.

### 8. User docs and ledger

- [ ] Port Go's `doc/cli/addressing-entries.md`; extend
      `doc/cli/entries.md` with `show` (and `--field`), `edit`,
      `rename`, `generate`, `insert -e`, replacing M4's "substring
      matching arrives with P3" note.
- [ ] Parity ledger: §2 `a0dcf7c`, `a4b989e` (`show --field` halves);
      §4 `show`, `edit`, `rename`, `generate`; §6 `addressing-entries.md`.
- [ ] P3 index status column.

## Definition of done

Every box above checked; all 79 claimed rows ported, or moved with the
destination named here; the 3 P2 rows in task 7 done; the Opus review of
the scratch-file tests done and recorded; `make lint` and `make test`
clean locally and on Linux, macOS and Windows CI. No query-taking command
is left on exact-only matching.
