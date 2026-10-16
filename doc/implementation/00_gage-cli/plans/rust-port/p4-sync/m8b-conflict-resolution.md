# M8b — Conflict resolution

[← M8a2](m8a2-sync-wiring.md) · [P4 plan](index.md) · [#35](https://github.com/distillerylabs/gage-rs/issues/35) · next: P5 Recipients & trust

> **Recommended model: Opus.** This milestone builds a real two-parent
> merge commit from chosen sides, and the resolution logic can lose a
> secret if it gets something wrong. That is problem-solving under a
> correctness constraint, not transcription. Review the no-secret-lost
> tests again at the end (task 7).

## Goal

`gage sync` resolves conflicts interactively. For each entry that both
sides changed, it:

1. decrypts both versions
2. shows who wrote each and when
3. applies the user's answer: **keep local**, **keep remote**, **keep
   both**, **skip** or **abort**

The result lands as a real merge commit. **Keep both** is the point of
the milestone. The losing version is re-encrypted under a fresh UUID,
with its original `created`, `updated` and `updated_by`, so resolving a
conflict never destroys a secret. M5's resolver already disambiguates two
entries with the same title.

Resolution unlocks lazily, only once a conflict actually needs
decrypting. It holds the write lock for the whole run. With nobody to ask
it refuses rather than choosing a side. This milestone also ports the
P4 user docs.

## Depends on

- **M8a2:**
  - `Vault::sync` and `SyncReport`
  - the pull observer that invalidates the index
  - `vault_without_unlocking`
  - the sync verbs
- **M8a1:**
  - `prepare_merge`, `PendingMerge::commit` and `merge_remote`, with the
    rollback
  - `reset_hard`
  - the `gittest` second clone
- **P2 · M3/M4:** entry encrypt and decrypt, and the write path that
  stamps nothing. Go's `WriteEntry`, like the Rust write path, leaves
  stamping to callers. Confirm that, and add Go's `encryptEntry`: the
  ciphertext without the write.
- **P3 · M5/M6:**
  - the ambiguous-query resolver
  - `Session`'s cached `Identity`
  - `TerminalPrompter`'s `interactive` flag

## Design references

- Design doc, "What \"diverged\" actually means": which of the five
  situations reach this milestone.
- Design doc, "Resolving an entry conflict":
  - the `[l/r/b/s/q]` prompt
  - why keep both exists
  - why the result is a merge commit
- Design doc, "Local trust cache". A recipient-file conflict goes to M10's
  confirmation, not to this menu.
- [Q-CONFLICT-PROMPTER](../open-questions.md#q-conflict-prompter),
  [Q-FILE-MERGE](../open-questions.md#q-file-merge),
  [Q-REMOTE-TIMEOUT](../open-questions.md#q-remote-timeout). The push
  after resolution starts a fresh deadline.

## Go sources

- Go plan `gage-cli-design/m8b-sync-conflicts.md`: all of it, including
  the four resolved decisions and the post-review implementation notes:
  - `encryptEntry`
  - files the merge creates are written 0600
  - no unlock for a conflict with nothing to decrypt
  - rollback finishes
  - a fresh push timeout
- Parity ledger rows:
  - §3 M8b
  - §4 `sync`, the resolving half
  - §6 `sync.md`, `git-and-auth.md`
- Go sources at the pinned SHA:
  - `internal/gage/syncresolve.go`
  - `internal/gage/gitrepo/merge.go`: `PendingMerge.Content`, choices,
    adds
  - the prompter's `ResolveConflict` in `cmd/gage`
  - `cmd/gage/sync.go`: `syncUnlocker`
- Test inventory, 49 rows:
  - `internal/gage/syncresolve_test.go` (30)
  - `cmd/gage/syncresolve_cli_test.go` (8)
  - `internal/gage/gitrepo/merge_test.go` (10)
  - `cmd/gage/sync_cli_test.go`: `TestPluralEntries` (1), moved from M8a2

## Decisions to make first

Confirmed when implementation started (all as recommended, with the
implementation notes below). Go's four M8b decisions carry over unchanged:

- keep both keeps the title verbatim
- keep both preserves provenance
- with nobody to ask, it refuses, and `--yes` keeps only its M10 meaning
- the lock is held throughout

What follows is specific to Rust.

- **The prompter's shape** (Q-CONFLICT-PROMPTER). Add
  `Prompter::resolve_conflict(&EntryConflict) -> Result<Resolution, PromptError>`.
  - Its default returns `PromptError::NotInteractive`.
  - `FakePrompter` gets a `with_resolutions` builder.
  - `TerminalPrompter` overrides it, and still refuses when it isn't
    interactive.
  - `TestNilPrompterRefusesRatherThanPanicking` is excused: no
    `&dyn Prompter` can be null. The CLI's `--script` and `--stdin` reach
    the same refusal through the non-interactive terminal path.
- **Lazy unlock and the borrow.** Go passes `Unlock func() (*Identity, error)`.
  - **One-shot:** `Vault::sync_resolving(&self, prompter, unlock)` takes
    `unlock: &mut dyn FnMut() -> Result<Identity, UnlockError>`. The
    identity is created on the spot and owned for the run.
  - **Session:** a `Session::sync_resolving(name)` method. Inside it, the
    session's cached `&Identity` and the vault are borrowed together,
    which works because both are immutable borrows of one `HeldVault`. If
    the vault is locked, it unlocks once through the session's own path.

  This keeps "called at most once, and only when a conflict is reached".
  `Identity` stays a parameter and is never stored on `Vault`.
- **Every path is checked before the unlock.** Map each conflicting path
  to an entry id first. A recipient file or `.gage/config.toml` goes to
  M10's route, which for now is a refusal that names the trust cache. A
  file that is neither is refused, naming the path. Only then does the
  unlock run.
- **The typed values:**
  - `EntryConflict { id, path, local: ConflictSide, remote: ConflictSide }`
  - `ConflictSide { present, entry: Option<Entry> }`: `Option` replaces
    Go's `Present` flag plus an empty entry.
  - `Resolution { KeepLocal, KeepRemote, KeepBoth, Skip, Abort }`

  Decrypted entries hold their `Zeroizing` secret fields, as `Entry`
  already does.
- **Merge commit message:** Go's wording from `resolutionMessage`,
  `gage: merge origin, resolving 1 entry` / `… 2 entries`. (An earlier
  draft of this line said `gage: resolve N entries`; Go's wording wins.)
  It has no title and no plaintext.

### Implementation notes

What implementing the decisions above settled, for whoever reads this
next:

- **Where the non-interactive refusal happens.** A default trait method
  can't be asked about before it is called, so the refusal comes at the
  first `resolve_conflict`, after the lazy unlock. That is where Go's
  real CLI refuses too: `terminalPrompter` always satisfies
  `ConflictPrompter`, and checks its `interactive` flag inside
  `ResolveConflict`. Go's earlier structural check only spares a
  prompter that doesn't implement the interface at all. The paths are
  still checked before the unlock.
- **`TerminalPrompter`'s `interactive` flag** didn't exist yet (the
  "Depends on" list above assumed P3 had added it). M8b adds it:
  `TerminalPrompter::interactive(bool)`, set by `main` from whether stdin
  is a terminal, as Go's `run.go` sets it.
- **`FakePrompter`** with no resolution scripted refuses like the trait
  default, so it stands in for a frontend with nobody to ask. Scripted
  answers come `with_resolutions` (in order) or `with_resolution_for`
  (by title, since conflicts arrive in UUID order).
- **The borrow.** `Vault::sync_resolving` takes the owned-identity
  closure. It adapts it to the crate-internal `sync_resolving_with`, whose
  closure lends `&Identity`, through a `OnceCell` that owns the identity
  for the run. `Session::sync_resolving` uses the same form. It lends the
  cached identity or unlocks once into a `OnceCell`, and keeps that key
  afterwards like any other unlock. It returns
  `Result<Result<SyncReport, SyncError>, SessionError>`: the outer error is
  the vault lookup.
- **A recipient-file conflict** is returned from `sync_resolving` exactly
  as M8a2's push refuses it, which is what Go does at the pin. The
  message isn't a new one naming the trust cache, since P4 has no trust
  cache to name. Follow-up for P5 · M10: that summary ends "run
  `gage sync`", which until M10 just refuses again.
- **`Entry` holds plain `String`s**, not `Zeroizing` ones (the decision
  above assumed otherwise). `EntryConflict` carries `Entry` as every
  other decrypted read does. Changing that is not this milestone's.
- **`SyncReport` grew `resolved`, `skipped` (both `u32`) and `aborted`**,
  and `SyncCause` grew `Prompt`, `Unlock`, `NotAnEntry`, `Entry`,
  `Unresolved` and `Aborted`. The counts are `u32`, and the unlock and
  entry causes are boxed, to keep `SyncError` under clippy's
  `result_large_err` threshold. A skipped or aborted resolution still
  reports `Kind::Diverged`, as Go's `resolutionRefusal` unwraps to
  `ErrDiverged`.
- **`PendingMerge::commit`** also refuses an add at a path the merge itself
  takes from the remote, not only one HEAD has (Go checks only HEAD's
  files). This doesn't matter for a fresh UUID, but it closes the case.

## Tasks

Write each task's tests first, at task granularity.

### 1. The `gitrepo` merge seam — 10 rows

- [x] Make public what M8a1 built internally:
  - `PendingMerge::conflicts`
  - `PendingMerge::content(side, path)`, which reads from the git
    objects and reports a side that isn't present
  - `commit(message, choices, adds)`, which refuses an unanswered
    conflict and refuses an add that would overwrite. Files it creates
    are 0600 (Unix).
  - rollback that removes the files it created, skips paths that aren't
    there, and always reaches the hard reset
- [x] Rows:
  - `TestPrepareMergeDecidesWithoutWriting`
  - `TestPendingMergeContentReadsBothSidesFromTheObjects`
  - `TestPendingMergeContentReportsAnAbsentSide`
  - `TestCommitRefusesAnUnansweredConflict`
  - `TestCommitAppliesEachChoice`
  - `TestCommitCreatesTheAddsItIsGiven`
  - `TestCommitRefusesAnAddThatWouldOverwrite`
  - `TestMergeCreatedFilesAre0600`
  - `TestCommitRollsBackAPartialFailure`: an add placed under a path
    that is a file.
  - `TestMergeRemoteAndPrepareMergeCannotDrift`

### 2. Resolution mechanics — 14 rows

- [x] `EntryConflict`, `ConflictSide` and `Resolution`. Add
      `Vault::encrypt_entry`, the ciphertext without the write. Keep
      both re-encrypts under the vault's current recipients and a fresh
      UUID.
- [x] Keep, both, skip and abort:
  - `TestConflictIsPresentedWithBothSidesDecrypted`
  - `TestKeepLocalKeepsTheLocalVersion`
  - `TestKeepRemoteReplacesTheLocalVersion`
  - `TestKeepBothKeepsBothVersions`
  - `TestKeepBothPreservesTheLosingSidesProvenance`
  - `TestKeepBothLeavesTwoEntriesTheResolverDisambiguates`
  - `TestSkipLeavesTheEntryConflictedAndDoesNotPush`
  - `TestSkipStillPresentsTheRemainingConflicts`
  - `TestOneSkipAppliesNoneOfTheOtherAnswers`
  - `TestTwoConflictsTakeTheirOwnAnswers`
  - `TestAPrompterErrorLeavesTheVaultUntouched`
  - `TestAbortRestoresThePreSyncState`: HEAD, the working tree and
    cleanliness are unchanged, and nothing is pushed.
- [x] Delete/modify:
  - `TestDeleteModifyConflictShowsTheSurvivingVersion`
  - `TestDeleteModifyConflictKeepingTheDeletion`
- [x] (`TestKeepBothOnADeleteModifyConflictKeepsTheSurvivor` is in task 3.)

### 3. Delete/modify keep-both, file modes, history — 6 rows

- [x] Rows:
  - `TestKeepBothOnADeleteModifyConflictKeepsTheSurvivor`
  - `TestEntryFilesAMergeCreatesAre0600`
  - `TestResolvedSyncProducesATwoParentMergeCommit`: parents [local,
    remote].
  - `TestLocalCommitGranularitySurvivesTheMerge`
  - `TestHistoryWalksThroughAMergeCommit`: structural, via a revwalk.
    `gitrepo::log` is P6's (see the [P4 plan](index.md#inventory-accounting)).
  - `TestMergeCommitMessageNamesTheCountAndNoPlaintext`

### 4. Unlocking, locking and the other milestones — 10 rows (1 excused)

- [x] Lazy unlock, per the borrow decision. Rows:
  - `TestCleanFastForwardNeverUnlocks`
  - `TestDisjointDivergenceNeverUnlocks`
  - `TestUnlockHappensOnlyWhenAConflictIsReached`
  - `TestResolutionReusesASessionsCachedIdentity`
  - `TestANonEntryConflictIsRefusedWithoutUnlocking`
- [x] `TestResolutionUpdatesTheSessionIndex`, through the pull observer.
- [x] `TestRecipientConflictDoesNotReachTheEntryResolver`
- [x] `TestResolutionHoldsTheWriteLockThroughout`: a child process probes
      the lock while a conflict waits for its answer (busy) and after
      (free).
- [x] `TestNonInteractivePrompterRefusesOnTheFirstConflict`
- [x] ~~`TestNilPrompterRefusesRatherThanPanicking`~~: excused
      (Q-CONFLICT-PROMPTER). A `&dyn Prompter` can't be null.
- [x] The push after resolution starts a fresh `REMOTE_OP_TIMEOUT`
      deadline. Beyond Go's rows: a slow answer doesn't make the push
      fail. Proved with the deadline a recording `RemoteSyncer` is handed,
      which must be at least `REMOTE_OP_TIMEOUT` past the moment the
      answer was given (`the_push_after_resolution_starts_a_fresh_deadline`).

### 5. The terminal menu and `gage sync` — 9 rows

- [x] `TerminalPrompter::resolve_conflict` renders the design doc's
      `[l/r/b/s/q]` menu: the title, both sides' `updated` and
      `updated_by`, and a variant for delete/modify. It re-asks on an
      unknown answer, with no default. It refuses when it isn't
      interactive.
- [x] `gage sync` runs `sync_resolving` with a lazy unlock (one-shot or
      session). An outcome left with skips ends with exit code `Conflict`
      and nothing pushed.
- [x] Rows:
  - `TestConflictPromptRendersTheDesignDocsMenu`
  - `TestConflictPromptMapsEveryKey`
  - `TestConflictPromptReAsksOnAnUnknownAnswer`
  - `TestDeleteModifyPromptSaysOneSideDeletedIt`
  - `TestNonInteractivePrompterRefusesToResolve`
  - `TestSyncResolvesAConflictInteractivelyAndPushes`
  - `TestSyncWithoutATerminalRefusesRatherThanChoosing`
  - `TestYesDoesNotResolveConflicts`: as in Go before M10, `sync --yes`
    is an unknown flag. Pin that, and leave a note for M10 to flip it.
  - `TestPluralEntries` (`sync_cli_test.go`), moved from M8a2: its only
    caller is the "resolved N entries" line this task adds. Go's
    `pluralEntries` spells "1 entry", not "1 entries".

### 6. User docs

- [x] Port Go's `doc/cli/sync.md` and `doc/cli/git-and-auth.md` to
      describe what P4 shipped:
  - the automatic pull and push
  - the verbs
  - divergence and resolution
  - `clone`
  - `auth` and tokens
  - SSH via ssh-agent and its limits
  - the remote-auth ground rules: no `~/.ssh/config`, no `insteadOf`,
    no credential helpers
- [x] Extend `doc/cli/exit-codes.md` with `Unreachable` (7) and the sync
      meanings of 1 and 5. Extend `doc/cli/configuration.md` with the
      tokens directory.

### 7. Review, earlier tests, ledger, phase close

- [x] Review again, on a fresh read, with a mutation check on each test
      that guards against losing a secret. Each mutation was applied by
      hand, the guarding test run, and the source restored:

  | Guard | Mutation | Test | Result |
  |---|---|---|---|
  | keep both, modify/modify | drop keep-both's add | `keep_both_keeps_both_versions` | killed |
  | keep both, modify/modify | keep both answers `Remote` | `keep_both_keeps_both_versions` | killed |
  | keep both, delete/modify | drop keep-both's add | `keep_both_on_a_delete_modify_conflict_keeps_the_survivor` | killed |
  | skip applies nothing | skip no longer refuses | `one_skip_applies_none_of_the_other_answers` | killed |
  | skip applies nothing | skip also records `Local` | `one_skip_applies_none_of_the_other_answers` | survived: equivalent, since the skip refusal still fires before anything is applied |
  | abort is exact | abort breaks out and goes on to apply | `abort_restores_the_pre_sync_state` | killed |
  | rollback finishes | skip the hard reset | `commit_rolls_back_a_partial_failure`, `a_partial_merge_failure_rolls_back` | killed |
  | rollback finishes | a removal error abandons the rollback | `commit_rolls_back_a_partial_failure` | killed |
  | unanswered conflict | `commit` stops checking choices | `commit_refuses_an_unanswered_conflict` | killed |

- [x] M8a1 and M8a2 tests pass with no assertion changed. Setup edits:
  - `gitrepo::merge_tests::a_pending_merge_with_conflicts_refuses_to_commit`
    (M8a1): `commit` now takes `choices` and `adds`, so the call passes two
    empty maps.
  - `repl_tests`' session harness (`Env::session_with_clock`): its
    `TerminalPrompter` is built `.interactive(true)`, as `main` builds one
    on a terminal, which the harness says it is. Only `resolve_conflict`
    reads the flag, and no earlier test reaches it.
  - `sync_tests::sync_reports_a_conflicting_divergence_and_names_the_vault`
    (M8a2) is unchanged, but now reaches its refusal through
    `sync_resolving`: its `shared.txt` is not an entry, so it is refused
    before any unlock and still names the path and the vault.
- [ ] Parity ledger:
  - §3 M8a and M8b marked `[x]`
  - §4 the P4 row
  - §6 `sync.md`, `git-and-auth.md`

  §4 and §6 are updated. §3 is marked `[x]` with the phase after CI, as
  M8a1 and M8a2 were.
- [ ] Mark P4 `[x]` in the [port plan](../index.md) and in this phase's
      index, after CI.

## Definition of done

Every box above is checked. All 49 claimed rows are ported, except the
one excused with its reason. The review in task 7 is recorded. `make
lint` and `make test` are clean locally and on Linux, macOS and Windows
CI.

The milestone is done when two simulated devices can edit the same
entry and `gage sync` resolves it three different ways, with no version
silently lost. Each resolution must leave a real two-parent merge commit
that can be walked. With no terminal, the same sync refuses and changes
nothing.
