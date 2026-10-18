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

- [ ] Tests for the guard's own parsing, as unit tests inside the
      integration-test file:
  - the name normalizer (`TestEditYAMLScratchFileCreated0600` ↔
    `edit_yaml_scratch_file_created_0600`)
  - the `#[test]` scanner with interleaved attributes
  - the exceptions parser rejecting:
    - a missing `ref`
    - `ported` with no Rust tests
    - `excused` with Rust tests
    - a duplicate key
- [ ] `every_inventory_row_is_ported_or_excused` fails with the list of
      unmatched rows, when the exceptions file is empty or holds only its
      header row.
- [ ] `every_ported_exception_names_real_tests`.
- [ ] `no_exception_is_stale`: each exception's key is in the inventory, and
      its Go name does not already name-match.

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

- [ ] P1 (24): `help`, `main_integration`, `vaultid_keying`, `version`, `atomicfile`, `config`, `devicename`, `exitcode`, `vaultlock`, `xdgpaths`
- [ ] P2 (43): `entry` (CLI), `insert_field`, `prompter`, `crypt`, `entry_crud`, `entry` (library), `field`, `testsupport`, `unlock`, `vault`
- [ ] P3 (7): `complete`, `insert_edit`, `session`, `generate`
- [ ] P4 (6): `sync_cli`, `auth_wiring`, `remote`, `syncerr`, `syncresolve`
- [ ] P5 (9): `move` (CLI and library), `recipient`, `recipients`
- [ ] P6 (5): `script`, `history`
- [ ] P8 (1): `recovery_enroll`
- [ ] The guard is green with the final exceptions file. Report the count
      in each disposition here.
- [ ] Proof the guard bites: rename one name-matched Rust test, see
      `every_inventory_row_is_ported_or_excused` fail naming its Go row,
      then revert. Rename one test listed in an exception, see
      `every_ported_exception_names_real_tests` fail, then revert.

### 3. Leftover boxes in milestone docs

- [ ] M0a `.github/workflows/test.yml`, `lint.yml`, `audit.yml`: tick each
      one, citing the workflow file as it stands. Where the doc's spec and
      the workflow differ, say so in the doc.
- [ ] M0a gosec mapping: point the box at S2's table. S2 writes the table
      and ticks the box.
- [ ] M8a1 cold CI build: tick it, citing a CI run on all three OSes with
      an empty cache. If none exists, trigger one with `workflow_dispatch`
      after clearing the cache, with the user's go-ahead.
- [ ] Sweep: `grep -n '^\s*- \[ \]' plans/rust-port/*/*.md` returns only P9's
      own boxes.

### 4. Accounting

- [ ] For each phase, check that its index's "Inventory accounting" table
      adds up to the §5 count, adjusted by the cross-phase claims each
      ledger §5 paragraph lists. Fix the arithmetic in the doc where it
      doesn't add up.
- [ ] Add a "Resolved in P9" paragraph to ledger §5. It gives the
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
