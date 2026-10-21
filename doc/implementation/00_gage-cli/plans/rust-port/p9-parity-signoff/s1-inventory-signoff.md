# S1 — Inventory sign-off

[P9 plan](index.md) · [#86](https://github.com/distillerylabs/gage-rs/issues/86) · next: [S2](s2-ledger-and-help.md)

> **Recommended model: Opus ⚑.** This milestone is the check against
> quiet scope narrowing: for each of the 95 rows, read the Go test and the
> Rust test that claims it, and judge equivalence. A Sonnet pass that
> matches names to names would reproduce exactly the failure P9 exists to
> catch.

## Goal

Every row of `tests/fixtures/go/test-inventory.tsv` is provably ported or
excused, and a test in `make test` keeps it that way.

The guard has two parts:

- **Name match.** A row passes when `lowercase(go_name minus "Test")`
  equals a Rust `#[test]` fn name with its underscores removed and
  lowercased.
- **An exceptions file**, `tests/parity/inventory-exceptions.tsv`, for the
  rest. It is kept minimal: a row that name-matches must not appear in it.

## Depends on

P8 merged (it is, at `6552930`). Nothing in P9 before it.

## Design references

- Port plan "Compatibility and parity strategy", item 5.
- Parity ledger §5.
- CLAUDE.md "Before considering any change done", the parity bullet.

## Go sources

- `tests/fixtures/go/test-inventory.tsv`, all 1,271 rows.
- The 95 rows that don't name-match, as of `6552930`:

| Phase | Go file | Rows |
|---|---|---|
| P1 | `cmd/gage/help_test.go` | `TestListsCommandEntryIgnoresProse` |
| P1 | `cmd/gage/main_integration_test.go` | `TestLinkTimeVersionInjection`, `TestMain`, `TestMakefileLdflagsTargetTheRealSymbols` |
| P1 | `cmd/gage/vaultid_keying_test.go` | `TestCloneCopiesTheVaultsIDIntoGlobalConfig` |
| P1 | `cmd/gage/version_test.go` | the four `TestResolveBuildInfo*` |
| P1 | `internal/gage/atomicfile/atomicfile_test.go` | the five `TestWriteFile*` |
| P1 | `internal/gage/config/config_test.go` | `TestWriteThenModifyLeavesOldValueUntouchedUntilRename` |
| P1 | `internal/gage/devicename/devicename_test.go` | `TestNormalizeNeverProducesAnAllDotsName` |
| P1 | `internal/gage/exitcode/exitcode_test.go` | `TestAllCodesAreDistinct`, `TestEveryDefinedCodeRoundTripsThroughConstructors`, `TestNilErrorIsSuccess`, `TestUnregisteredCodeStringsAsItsNumber`, `TestWrapNilIsNil` |
| P1 | `internal/gage/vaultlock/vaultlock_test.go` | `TestMain`, `TestReleaseOnANilLockIsANoOp` |
| P1 | `internal/gage/xdgpaths/xdgpaths_test.go` | `TestUnknownRoleValuePanicsInAllThreeSelectors` |
| P2 | `cmd/gage/entry_test.go` | `TestLsAndSearchPrintNothingWhenThereIsNothing`, `TestTrimOneTrailingNewline` |
| P2 | `cmd/gage/insert_field_test.go` | 8 rows: `TestInsertField*`, `TestInsertDuplicateFieldNameIsUsageErrorBeforeUnlock`, `TestInsertMultipleFieldFlagsEachIndependentlyReadable` |
| P2 | `cmd/gage/prompter_test.go` | `TestPrompterRefusesAMethodItCannotSatisfy`, `TestResolveConflictNeverGuesses` |
| P2 | `internal/gage/crypt_test.go` | `TestDecryptWithAClosedIdentityFails`, `TestEncryptRefusesAZeroValueRecipient` |
| P2 | `internal/gage/entry_crud_test.go` | `TestResolveAmbiguousTitleFails`, `TestResolveByUUID` |
| P2 | `internal/gage/entry_test.go` | 17 rows: `TestEntry*`, `TestReadEntry*`, `TestTimestamp*YAML*`, `TestUnknownValueNode*`, `TestEncryptDecryptEntryRoundTripIsByteIdentical` |
| P2 | `internal/gage/field_test.go` | the four `TestEntryField*` |
| P2 | `internal/gage/testsupport_test.go` | `TestMain` |
| P2 | `internal/gage/unlock_test.go` | `TestUnlockRejectsAPrompterAnsweringTheWrongKind` |
| P2 | `internal/gage/vault_test.go` | `TestIdentityCopiesShareOneClose`, `TestNilIdentityCloseDoesNotPanic`, `TestUsingAnIdentityAfterCloseFails`, `TestZeroIdentityIsSafeToCloseAndUnusable` |
| P3 | `cmd/gage/complete_test.go` | `TestEntryTitleCandidatesNoSessionIsNil`, `TestFlagCandidatesMatchesTheRealCobraCommand`, `TestSessionCompleterDoWiresCandidatesIntoReadlinesContract` |
| P3 | `cmd/gage/insert_edit_test.go` | `TestInsertEditRejectedAlongsideMultilineOrValueStdinBeforeAnyIO` |
| P3 | `cmd/gage/session_test.go` | `TestBareGageWithPipedStdinPrintsHelp` |
| P3 | `internal/gage/generate_test.go` | `TestGenerateDrawsFromCryptoRandNotMathRand`, `TestRandomStringRejectsNonPositiveLengthAndEmptyAlphabet` |
| P4 | `cmd/gage/sync_cli_test.go` | `TestSyncReportsAPushEvenWhenNothingWasSent` |
| P4 | `internal/gage/gitrepo/auth_wiring_test.go` | `TestFetchAndPushCarryTheStoredTokenToGoGit` |
| P4 | `internal/gage/gitrepo/remote_test.go` | `TestRemoteStateStringCoversAllStatesAndUnknown` |
| P4 | `internal/gage/syncerr/syncerr_test.go` | `TestClassifyPassesNilThrough`, `TestClassifyPushCatchesGoGitsRealRejection` |
| P4 | `internal/gage/syncresolve_test.go` | `TestNilPrompterRefusesRatherThanPanicking` |
| P5 | `cmd/gage/move_test.go` | `TestMvRequiresToVaultFlag`, `TestMvToUnregisteredVaultFailsCleanly` |
| P5 | `internal/gage/move_test.go` | `TestMoveRejectsTwoRegistrationsOfOneVaultAsSourceAndDestination`, `TestSessionIndexUpdatesBothVaultsWithoutARebuild`, `TestTwoVaultLocksBothHeldForTheWholeOperation` |
| P5 | `internal/gage/recipient_test.go` | `TestRecipientListMatchesConfigExactly`, `TestVerifyReadsNeitherIdentityNorPrompter` |
| P5 | `internal/gage/recipients/recipients_test.go` | `TestReadRejectsALineLongerThanTheScannerCanBuffer`, `TestReadSkipsBlankLines` |
| P6 | `cmd/gage/script_test.go` | `TestAssumeYesAnswersTheTrustQuestionAndNothingElse`, `TestAssumeYesOverANonConflictPrompterStaysNonConflict`, `TestMissingScriptFileIsAUsageError`, `TestTerminalPrompterOfWalksDecoratorChains` |
| P6 | `internal/gage/gitrepo/history_test.go` | `TestLogAndHeadCommitterTimeOnARepoWithNoCommitsYet` (split: `log` half P6, `HeadCommitterTime` half P7 · E3) |
| P8 | `internal/gage/recovery_enroll_test.go` | `TestRecoverDeviceZeroesTheSecretItWasHanded` |

- The milestone docs' own mapping tables and "Inventory accounting"
  sections are where each claim was made. Grep the plan directory for the
  Go name.

## Decisions to make first

All four are **resolved as proposed**, and match
[Q-INVENTORY-GUARD](../open-questions.md#q-inventory-guard). Two additions
came out of implementing them:

- Lines starting with `#` are comments in the exceptions file. Each
  `excused` row, and each non-obvious `ported` one, carries its reason on
  the comment line above it.
- `ref` may list several refs, comma-separated. A guard test checks that
  each one resolves: a `Q-NAME` must have its `{#q-name}` anchor in
  `open-questions.md`, and a path must be a file under `plans/rust-port/`.
  A ref a reader can't follow is caught the same way as a missing one.

1. **Where the guard lives.**
   - Proposed: `crates/gage-cli/tests/parity_inventory.rs`, an integration
     test. It reads the repo through `env!("CARGO_MANIFEST_DIR")/../..`.
   - It is a test, not a script, so `make test` runs it on all three CI
     platforms with no extra wiring.
   - It is not in `crates/gage`, since it reads the CLI crate's sources too.
2. **How Rust test names are collected.**
   - Proposed: walk `crates/**/*.rs` with `std::fs`, adding no dependency.
   - Take the `fn` that follows a `#[test]` attribute line, allowing other
     attribute lines (`#[cfg(...)]`, `#[cfg_attr(...)]`, `#[ignore]`)
     between the two.
   - Platform-cfg'd tests count. They are real ports that run on the OS
     they apply to.
   - Go's `t.Run` subtests and Rust's per-case tests are both fine: the
     unit is the Go top-level function.
3. **The exceptions format.** Proposed columns, tab-separated, with a header
   row:

   ```
   go_file  go_test  disposition  rust_tests  ref
   ```

   - `disposition` is `ported` or `excused`.
   - `rust_tests` is a comma-separated list of fn names. It is required for
     `ported` and empty for `excused`.
   - `ref` is required: a `Q-` anchor or a milestone-doc path. It says
     where the mapping or excuse was decided.
   - `(go_file, go_test)` is the key. Go names are unique per package, not
     per repo: `TestMain` appears three times.
4. **Split rows.** A Go test whose halves landed in different phases (e.g.
   `TestLogAndHeadCommitterTimeOnARepoWithNoCommitsYet`) is one `ported`
   row that lists every Rust test. Its `ref` names both milestone docs.

## Tasks

### 1. The guard, red first

- [x] Tests for the guard's own parsing, as unit tests inside the
      integration-test file:
  - the name normalizer (`TestEditYAMLScratchFileCreated0600` ↔
    `edit_yaml_scratch_file_created_0600`)
  - the `#[test]` scanner with interleaved attributes
  - the exceptions parser rejecting:
    - a missing `ref`
    - `ported` with no Rust tests
    - `excused` with Rust tests
    - a duplicate key
- [x] `every_inventory_row_is_ported_or_excused` fails with the list of
      unmatched rows, when the exceptions file is empty or holds only its
      header row. Run against the header-only file, it listed all 95.
- [x] `every_ported_exception_names_real_tests`.
- [x] `no_exception_is_stale`: each exception's key is in the inventory, and
      its Go name does not already name-match.
- [x] Added: `every_exception_ref_resolves`, with
      `a_ref_resolves_to_a_q_anchor_or_an_existing_plan_doc` for the
      resolver (see "Decisions" above).
- The scanner also asserts that it found more than 1,000 Rust tests. A
  moved tree or a changed attribute style would otherwise make every
  check vacuous.

### 2. The audit, file by file

For each row in the Go sources table above:

- Read the Go test at the pinned SHA.
- Find the claim in the milestone doc.
- Read the Rust test it names.
- Record one of three outcomes:
  - **Equivalent:** a `ported` row naming the Rust test or tests.
  - **Deliberately not ported:** an `excused` row whose `ref` already
    explains why, or a new `Q-` entry if nothing does.
    - Language guarantees, like the `nil`-receiver rows, need such a
      reason: Rust has no nil `Identity`, and the type system rules out
      the zero value.
    - So do the `TestMain` harness rows, and rows tied to Go's build
      metadata such as `ResolveBuildInfo`.
  - **Missing:** port it, tests-first, in this task. Fix whatever it
    exposes. Record the port in the original milestone's doc and the
    ledger.

Checklist, one box per Go file group:

- [x] P1 (24): `help`, `main_integration`, `vaultid_keying`, `version`, `atomicfile`, `config`, `devicename`, `exitcode`, `vaultlock`, `xdgpaths`
  - **Missing:** `vaultid_keying` `TestCloneCopiesTheVaultsIDIntoGlobalConfig`.
    M1 sent it to P4 · M8a, and P4 planning never claimed it.
    `clone_produces_a_registered_working_vault` checks only that the id
    is non-empty. Ported as `clone_copies_the_vaults_id_into_global_config`
    (green on first run: the behavior was right, the test was missing).
    It name-matches, so it has no exception row. Recorded in M1's doc,
    P4's index and ledger §5.
  - cmd `TestMain` moves from "excused" (M0b) to `ported`. Its fake-editor
    re-entry and scrypt hook have Rust tests. Only the build-dir cleanup
    has nothing to port.
  - `TestLinkTimeVersionInjection` is `excused`, as it isn't a `#[test]`:
    it is `make stamp-check`, a test.yml step.
- [x] P2 (43): `entry` (CLI), `insert_field`, `prompter`, `crypt`, `entry_crud`, `entry` (library), `field`, `testsupport`, `unlock`, `vault`
  - **Tightened:** the five `insert_field` refusal rows. The Rust table
    test checked Usage and no unlock, but not that each message names its
    mistake, which every Go row asserts. Now each case asserts its reason,
    and the stripping case asserts `"user" was given more than once`.
  - **Unrecorded port:** `prompter` `TestResolveConflictNeverGuesses`. P2
    sent it to P4 · M8b, whose doc never claimed it.
    `conflict_prompt_re_asks_on_an_unknown_answer` covers it, and now
    also asserts Go's re-ask count. Recorded in M8b's doc.
  - `TestTimestampUnmarshalYAMLRejectsNonStringNode` is
    `collections_where_strings_belong_are_refused` (its `created:` rows).
    It had no marker; one is added.
  - The 4 `vault` rows and 2 `crypt` rows are excused as compile-time:
    `compile_fail` doctests, each paired with a compiling sibling, or no
    nil or zero value to test.
- [x] P3 (7): `complete`, `insert_edit`, `session`, `generate`
  - Go's crypto-rand row also checks for an unbiased draw by reading the
    source (`rand.Int`, not a modulo). In Rust that is proven by behavior
    (`random_string_rejects_bytes_that_would_bias_the_draw`), so the row
    lists both tests.
- [x] P4 (6): `sync_cli`, `auth_wiring`, `remote`, `syncerr`, `syncresolve`
  - `TestSyncReportsAPushEvenWhenNothingWasSent` is ported with its
    assertion flipped: Q-SYNC-PUSHED fixes Go's bug F2.
- [x] P5 (9): `move` (CLI and library), `recipient`, `recipients`
  - `TestRecipientListMatchesConfigExactly` also has to reflect an add and
    a remove. The row lists the two `recipient_tests` that check
    `recipients()` after each.
  - `TestReadSkipsBlankLines` had no marker; one is added.
  - `TestReadRejectsALineLongerThanTheScannerCanBuffer` keeps M9a's
    decision: an overlong line comes back whole, and no cap is invented.
- [x] P6 (5): `script`, `history`
- [x] P8 (1): `recovery_enroll`
- [x] The guard is green with the final exceptions file: 94 rows, **76
      `ported`** and **18 `excused`**. The 95th row, the late clone-id
      port, now name-matches.
  - The 18 excused rows: P1 6 (`help` helper, stamp CI step, 2
    `runtime/debug` rows, `WrapNil`, `xdgpaths` closed enum), P2 9, P4 2
    (nil classifier input, nil prompter), P6 1 (Q-SCRIPT-PASSPHRASE).
- [x] Proof the guard bites: rename one name-matched Rust test, see
      `every_inventory_row_is_ported_or_excused` fail naming its Go row,
      then revert. Rename one test listed in an exception, see
      `every_ported_exception_names_real_tests` fail, then revert.
  - Done on this branch:
    - Renaming `success_is_zero` failed the first test with
      `P1 internal/gage/exitcode/exitcode_test.go TestSuccessIsZero`.
    - Renaming `ok_is_success` failed the second with
      `TestNilErrorIsSuccess -> ok_is_success`.
  - Beyond the plan: appending an exception for the name-matched
    `TestSuccessIsZero` with ref `Q-NO-SUCH-ENTRY` failed both
    `no_exception_is_stale` and `every_exception_ref_resolves`.
  - Each change was reverted, and `git diff` showed the file clean.

### 3. Leftover boxes in milestone docs

- [x] M0a `.github/workflows/test.yml`, `lint.yml`, `audit.yml`: tick each
      one, citing the workflow file as it stands. Where the doc's spec and
      the workflow differ, say so in the doc.
  - lint.yml matches its spec. test.yml runs `cargo fetch --locked`
    inside `make cover`, not as its own step, and has M0b's
    `make stamp-check`. audit.yml adds `workflow_dispatch`.
- [x] M0a gosec mapping: point the box at S2's table. S2 writes the table
      and ticks the box.
- [x] M8a1 cold CI build: tick it, citing a CI run on all three OSes with
      an empty cache. If none exists, trigger one with `workflow_dispatch`
      after clearing the cache, with the user's go-ahead.
  - None existed. M8a1's first run (38826745518) was cold on macOS and
    Windows only: Linux restored a partial rust-cache. None of the 40
    most recent `test.yml` runs was cold on all three.
  - With the user's go-ahead, all 35 Actions caches were deleted and
    `test.yml` was dispatched on `main` (`6552930`): run 39549666209.
  - "No cache found." on all three OSes, and all three jobs passed.
- [x] Sweep: `grep -n '^\s*- \[ \]' plans/rust-port/*/*.md` returns only P9's
      own boxes.
  - Besides P9's own, it returns only M0a's gosec box, which points at
    S2's table, and S2 ticks it.

### 4. Accounting

- [x] For each phase, check that its index's "Inventory accounting" table
      adds up to the §5 count, adjusted by the cross-phase claims each
      ledger §5 paragraph lists. Fix the arithmetic in the doc where it
      doesn't add up.
  - Every phase's milestone split adds up to its §5 count, and each
    milestone total equals its per-file counts.
  - P1 is 97 + 89. P2 is 5 (to P1) + 83 + 37 + 54 + 14 moved. P3 is
    221 + 4 moved. P4 is 7 (M1) + 57 + 53 + 49. P5 is 26 + 37 + 38 + 20.
    P6 is 24 + 10 + 27 + 26. P7 is 15 + 55 + 60 + 71. P8 is 11 + 25 + 30
    + 26.
  - Fixed:
    - P4's index said no rows moved in. Three had; it now lists them and
      what became of each.
    - M1's task 5 heading omitted its 2 `vaultid_keying` rows. Without
      them, that file's 21 rows didn't add up.
    - §5 had no P8 paragraph.
- [x] Add a "Resolved in P9" paragraph to ledger §5. It gives the
      exceptions file, the guard test, the disposition counts, and any
      rows ported late.

## Definition of done

- `parity_inventory`'s tests are green locally and on Linux, macOS and
  Windows CI.
- Every one of the 95 rows (or whatever the count is at the start, if
  `main` moved) has a reviewed disposition. No row is `excused` without a
  `ref` that explains it.
- The guard was proven to fail, and that is recorded in task 2.
- No unchecked box outside P9 in any milestone doc.
- `make lint` and `make test` clean.

**Done.** Every item above holds.

- `make lint` and `make test` are clean locally.
- On PR #91 at `ff9e3f7`, the Test (39551168244), Lint (39551168299) and
  Audit (39551168260) workflows all passed on the first run.
- `parity_inventory`'s 13 tests ran and passed on Linux, macOS and
  Windows.
- The one box left outside P9 is M0a's gosec mapping, which points at
  S2's table.
