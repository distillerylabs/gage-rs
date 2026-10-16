# M8a2 — Sync wiring

[← M8a1](m8a1-remote-primitives.md) · [P4 plan](index.md) · [#34](https://github.com/distillerylabs/gage-rs/issues/34) · next: [M8b — Conflict resolution](m8b-conflict-resolution.md)

> **Recommended model: Opus.** This milestone is about lock ordering. The
> automatic pull resets the working tree, so it needs the write lock, but
> only if the lock is free. The automatic push runs inside the write's own
> lock and must not deadlock on it. A manual sync waits for the lock.
> Getting any of these wrong either stalls reads behind writes or lets a
> pull reset the tree underneath a write. Review the lock tests again at
> the end (task 8).

## Goal

The integration half of Go's M8a, built on M8a1's primitives:

- every vault unlock fetches and fast-forwards, in both invocation modes
- every write pushes
- `sync`, `pull` and `push` exist
- `clone` exists
- `init --remote` publishes, and asks for a token inline (#38)

Divergence is detected, classified and reported. A divergence whose two
sides touched different entries merges and pushes, with no prompt and no
unlock. A real entry conflict is reported with a pointer to `gage sync`
and left alone. M8b resolves it.

## Depends on

- **M8a1:**
  - `gitrepo` remote operations, the file-level merge and the dirty-tree
    error
  - `syncerr`
  - `remoteauth` (`token_host`, `load`, `store`)
  - the `gittest` second clone and recorder
- **P2 · M4:** `Vault::with_write_lock` (`entry/crud.rs`) and the
  commit-per-write it wraps.
- **P3 · M6/M7:** `Session` and its `HeldVault`. `discard_index` is the
  documented hook for a pull. The private `hold` opens a vault without
  unlocking it.
- **P1 · M1:**
  - `CreateSpec.remote` and `[vaults.<name>.git] origin`
  - `.gitattributes`
  - the format-version check
  - name validation in `init`

## Design references

- Design doc, "Sync model". Local durability and network risk are kept
  apart:
  - automatic sync warns and proceeds
  - an explicit sync verb reports
- Design doc, "Sync (vault-generic, git-implemented today)": the command
  surface.
- Design doc, "Vault lifecycle": `clone` "does NOT grant you access".
- Design doc, "What \"diverged\" actually means".
- [Q-SYNC-CLASSIFY](../open-questions.md#q-sync-classify),
  [Q-REMOTE-TIMEOUT](../open-questions.md#q-remote-timeout),
  [Q-INIT-INTERIM](../open-questions.md#q-init-interim): the `--remote`
  half closes here.

## Go sources

- Go plan `gage-cli-design/m8a-sync-transport.md`, sections "Pull on
  unlock", "Push on write", "Divergence", `clone`, #38. It includes the
  implementation notes on try-lock and `RemoteOpTimeout`.
- Parity ledger rows:
  - §2 `386c57e` (#38)
  - §2 `ddce8b1`: the warning half
  - §3 M8a
  - §4 `clone`, `sync`, `pull`, `push`, and `init --remote`
- Go sources at the pinned SHA:
  - `internal/gage/remotesync.go`: `RemoteSyncer`, `SyncReport`,
    `ConflictKind`, `Pull`/`Push`/`Sync`, `underWriteLock`, `tryPull`,
    `fastForward`, `notePulled`, `syncOnUnlock`, `pushAfterWrite`,
    `Summary`
  - `internal/gage/unlock.go`: the `syncOnUnlock` call
  - `internal/gage/session.go`: `Use` and `onPull`
  - `cmd/gage/sync.go`
  - `cmd/gage/clone.go`, except `offerEnrollment`, which is P7's
  - `cmd/gage/init.go`: `--remote` and `ensureRemoteToken`
- Test inventory, 54 rows:
  - `internal/gage/remotesync_test.go` (18)
  - `internal/gage/sync_index_test.go` (3)
  - `cmd/gage/sync_cli_test.go` (16)
  - `cmd/gage/clone_test.go` (13)
  - `cmd/gage/init_remote_auth_test.go` (4)
- Help fixtures: `clone`, `sync`, `pull`, `push`, and `init`'s `--remote`.

## Decisions to make first

These were the recommended answers, confirmed when the P4 plan was
reviewed. Implementation settled the points the plan left open; they
follow under "Resolved in M8a2".

- **Where pull on unlock hooks in.** Inside `Vault::unlock`, as in Go,
  with no change to its signature. That is the one point both
  invocation modes pass through. The index invalidation stays where the
  tree moves, as Go's `notePulled` does:
  - `Vault` carries an optional pull observer, an
    `Arc<AtomicBool>` "tree moved" flag.
  - `Session` installs the flag on each `HeldVault` it opens.
  - `Session` checks the flag, and calls `discard_index` if it is set,
    before it serves anything from the index.
  - A fast-forward, a merge, and later M8b's resolution, all set the flag.
    Callers of those operations don't.

  The alternative was comparing HEAD before and after each call in
  `Session`. It was rejected because it would miss a pull made by a sync
  verb that runs outside an unlock.
- **`use` on a vault that is already unlocked** pulls explicitly, as Go's
  `Use` does (`TestUseFetchesAndFastForwards`). `use` means "catch me up".
- **`RemoteSyncer`** is a trait with `fetch(&self, deadline) -> Result<bool, SyncError>`
  and `push(&self, deadline) -> Result<bool, SyncError>`. The `bool` says
  whether anything moved.
  - The git-backed implementation is the default.
  - Tests inject one through `Vault::with_remote_syncer`, available under
    `cfg(test)` or the `test-support` feature.
  - Everything else (compare, fast-forward, merge) runs for real.
- **F2.** Go's `TestSyncReportsAPushEvenWhenNothingWasSent` pins a known
  bug. Go's `RemoteSyncer.Push` discards whether anything was sent, so
  `gage sync` on a vault that is already in sync claims to have pushed.
  The test is written to go red once the bug is fixed.
  - With `push` returning `bool`, the bug goes away by construction.
    Port the row with the assertion Go's comment anticipates, "already
    in sync", folded into `TestSyncLineSaysAlreadyInSyncWhenThereWasNothingToDo`
    as Go says to.
  - Record this as a deliberate deviation, `Q-SYNC-PUSHED`.
- **Locking:**
  - **Automatic pull on unlock:** `vaultlock::Lock::try_acquire`, a new
    additive call over `File::try_lock`. If the lock is busy, the pull is
    skipped silently.
  - **Automatic push after a write:** runs inside the closure passed to
    `with_write_lock`, as the unlocked form of push.
  - **Manual `sync`, `pull` and `push`:** wait on the lock with the
    existing timeout.
- **Deadline.** `REMOTE_OP_TIMEOUT` starts once per automatic operation,
  and once per manual verb.
- **The library's warnings.** These are push failures after a write, and
  failures or divergence during pull on unlock.
  - Go's wording, through `Prompter::warn`, naming gage once.
  - The push-failure wording is a constant, Go's "the write is committed
    locally but not pushed". M9 reuses it, as in Go.
- **Vault access for sync verbs.** They resolve their vault *without
  unlocking it*, by making Go's `vaultForSync` / `VaultWithoutUnlocking`
  public as `Session::vault_without_unlocking`. That is the existing
  `hold`. One-shot mode opens the registration directly.
- **`clone`.** One-shot only. It reuses the checks `init` makes on the
  vault name, path and registration. `--device` is ported, because it
  names this device in the registration. Go's enrollment offer is P7's.
- **`init --remote` publishes after `Vault::create`.** If the push fails,
  what happens? Port Go: the vault stays registered and committed
  locally, and the error explains why. Confirm against
  `TestInitAgainstAMissingRepositorySaysToCreateIt`. A "repository not
  found" from the auth bucket at init time says to create the
  repository.

### Resolved in M8a2

Points the plan left open, settled before the code that depends on them:

- **How a write reaches a human.** Go's writes warn through the
  prompter the identity was unlocked with (`id.warnTo()`). A Rust
  `Identity` holds no prompter, and `Vault::update` takes no identity.
  Each write method (`insert`, `remove`, `update`, `rename`) gains a
  trailing `prompter: &dyn Prompter`, the same advisory channel `unlock`
  already takes. A future GUI passes the same prompter it unlocked with.
  The P2 and P3 call sites gain that argument and nothing else (task 8).
- **`RemoteSyncer`'s error type** is `gitrepo::Error`. Its `Sync`
  variant carries `syncerr`'s buckets, so a fake builds an unreachable
  failure as `gitrepo::Error::Sync(syncerr::Error::new(..))` without
  touching git2. The git-backed implementation passes through `gitrepo`'s
  own errors (`RemoteAuth`, and `Git` for a failure in no bucket).
- **`SyncError` carries the report.** Go returns a report next to an
  error, and both `gage push` and `gage sync` print a report's conflicts
  on failure. `SyncError { report, cause }`, where `cause` is one of:
  - `Remote`: fetch or push failed
  - `Conflict`: a real conflict, rendered as the report's summary
  - `Local`: compare, fast-forward, merge or count failed. A dirty tree
    is the expected case, and is `Conflict` (1).
  - `Lock`: the write lock

  `SyncError::remote_kind()` answers Go's `errors.Is(err,
  syncerr.ErrDiverged)`: a `Conflict` cause is `Diverged` too.
- **No lock file for a vault with no remote.** Pull on unlock checks for
  a remote before it tries the lock. Without the check, every unlock of
  a local-only vault would create `$GAGE_STATE/locks/<id>.lock`, and a
  registration with an invalid id would warn on unlock before failing.
  `pull` re-checks under the lock, so the early check only avoids work.
- **The vault-name checks live in the library**, as `vault::name_from_url`
  and `vault::check_name`. Go keeps them in `cmd/gage`. A cloned vault's
  name can be inferred from the URL, so it is attacker-influenced input
  on its way to becoming a directory. Like `devicename`, the check is a
  security boundary, and a GUI's clone needs it as much as the CLI's.
- **`TestPluralEntries` moves to M8b.** Its only caller is `sync`'s
  "resolved N entries" line, which M8b's resolution produces. Porting it
  here would mean shipping an unused function.
- **At `init`, a local remote path that doesn't exist is a missing
  repository, not an outage.** Q-SYNC-CLASSIFY classifies a missing local
  path as unreachable, since for a sync it is most likely an unmounted
  drive. At `init` that would turn Go's
  `TestInitAgainstAMissingRepositorySaysToCreateIt` into "could not reach
  it, will publish later", exit 0. gage never creates a repository, so at
  init time a path that isn't there is one nobody has made yet. `init`
  reports it as Go does, with "create an empty one there, then run
  `gage push`" and `LockedOrAuth` (Go's bucket for it). Every other
  unreachable remote at init warns and exits 0, as in Go.
- **`clone`'s access message points at `gage identity add` and
  `gage recipient add`** (both P5). Go's other lines (`identity enroll`,
  `recovery enroll`) arrive with those commands in P7 and P8.

## Tasks

Write each task's tests first, at task granularity.

### 1. `RemoteSyncer`, `SyncReport` and the fake — 2 rows

- [x] The `RemoteSyncer` trait, its git-backed implementation, and the
      injection point.
- [x] `SyncReport`, with `ahead`, `pulled`, `pushed`, `merged`,
      `diverged`, `conflicts` and `conflict_kind`, and its summary
      text. Port Go's wording: "origin has diverged — N local commits
      pending, run `gage sync`".
- [x] Rows:
  - `TestTheInjectedOfflineErrorIsWhatVaultTreatsAsUnreachable`
  - `TestDivergenceIsReportedDifferentlyFromBeingOffline`

### 2. Pull on unlock — 5 + 3 rows

- [x] `vaultlock::Lock::try_acquire`, with a test in a child process
      showing that a held lock answers "busy" without waiting.
- [x] `Vault::unlock` runs the automatic pull. It warns once and carries
      on when the remote is unreachable, or on any other failure, and
      warns on divergence. A vault with no remote does nothing.
- [x] The pull observer, and `Session`'s index invalidation. `use` on a
      vault that is already unlocked pulls.
- [x] `remotesync` rows:
  - `TestUnlockFastForwardsFromTheRemote`
  - `TestUnlockPullsWithoutASessionCoversTheOneShotPath`
  - `TestUnlockWarnsOnceAndProceedsWhenTheRemoteIsUnreachable`
  - `TestPullThatBringsNothingReportsNoChange`
  - `TestUnlockDoesNotQueueBehindAConcurrentWrite`: a child process
    holds the lock, and the unlock must finish well within
    `WRITE_LOCK_TIMEOUT`.
- [x] `sync_index` rows:
  - `TestUseFetchesAndFastForwards`
  - `TestPullDuringASessionInvalidatesTheIndex`
  - `TestPullThatBringsNothingLeavesTheIndexAlone`

### 3. Push on write — 3 rows

- [x] Every write in `entry/crud.rs` (insert, update, remove, rename)
      pushes inside its own lock, after the commit. If the push fails,
      it warns and the write still succeeds. Rows:
  - `TestWritePushesToTheRemote`
  - `TestOfflinePushKeepsTheCommitAndRetriesLater`
  - `TestWriteDoesNotDeadlockOnItsOwnPush`
- [x] Beyond Go's rows: every write command pushes, checked through a
      table over insert, edit, rename, rm and generate. Go names only
      one.

### 4. `pull`, `push` and `sync` in the library — 8 rows

- [x] `Vault::pull`, `Vault::push` and `Vault::sync`, each under the
      waited lock:
  - **pull:** fetch, then fast-forward only. A divergence is reported,
    never merged.
  - **push:** a push that is rejected fetches, merges what doesn't
    conflict, and pushes again. A real conflict is reported.
  - **sync:** fetch, then fast-forward or merge, then push.

  A dirty working tree maps to Conflict.
- [x] Rows:
  - `TestDisjointDivergenceMergesAndPushes`
  - `TestConflictingDivergenceIsDetectedAndLeftAlone`
  - `TestRecipientFileDivergenceConflictsRatherThanUnioning`: the
    security row. Two devices each `git`-commit a different
    `.age-recipients` line.
  - The same for `.gage/config.toml`, as a sibling test. Go's plan
    lists it separately.
  - `TestInitWritesGitattributesForBothRecipientFiles`, asserted as
    content
  - `TestSyncTakesTheWriteLock`, with the lock held by a child process
  - `TestSyncOnAVaultWithNoRemoteDoesNothing`
  - `TestSyncReportsADirtyWorkTreeAsAConflictNotAnInternalError`
  - `TestAPendingMergeStillReportsWhatIsPending`: a merge whose push then
    fails reports 2, not 0.
- [x] Beyond Go's rows: a merge sets the pull observer, and a fast-forward
      inside `sync` sets it too.

### 5. `gage pull`, `push` and `sync` — 13 rows (1 moved to M8b)

- [x] Register `sync`, `pull` and `push`, each with `-u/--use`, in the
      sync group and available in both modes. Descriptions come from the
      fixtures. Each resolves its vault without unlocking it.
- [x] Exit codes: diverged is `Conflict` (1), offline is `Unreachable` (7),
      auth is `LockedOrAuth` (5) with the auth hint, and a dirty tree is
      `Conflict`.
- [x] Rows:
  - `TestPushPublishesAndPullCatchesUp`
  - `TestOneShotCommandPullsOnItsImplicitUnlock`
  - `TestWriteReachesTheRemoteWithoutAnyoneAskingIt`
  - `TestPullReportsADivergenceRatherThanMergingIt`
  - `TestSyncMergesADisjointDivergenceWithoutPrompting`
  - `TestSyncReportsAConflictingDivergenceAndNamesTheVault`
  - `TestSyncOnAVaultWithNoRemoteSaysSoAndSucceeds`
  - `TestOfflineExitsUnreachableAndDivergedExitsConflict`
  - `TestPushNamesTheConflictingPathsJustLikeSync`
  - `TestSyncRefusesOverUncommittedChanges`
  - `TestSyncLineSaysAlreadyInSyncWhenThereWasNothingToDo`
  - `TestSyncReportsAPushEvenWhenNothingWasSent`: ported with the
    assertion flipped, per the F2 decision (Q-SYNC-PUSHED).
  - ~~`TestPluralEntries`~~: moved to M8b (see "Resolved in M8a2").
  - `TestReportConflictsSaysRecipientFilesAreNeverMerged`
- [x] `p4_sync_commands_have_their_go_fixture_flags_and_descriptions`.
      Add the sync verbs and the push warnings to the `errorprefix`
      table. That closes ledger row `ddce8b1`'s `[~]`.

### 6. `gage clone` — 13 rows

- [x] `vault_name_from_url` infers a name from every URL spelling, and
      `check_vault_name` validates it. An inferred name always goes
      through the check.
- [x] `clone` with `--name`, `--dir` and `--device`:
  1. clone through `gitrepo::clone`
  2. read and check `.gage/config.toml` (format version, vault id)
  3. register the vault
  4. report "not a recipient yet, run `gage identity add`" when this
     device can't decrypt anything
- [x] Rows:
  - `TestCloneProducesARegisteredWorkingVault`
  - `TestCloneInfersTheNameFromTheURLAndNameOverridesIt`
  - `TestCloneWithoutDirLandsUnderGageData`
  - `TestCloneSaysSoWhenThisDeviceCannotDecryptAnything`
  - `TestCloneRefusesAnUnrecognizedFormatVersion`
  - `TestCurrentFormatVersionIsWhatCloneChecks`
  - `TestCloneRefusesAnAlreadyRegisteredName`
  - `TestCloneIsOneShotOnly`
  - `TestCloneRefusesANameThatEscapesTheVaultsDirectory`
  - `TestVaultNameFromURL`
  - `TestVaultNameFromURLHandlesEverySpelling`
  - `TestCheckVaultNameRefusesAnythingThatIsNotOnePathComponent`
  - `TestInferredNamesStillGoThroughCheckVaultName`
- [x] Beyond Go's rows: a clone that is refused leaves no registration
      and no half-made vault directory.

### 7. `init --remote` — 2 + 4 rows

- [x] Add `--remote` to `init`, with the fixture's help text. It sets
      `origin` through `CreateSpec.remote` and publishes with an
      immediate push. Rows:
  - `TestInitPublishesTheNewVault`
  - `TestInitAgainstAMissingRepositorySaysToCreateIt`
- [x] Inline token. The token is requested before the push, only when
      `token_host` names a host and no token is stored for it. The
      prompt is `Prompter::value`, the same prompt as `auth login`. A
      blank answer stores nothing and the push goes ahead anonymously.
      Rows:
  - `TestInitSolicitsATokenWhenNoneIsStored`: the token is checked at
    the recorder.
  - `TestInitSkipsTheTokenWhenTheAnswerIsBlank`
  - `TestInitDoesNotPromptWhenATokenAlreadyExists`
  - `TestInitDoesNotPromptForALocalRemote`
- [x] Close the `--remote` half of Q-INIT-INTERIM, and ledger §2
      `386c57e`.

### 8. Review, unmodified earlier tests, ledger

- [x] Review again, on a fresh read, with a mutation check on each:
  - try-lock on unlock
  - the push inside the write lock
  - the waited lock on manual sync
  - the observer that invalidates the index
- [x] P2 and P3 tests pass with no assertion changed. Every vault they
      use has no remote, so neither sync path runs. List any setup edit
      here with its reason.
- [x] Parity ledger:
  - §2 `386c57e` and `ddce8b1`
  - §4 `clone`, `sync`, `pull`, `push`
- [x] P4 index status column.

### Review record (task 8)

**Where the tests live.** Library rows are in `remotesync_tests.rs` (18
rows, plus the extra cases) and `sync_index_tests.rs` (3 rows, plus two).
`vault.rs` holds the four name rows. The CLI rows are in
`gage-cli/src/sync_tests.rs`. Each test names its Go row.

**Mutation check.** Each property below was broken on purpose, and at
least one test went red each time:

| Property | Mutation | Red |
|---|---|---|
| Try-lock on unlock | `try_pull` waits on the lock with `WRITE_LOCK_TIMEOUT` | `unlock_does_not_queue_behind_a_concurrent_write` |
| Push inside the write's lock | `push_after_write` calls the locked `push()` | `write_does_not_deadlock_on_its_own_push` |
| Waited lock on manual sync | `under_write_lock` runs without the lock | `sync_takes_the_write_lock` |
| Observer, raising side | a fast-forward doesn't raise the flag | `a_merge_and_a_fast_forward_inside_sync_raise_the_observer`, `pull_during_a_session_invalidates_the_index`, `a_sync_verb_needs_no_unlock_and_still_invalidates_the_index` |
| Observer, reading side | `settle` lowers the flag without discarding | `pull_during_a_session_invalidates_the_index`, `a_merge_inside_a_sessions_write_invalidates_the_index`, `a_sync_verb_needs_no_unlock_and_still_invalidates_the_index` |

**Found during review.** The CLI `Env` runs commands with a
`FakePrompter`, which records warnings rather than printing them. A
"no sync friction on stderr" assertion therefore passed vacuously. The
CLI tests check `FakePrompter::warnings()` instead, and
`offline_warnings_reach_the_terminal` drives the real `TerminalPrompter`
to prove the warnings reach stderr.

**Earlier tests.** No P2 or P3 test changed an assertion, except one P1
help test:

- `init_help_describes_this_milestone` pinned "no `--remote` yet". Its
  absence list loses `--remote`, which joins the present list, because
  this milestone adds the flag.

Setup edits, with no assertion changed:

- Every call to `insert`, `remove`, `update` and `rename` in the P2 and
  P3 tests gains the new `prompter` argument (`&FakePrompter::new()`).
  This affects `entry/crud.rs`, `index_tests.rs`, `session_tests.rs`,
  `search.rs` and `gage-cli/src/query_tests.rs`.
- `vault.rs`'s two struct-literal comparisons gain the new `syncer` and
  `pulled` fields.

Those vaults have no remote, so neither sync path runs.

**Order of work.** `remotesync.rs`'s library code went in as one module
before its tests went green. The tests-first convention was kept at the
CLI tasks and the name functions. The mutation check above is what
stands in for the library tasks' red-first step.

## Definition of done

Every box above is checked and all 54 claimed rows are ported (or struck
with a reason in this doc). The review in task 8 is recorded. `make lint`
and `make test` are clean locally and on Linux, macOS and Windows CI.

The milestone is done when all of the following hold between two
simulated devices:

- A vault syncs between the two devices.
- An unlock catches up without ever waiting behind a write.
- A write is published with no prompt.
- Being offline costs a warning, not a failure.
- A disjoint divergence merges and pushes without unlocking.
- A real conflict, or a recipient-file divergence, is reported with exit
  code 1 and left for `gage sync`.
- `clone` of a vault this device can't read says so plainly.
