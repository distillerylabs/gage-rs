# M2 — Identity, age wrapper, memory protection

[← P1 · M1](../p1-foundation/m1-vault-lifecycle.md) · [P2 plan](index.md) · next: [M3 — Entry format](m3-entry-format.md)

> **Recommended model: Opus.** The one milestone not to economize on:
> crypto correctness, page locking and core-dump suppression on three
> platforms, and the ordering of secret material in memory. Subtly wrong
> here is the worst outcome in the project, and it is the failure a passing
> suite is least likely to catch. **Run an Opus review pass over the
> tests** before M3 starts.

## Goal

The first milestone where key material exists. In dependency order:

1. **Memory protection**: `memlock` (page-aligned, per-key allocation;
   `mlock`/`VirtualLock`) behind the `Locker` seam.
2. **A thin age wrapper**: encrypt bytes to recipients, decrypt bytes with
   an identity; typed errors. No files, no entry format.
3. **`Identity`**: key material locked in memory, wiped and unlocked in
   `Drop`.
4. **Passphrase identities**: generate an X25519 keypair, wrap it with
   age's scrypt recipient at the shipped work factor, write it to
   `$GAGE_DATA/identities/<vault-id>/<device>.age` with the
   `vault-name.txt` marker; `Vault::unlock(&dyn Prompter)` reads it back.
5. **The terminal prompter**: masked passphrase entry, new-passphrase
   confirmation, the three-attempt retry policy, warnings.
6. **Core-dump suppression** at `gage` startup.
7. **`gage init` completed**: it generates this device's identity, so
   `--recipient` becomes optional and additive
   ([Q-INIT-INTERIM](../open-questions.md#q-init-interim) ends here).

## Depends on

- **P1**: `Prompter`/`UnlockRequest`/`Identity` skeletons, `paths`
  (`identity_file_path`, `identities_dir`, both validating vault id and
  device name), `atomicfile`, global `config` (`device`, `method`,
  `pubkey`), `Vault::create`, `gage init`, the exit-code taxonomy,
  `recipients` reader/writer.

## Design references

- Design doc "Local identity storage" (why the file lives under
  `$GAGE_DATA` and nowhere else; why losing it is a recovery problem),
  "Session model: `gage` as its own agent" (page locking, core dumps, the
  honest Windows gap), "Library architecture" (method-agnostic unlock
  exchange, typed unlock errors, retry policy in the frontend),
  "Decryption methods are per-device".
- Go open questions A3 (scrypt sole-recipient invariant), Q-DEVICE-NAME,
  Q-METHOD-SCOPE, A20 (vault-id keying, the marker file).
- [Q-AGE-CRATE](../open-questions.md#q-age-crate),
  [Q-PLATFORM-CRATES](../open-questions.md#q-platform-crates),
  [Q-IDENTITY-SECRET](../open-questions.md#q-identity-secret),
  [Q-SCRYPT-HOOK](../open-questions.md#q-scrypt-hook).

## Go sources

- Go plans `gage-cli-design/m2-identity-and-crypto.md` and
  `gage-cli-init-design/e0-vault-id-keying.md` (identity path and marker).
- Parity ledger rows: §3 M2 (+ E0 identity paths); §2 `844f880`
  (memory-locking fix, the `Alloc` regression test), `e52828a` (scrypt
  test hook and guard tests), `ddce8b1` (the `Prompter::warn` half), E0
  `0d710d4` (M2's half).
- Go sources at the pinned SHA: `internal/gage/{crypt,identity,locker,identityfile,unlock,prompter}.go`,
  `internal/gage/memlock/`, `cmd/gage/{prompter,coredump_unix,coredump_windows,init}.go`.
- Test inventory: the P2 rows of `memlock` (4), `crypt` (17), `vault`
  (11), `unlock` (23), `identityfile_marker` (4), `prompter` (11 of 17),
  `prompter_pty` (1), `coredump` (3), `init_identity` (8) and
  `testsupport` (1): 83 rows. Plus the rows P1 moved here (task 8).

## Decisions to make first

Recommended answers; confirm them when this plan is reviewed.

- **Crates** ([Q-AGE-CRATE](../open-questions.md#q-age-crate),
  [Q-PLATFORM-CRATES](../open-questions.md#q-platform-crates)): `age`
  0.12.1 with default features; `zeroize`; `libc` (Unix) and `windows-sys`
  (Windows) direct, for `memlock` in the library and core dumps in the
  CLI; `rpassword` 7.5 in `gage-cli`; `portable-pty` 0.9 as a
  `gage-cli` dev-dependency. `deny.toml` gains BSD-3-Clause. The workspace
  `Cargo.toml` gains `opt-level = 3` for `scrypt`, `salsa20`, `pbkdf2`,
  `sha2` and `hmac` under `[profile.dev.package.*]`.
- **Where the code lives.** Library: `memlock`, the `Locker` trait and its
  real implementation, the age wrapper (`crypt`), `Identity`, identity-file
  create/reuse/remove and the marker, `Vault::unlock`. CLI: the terminal
  prompter, the retry policy, core-dump suppression (a process-wide policy
  belongs to the binary, as in Go's `cmd/gage`).
- **Shipped work factor 19, test factor 10, decrypt cap 22** (Go's
  numbers and reasons; see `identityfile.go`). The cap is set explicitly
  on every scrypt identity, because `age`'s default cap is calibrated per
  machine. All scrypt identities and recipients are built by one function
  each, so the guard tests have one place to check.
- **Wrong-passphrase classification** (Rust `age` differs from Go's): a
  decrypt of an identity file whose header is scrypt-only
  (`Decryptor::is_scrypt()`) that fails with `DecryptError::DecryptionFailed`
  is a wrong passphrase and re-prompts. A header that isn't scrypt-only
  fails with `NoMatchingKeys` and is a corrupt identity file. Every other
  failure (`InvalidHeader`, `InvalidMac`, `ExcessiveWork`, `UnknownFormat`,
  a payload read error) is a corrupt identity file and never re-prompts.
  Accepted, as in Go: a damaged scrypt stanza body is indistinguishable
  from a wrong passphrase, because the AEAD can't tell them apart.
- **Entry decrypt classification**: `NoMatchingKeys` is `NotARecipient`
  (LockedOrAuth); everything else is `CorruptCiphertext` (Conflict),
  matching Go's exit codes. Nothing partial is ever returned.
- **Plugin recipients** stay legal in `.age-recipients` (M1's shape-only
  check) but the wrapper parses only X25519 recipients, so encrypting to a
  plugin recipient is refused with a typed error, as Go's `ParseRecipient`.
- **`Identity`'s memory layout**
  ([Q-IDENTITY-SECRET](../open-questions.md#q-identity-secret)): the
  `AGE-SECRET-KEY-1…` line in a `memlock::LockedBuf` (page-aligned via
  `std::alloc` with a page-size `Layout`, rounded to whole pages, so no
  other allocation shares its pages). The X25519 key is parsed from it per
  operation (`age::x25519::Identity` zeroizes on drop). `Drop` order:
  zeroize, then unlock, then deallocate; unlocking happens only if the lock
  succeeded. Recorded gap, as Go records its own: `age`'s bech32 decoding
  makes transient copies that are neither locked nor wiped.
- **`Identity` carries** its device name and public key (`device()`,
  `recipient()`), and whether its page lock took (`page_locked()`). It
  does not carry a `Prompter`: Go keeps one for P4's push warnings, and P4
  decides how Rust routes those.
- **Mapping Go's `Close` tests onto `Drop`.** Rust's ownership rules make
  several of them compile errors rather than runtime checks:
  - use after close, and copies sharing one close: `compile_fail` doctests
    on `Identity` (no new dependency; `cargo test` runs doctests);
  - nil and zero-value identities: excused in writing, since neither is
    constructible outside the library;
  - idempotence: ported as "dropping releases the page lock exactly once",
    with a counting `Locker`.
- **Scrypt test hook** ([Q-SCRYPT-HOOK](../open-questions.md#q-scrypt-hook)):
  `const SHIPPED_SCRYPT_WORK_FACTOR: u8 = 19` and a live
  `static SCRYPT_WORK_FACTOR: AtomicU8` that the recipient constructor
  reads. The setter exists only under `cfg(any(test, feature = "test-support"))`,
  so the shipped binary (which never enables `test-support`) has no way to
  lower it. Tests call an idempotent `testsupport::fast_scrypt()` (a
  `std::sync::Once`) from the fixtures that create or unlock vaults; Rust
  has no `TestMain`, and a test that forgets it is slow, not wrong.
- **`UnlockRequest` gains `purpose`** (`Unlock` or `Create`), as Go's,
  so the frontend knows to ask for a new passphrase twice. The passphrase
  in `UnlockResponse` becomes `Zeroizing<String>`, the secret type the M0b
  comment promised.
- **`Prompter` gains `warn(&str)`** (returns nothing). `value` arrives with
  M4, which first asks for it; the session, clone, recipient-change and
  conflict methods arrive with their phases.
- **Retry policy**: the CLI prompter answers attempts 1 to 3 and refuses
  attempt 4 with a typed "too many attempts" (LockedOrAuth). The library
  keeps Go's runaway guard of 100 attempts and retries only on a wrong
  passphrase.
- **Masked input**: when stdin is a terminal, `rpassword` reads with echo
  off (it opens the controlling terminal); otherwise a line is read from
  the injected stdin, buffered across calls, so `--stdin` scripts and
  in-process tests work. Prompts go to stderr. The PTY test points
  `rpassword`'s configured input at the PTY's slave path in-process, the
  way Go hands `x/term` the slave fd.
- **Core dumps**: `setrlimit(RLIMIT_CORE, 0)` on Unix;
  `SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX)` on Windows,
  documented as the narrower guarantee it is. Called first in `main`.
  Linux `prctl(PR_SET_DUMPABLE)` is not added: Go doesn't, and it also
  blocks debuggers and `/proc` access, which is a behavior change.
- **`init` ordering**, as Go: validate everything knowable, ask for the
  new passphrase, generate and write the identity, then create the vault;
  if creation fails, remove the identity (the rollback that keeps failed
  `init`s from leaving orphaned keys) and leave no vault behind.

## Tasks

Tests first, at task granularity. Library tasks (1–5) before CLI tasks
(6–8).

### 1. `memlock` and the `Locker` seam — `memlock` (4 rows)

- [x] `TestLockUnlockRoundTrip` (a key-sized buffer; skipped only for a
      resource-limit refusal: `ENOMEM`/`EPERM` on Unix, working-set or
      quota errors on Windows; any other error fails),
      `TestEmptySliceIsANoOp`.
- [x] `TestAllocGivesEachBufferItsOwnPage` (the `844f880` regression),
      `TestUnlockingOneBufferLeavesAnotherLocked`.
- [x] `memlock::{lock, unlock}` and `LockedBuf` (page-aligned, own pages,
      fixed length, zeroized in `Drop`) in one `#[allow(unsafe_code)]`
      module; `Locker` trait with the real implementation as the default.
- Rust notes: Go's capacity-cap assertion becomes "the buffer can't grow"
  (`LockedBuf` has a fixed length and no `push`), asserted by its API.

### 2. The age wrapper and the work-factor hook — `crypt` (17 rows), `testsupport` (1)

- [x] Round trips: `TestEncryptDecryptRoundTripIsByteIdentical`,
      `TestEachRecipientCanDecryptIndependently`,
      `TestEncryptToEmptyPayloadRoundTrips`,
      `TestPassphraseRecipientAloneRoundTrips`.
- [x] Refusals: `TestEncryptToZeroRecipientsIsRefused`,
      `TestScryptRecipientCannotBeCombined` (with an X25519 recipient and
      with a second scrypt recipient; gage refuses before `age` does),
      `TestParseRecipientRejectsNonEncryptableStrings` (plugin recipient,
      secret key, malformed), `TestRecipientStringRendersTheParsedSpelling`
      (a passphrase recipient renders `<passphrase>`, never a key).
- [x] Decrypt failures: `TestDecryptWithANonRecipientFailsTyped`,
      `TestCorruptedCiphertextFailsCleanly` (truncated header, truncated
      payload, a flipped payload byte, garbage, empty input; each is
      `CorruptCiphertext` with no partial plaintext).
- [x] Work factor ⚑: `TestScryptWorkFactorIsDeliberate` (the constant is
      19), `TestScryptWorkFactorIsActuallyApplied` (read the
      `-> scrypt <salt> <logN>` stanza off a real file),
      `TestShippedWorkFactorReachesARealAgeFile` (restore 19 for one
      encryption and read it back off the header; needs the dev-profile
      override to run in under a second),
      `TestOnlyTheTestHooksWriteTheWorkFactors` (a source scan: the only
      store to the atomic is in the setter; the enrollment half is P7's),
      `TestEveryScryptIdentityCapsItsWorkFactor` (a source scan:
      `age::scrypt::Identity::new` appears only in the one constructor,
      which sets the cap; the enrollment half is P7's).
- [x] `TestMain` → `testsupport::fast_scrypt()`, plus a test that it
      lowers the live factor and is idempotent.
- Done notes: `TestCorruptedCiphertextFailsCleanly` keeps Go's six cases;
  the two that damage the header's recipient stanza keep Go's
  either-error assertion (a damaged X25519 stanza is indistinguishable from
  someone else's), and the plan's five, plus a mid-payload truncation, must
  be `CorruptCiphertext`. `TestShippedWorkFactorReachesARealAgeFile` runs
  in a child process that never calls the hook, since lowering and
  restoring a process-wide value races Rust's parallel tests
  ([Q-SCRYPT-HOOK](../open-questions.md#q-scrypt-hook)). Added beyond Go:
  the recipient-side twin of the cap scan (one `scrypt::Recipient::new`, in
  the function that sets the factor). All three guard scans were
  mutation-checked (a stray store, a dropped `set_work_factor`).
- [x] Excused: `TestEncryptRefusesAZeroValueRecipient`. A `Recipient` has
      private fields and no `Default`, so the only way to get one is its
      constructors; a `compile_fail` doctest asserts it can't be built by
      hand. `TestDecryptWithAClosedIdentityFails` becomes a `compile_fail`
      doctest (decrypting with a dropped `Identity`), in task 3.

### 3. `Identity` — `vault` (11 rows)

- [x] `TestIdentityCloseReleasesTheLockAndZeroesTheKey` (a recording
      `Locker` sees the buffer all zeros when its `unlock` runs),
      `TestIdentityCloseIsIdempotent` (as "drop unlocks exactly once"),
      `TestCloseAfterAFailedPageLockDoesNotUnlockAnything`.
- [x] `TestPageLockFailureWarnsOnceAndProceeds`,
      `TestPageLockIsAttemptedExactlyOncePerUnlock`,
      `TestUnlockPageLocksOrWarnsButNeverBoth` (with the real locker, so it
      holds whether or not this host allows locking),
      `TestDefaultLockerIsTheRealMemlockBackedOne`. These need
      `Vault::unlock`, so they are written in task 5's red-green cycle.
- [x] `compile_fail` doctests: `TestUsingAnIdentityAfterCloseFails`,
      `TestIdentityCopiesShareOneClose` (not `Clone`),
      `crypt`'s `TestDecryptWithAClosedIdentityFails`.
- [x] Excused: `TestNilIdentityCloseDoesNotPanic`,
      `TestZeroIdentityIsSafeToCloseAndUnusable`. Rust has no nil
      `Identity`, and there is no zero value: the type can't be
      constructed outside the library.
- Done notes: stable rustdoc does not check the `E0xxx` codes on a
  `compile_fail` block (verified by mislabelling one), so each is paired
  with a compiling sibling: the same code minus the offending line. The
  `Identity` rows live in `crates/gage/src/unlock_tests.rs` with task 5's.

### 4. Identity files — `unlock` (10 of 23 rows), `identityfile_marker` (4)

- [x] Create: `TestCreateIdentityFileIsPassphraseWrappedAndSoleRecipient`,
      `TestCreateIdentityWritesTheFileWithTightPermissions` (directory
      `0700`, tightened if it exists looser; file `0600`; Unix only),
      `TestCreateIdentityRejectsAnEmptyPassphrase`,
      `TestIdentityFileIsAgeKeygenCompatible` (the decrypted plaintext is
      `# created: …`, `# public key: …`, the key line; it parses with
      `age::IdentityFile::from_buffer`).
- [x] Reuse, not overwrite: `TestCreateIdentityReusesAnExistingFile`,
      `TestCreateIdentityReuseRejectsWrongPassphrase`.
- [x] Untrusted input ⚑: `TestCreateIdentityRefusesAnUnsafeDeviceName`,
      `TestIdentityFilePathRefusesAnUnsafeDeviceName`,
      `TestIdentityFilePathRefusesAnUnsafeVaultName` (the vault *id*,
      since E0).
- [x] `TestRemoveIdentityIsTheRollbackPathAndNothingMore`.
- [x] Marker: `TestIdentitiesDirectoryCarriesAPlaintextMarkerNamingTheVault`,
      `TestNothingReadsTheMarkerToMakeADecision`,
      `TestRemovingANonLastIdentityLeavesTheMarkerInPlace`,
      `TestRemovingTheLastIdentityLeavesNoOrphanedDirectory`.
- Done notes: `TestNothingReadsTheMarkerToMakeADecision` ports its unlock
  half; Go's `HasIdentity` and `ListIdentities` halves move with those
  functions (P4 · M8a's clone, and `identity list`). Added: the
  "tightened if it exists looser" half as its own test
  (`create_identity_tightens_an_existing_looser_directory`), and a file
  that decrypts to no usable key line is corrupt.
- [x] The `.age-recipients` round trip M1 deferred: every non-blank,
      non-comment line of a written file parses with
      `age::x25519::Recipient`, the parser stock `age -R` uses for those
      lines. age's own `read_recipients` needs `cli-common`, a frontend
      dependency.

### 5. `Vault::unlock` — `unlock` (13 of 23 rows)

- [x] `TestUnlockProducesAnIdentityThatDecrypts`,
      `TestUnlockRequestsThePassphraseThroughThePrompter` (kind
      `Passphrase`, purpose `Unlock`, vault name, device, attempt 1).
- [x] Distinguishable errors: `TestUnlockWithTheWrongPassphraseIsDistinguishable`,
      `TestUnlockWithACorruptIdentityFileIsDistinguishable`,
      `TestUnlockWithNoLocalIdentityIsDistinguishable`,
      `TestUnlockWithAWellFormedButWrongKindOfFileIsCorruptNotWrongPassphrase`
      (an X25519-encrypted file where the identity file should be).
- [x] Retry: `TestUnlockRetriesThroughThePrompterAndStopsWhenItStops`,
      `TestUnlockDoesNotRetryOnNonPassphraseFailures`,
      `TestUnlockRejectsAnEmptyPassphrase`,
      ~~`TestUnlockRejectsAPrompterAnsweringTheWrongKind`~~: excused, a
      response of another kind is unconstructible. `UnlockResponse` has one
      variant per `Kind`, so the library's destructuring is irrefutable
      today and becomes a compile error, forcing the refusal arm, the day a
      second `Kind` is added.
- [x] Local method and untrusted input ⚑:
      `TestUnlockDispatchesOnTheLocalMethodNotTheVaultDefault`,
      `TestUnlockNeverBuildsAPathFromAnUnvalidatedDeviceName`,
      `TestUnlockWithNoRecordedDeviceIsAMissingIdentityNotAnUnsafeName`.
- [x] `UnlockError` replaces M0b's "not implemented" placeholder with
      variants carrying Go's exit codes (`WrongPassphrase`,
      `NoLocalIdentity`, `UnsupportedMethod`, a prompter that gave up:
      LockedOrAuth; `CorruptIdentityFile`: Conflict).
- Rust notes: `Vault` gets its `Locker` through a constructor used by
  tests only (`Vault::with_locker`); `from_registration` and `create` use
  the real one, which `TestDefaultLockerIsTheRealMemlockBackedOne` pins.
- Settled in implementation: `Vault` carries this device's `Dirs`, so
  `Vault::create(&dirs, &spec)` and `Vault::from_registration(&dirs, name,
  entry)` take them. Go reads its roots from the process environment;
  gage-rs injects them everywhere, and `unlock` needs them for both the
  registration (device, method) and the identity file. `unlock` keeps the
  documented `Vault::unlock(&dyn Prompter)` signature. One `UnlockError`
  (in `identityfile`, re-exported as `vault::UnlockError`) covers unlock,
  create and remove; a wrong passphrase carries the prompter's reason for
  stopping as its source. The 100-attempt runaway guard has no test (100
  scrypt passes); it isn't a Go row.

### 6. The terminal prompter — `prompter` (11 of 17 rows), `prompter_pty` (1)

- [x] `TestPromptsGoToTheErrorStreamNotStdout`,
      `TestReadingBufferedInputDoesNotSwallowFollowingLines`,
      `TestReadingPastEndOfInputFails`.
- [x] Unlock: `TestRetryPolicyLivesInThePrompter` (attempt 4 refused),
      `TestRetryPromptTellsTheUserWhichAttemptThisIs` ("attempt 2 of 3"),
      `TestNewPassphraseIsConfirmed` (purpose `Create` asks twice and
      re-asks on a mismatch, up to 3 times),
      `TestNewPassphraseRejectsAnEmptyOne`.
- [x] `TestConfirmDefaultsToNo`, `TestConfirmStillRendersDefaultNo`
      (`[y/N]`), `TestWarnIsSurfacedAndReturnsNothing`.
- [x] `TestPrompterRefusesAMethodItCannotSatisfy`: `Kind` is
      `#[non_exhaustive]`, so the CLI's match must have a wildcard arm,
      which returns a typed refusal. Untestable until a second `Kind`
      exists; excused with that reason, and the arm is reviewed instead.
- [x] Real terminal (Unix only, as Go):
      `TestRealPrompterReadsAPassphraseWithoutEchoing`, against a
      `portable-pty` pair: wait until the terminal's `ECHO` flag is off,
      type the passphrase, assert it came back, was never echoed, and that
      the prompt names the vault.
- [x] `main` swaps M0b's `NoPrompter` for the terminal prompter.
- Done notes: `TestPromptsGoToTheErrorStreamNotStdout` is about `main`'s
  wiring, so it is a real-binary smoke test (`assert_cmd`, redirected
  stdin; also the one run of the shipped KDF cost), with an in-process
  half driving `init` through the real prompter. `Io` no longer holds
  stdin's lock for the run: the prompter reads the same stream, and that
  lock isn't reentrant. The PTY test was mutation-checked (the same
  terminal read through the plain line reader fails on echo).
- Moved: `TestChooseOnAnEmptyCandidateListIsInternal`,
  `TestChooseRetriesThenGivesUp` (P3 · M6), `TestConfirmDefaultYesDefaultsToYes`
  (P4 · M8a), `TestResolveConflictNeverGuesses` (P4 · M8b),
  `TestConfirmRecipientChangeNamesBothDirectionsOfAMismatch`,
  `TestRecipientChangeCountSummarizesEveryShape` (P5 · M10).

### 7. Core dumps — `coredump` (3 rows)

- [x] `TestCoreDumpsAreDisabled`: process-level, so in a child process
      (the test binary re-executed, as M0b's `vaultlock` tests do); reads
      back `RLIMIT_CORE` = 0 on Unix and the error-mode bits on Windows.
- [x] `TestDisablingCoreDumpsIsIdempotent`.
- [x] `TestMainDisablesCoreDumpsAtStartup`: a source assertion that the
      call is `main`'s first statement, as Go's (a missing call is
      silent).
- Done notes: both behavioral tests run in child processes, because a
  lowered hard limit is permanent and inherited by children; the main test
  first undoes any suppression already in force, so it isn't vacuous on
  macOS. Mutation-checked (a no-op `disable_core_dumps` fails it).

### 8. `gage init` generates the identity — `init_identity` (8 rows) and rows from P1

- [x] `TestInitAsksForTheNewPassphraseThroughThePrompter`,
      `TestInitReportsTheDeviceAndPublicKey`,
      `TestInitThenUnlockDecryptsSomethingEncryptedToTheVault`,
      `TestUnlockTypedErrorsCarryTheRightExitCode`.
- [x] Failure leaves nothing behind: `TestInitLeavesNoVaultBehindWhenCreateFails`,
      `TestInitWithARejectedDeviceNameWritesNoIdentityFile`,
      `TestTwoFailedInitsLeaveNoOrphanedIdentities`,
      `TestInitAfterVaultRemoveGeneratesAFreshKeyRatherThanReusingTheOldOne`.
- [x] From P1 (M1's task 7 and partial rows):
      `TestInitWithNoRecipientGeneratesThisDevicesIdentity` (rewrites
      M1's interim "requires `--recipient`" test),
      `TestInitWritesTheWrappedIdentityFile`,
      `TestInitExtraRecipientFollowsTheDeviceKey` (device key first, then
      `--recipient` keys),
      `TestTheIDInitFilesTheIdentityUnderIsTheIDItCommits`; and the
      identity halves of `TestCreateDeviceNameLandsInRecipientAndIdentityPath`,
      `TestInitDeviceFlagAgreesEverywhere`,
      `TestReinitAfterVaultRemoveIsAnOrdinaryRetry`, and the `init` half of
      `TestInitAndIdentityAddRecordThePubkeyAndCloneDoesNot` (global
      config records `pubkey`).
- [x] The warning row of `errorprefix`'s table, end to end through
      `Prompter::warn` (no doubled `gage: gage:`).
- [x] `init`'s help text describes what it now does; `--recipient` is
      optional.
- [x] Q-INIT-INTERIM: mark the `--recipient` half done.
- Done notes: `init` validates every flag before asking for the passphrase
  and leaves the target-directory check (and rollback) where Go has them
  ([Q-INIT-ORDER](../open-questions.md#q-init-order)). The warning row of
  `errorprefix` is asserted on the library's two M2 warnings
  (`warnings_do_not_double_gages_own_prefix`), which the terminal prompter
  prints verbatim. Process slip, recorded: this task's implementation was
  written before its tests; the rollback tests were then mutation-checked
  (skipping the rollback fails both).

### 9. Ledger and docs

- [x] Parity ledger: §3 M2 row and §2 `844f880`, `e52828a`, `ddce8b1`,
      E0 rows updated.
- [x] `doc/cli/vaults.md`: `init` without `--recipient`, the identity file
      and its marker, the passphrase prompt and retry policy.

## Definition of done

Status: **done.** Every task above is implemented; the Opus review of
the tests is the test audit below; `make lint` and `make test` are green
on Linux, macOS and Windows CI, so the Windows `memlock` and core-dump
code now compiles and runs there, not only against `windows-sys`'s
declarations.

Test audit additions (beyond Go's rows, each mutation-checked):
`unlock_of_a_file_claiming_excessive_work_is_corrupt_not_wrong_passphrase`
(the behavioral half of the decrypt cap: a claimed factor above 22 is
corrupt after one prompt, not re-prompted);
`init_that_gets_no_passphrase_fails_locked_and_writes_nothing` (cancel,
empty, EOF, never-matching confirmation: LockedOrAuth, nothing written)
and M2's rows in the `errorprefix` table; and
`crates/gage-cli/tests/shipped_features.rs` (a non-dev build of `gage-cli`
never enables `gage/test-support`, so the shipped binary can't lower the
work factor; Q-SCRYPT-HOOK's premise, checked through cargo's resolver).
Also tightened: the keygen-compatibility test checks age's derived
recipient equals the reported key;
`every_unlock_failure_keeps_its_exit_code_through_the_cli` carries each
real unlock failure through `CliError` (Go's row covers only the
missing-identity one); the `Recipient` struct-literal `compile_fail`
doctest names every field, so only privacy can fail it; and a vaultlock
test proves a contended acquire waits out its whole timeout.


Every box above checked; all 83 claimed rows ported, or excused in this
doc with a reason, or moved in task 6's list; the rows from P1 in task 8
ported; the Opus review of the tests done; `make lint` and `make test`
clean locally and on Linux, macOS and Windows CI. A vault can be created
with a real identity, and that identity can be unlocked and used to
decrypt something encrypted to it. There are still no entries.
