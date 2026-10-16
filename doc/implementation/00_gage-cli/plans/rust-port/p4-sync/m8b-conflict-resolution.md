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
- Test inventory, 48 rows:
  - `internal/gage/syncresolve_test.go` (30)
  - `cmd/gage/syncresolve_cli_test.go` (8)
  - `internal/gage/gitrepo/merge_test.go` (10)

## Decisions to make first

These are the recommended answers. Confirm them when this plan is
reviewed. Go's four M8b decisions carry over unchanged:

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
- **Merge commit message:** `gage: resolve N entries`, with the singular
  and plural forms. Port Go's wording from `resolutionMessage`. It has no
  title and no plaintext.

## Tasks

Write each task's tests first, at task granularity.

### 1. The `gitrepo` merge seam — 10 rows

- [ ] Make public what M8a1 built internally:
  - `PendingMerge::conflicts`
  - `PendingMerge::content(side, path)`, which reads from the git
    objects and reports a side that isn't present
  - `commit(message, choices, adds)`, which refuses an unanswered
    conflict and refuses an add that would overwrite. Files it creates
    are 0600 (Unix).
  - rollback that removes the files it created, skips paths that aren't
    there, and always reaches the hard reset
- [ ] Rows:
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

- [ ] `EntryConflict`, `ConflictSide` and `Resolution`. Add
      `Vault::encrypt_entry`, the ciphertext without the write. Keep
      both re-encrypts under the vault's current recipients and a fresh
      UUID.
- [ ] Keep, both, skip and abort:
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
- [ ] Delete/modify:
  - `TestDeleteModifyConflictShowsTheSurvivingVersion`
  - `TestDeleteModifyConflictKeepingTheDeletion`
- [ ] (`TestKeepBothOnADeleteModifyConflictKeepsTheSurvivor` is in task 3.)

### 3. Delete/modify keep-both, file modes, history — 6 rows

- [ ] Rows:
  - `TestKeepBothOnADeleteModifyConflictKeepsTheSurvivor`
  - `TestEntryFilesAMergeCreatesAre0600`
  - `TestResolvedSyncProducesATwoParentMergeCommit`: parents [local,
    remote].
  - `TestLocalCommitGranularitySurvivesTheMerge`
  - `TestHistoryWalksThroughAMergeCommit`: structural, via a revwalk.
    `gitrepo::log` is P6's (see the [P4 plan](index.md#inventory-accounting)).
  - `TestMergeCommitMessageNamesTheCountAndNoPlaintext`

### 4. Unlocking, locking and the other milestones — 10 rows (1 excused)

- [ ] Lazy unlock, per the borrow decision. Rows:
  - `TestCleanFastForwardNeverUnlocks`
  - `TestDisjointDivergenceNeverUnlocks`
  - `TestUnlockHappensOnlyWhenAConflictIsReached`
  - `TestResolutionReusesASessionsCachedIdentity`
  - `TestANonEntryConflictIsRefusedWithoutUnlocking`
- [ ] `TestResolutionUpdatesTheSessionIndex`, through the pull observer.
- [ ] `TestRecipientConflictDoesNotReachTheEntryResolver`
- [ ] `TestResolutionHoldsTheWriteLockThroughout`: a child process trying
      to write blocks until resolution ends.
- [ ] `TestNonInteractivePrompterRefusesOnTheFirstConflict`
- [ ] ~~`TestNilPrompterRefusesRatherThanPanicking`~~: excused
      (Q-CONFLICT-PROMPTER). A `&dyn Prompter` can't be null. Strike it
      here once confirmed.
- [ ] The push after resolution starts a fresh `REMOTE_OP_TIMEOUT`
      deadline. Beyond Go's rows: a slow answer doesn't make the push
      fail. Prove it with an injected deadline, not real time.

### 5. The terminal menu and `gage sync` — 9 rows

- [ ] `TerminalPrompter::resolve_conflict` renders the design doc's
      `[l/r/b/s/q]` menu: the title, both sides' `updated` and
      `updated_by`, and a variant for delete/modify. It re-asks on an
      unknown answer, with no default. It refuses when it isn't
      interactive.
- [ ] `gage sync` runs `sync_resolving` with a lazy unlock (one-shot or
      session). An outcome left with skips ends with exit code `Conflict`
      and nothing pushed.
- [ ] Rows:
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

- [ ] Port Go's `doc/cli/sync.md` and `doc/cli/git-and-auth.md` to
      describe what P4 shipped:
  - the automatic pull and push
  - the verbs
  - divergence and resolution
  - `clone`
  - `auth` and tokens
  - SSH via ssh-agent and its limits
  - the remote-auth ground rules: no `~/.ssh/config`, no `insteadOf`,
    no credential helpers
- [ ] Extend `doc/cli/exit-codes.md` with `Unreachable` (7) and the sync
      meanings of 1 and 5. Extend `doc/cli/configuration.md` with the
      tokens directory.

### 7. Review, earlier tests, ledger, phase close

- [ ] Review again, on a fresh read, with a mutation check on each test
      that guards against losing a secret:
  - keep both on modify/modify and on delete/modify
  - skip applies nothing
  - abort is exact
  - rollback finishes
  - commit refuses an unanswered conflict
- [ ] M8a1 and M8a2 tests pass with no assertion changed. Any setup edit
      is listed here with its reason.
- [ ] Parity ledger:
  - §3 M8a and M8b marked `[x]`
  - §4 the P4 row
  - §6 `sync.md`, `git-and-auth.md`
- [ ] Mark P4 `[x]` in the [port plan](../index.md) and in this phase's
      index, after CI.

## Definition of done

Every box above is checked. All 48 claimed rows are ported, except the
one excused with its reason. The review in task 7 is recorded. `make
lint` and `make test` are clean locally and on Linux, macOS and Windows
CI.

The milestone is done when two simulated devices can edit the same
entry and `gage sync` resolves it three different ways, with no version
silently lost. Each resolution must leave a real two-parent merge commit
that can be walked. With no terminal, the same sync refuses and changes
nothing.
