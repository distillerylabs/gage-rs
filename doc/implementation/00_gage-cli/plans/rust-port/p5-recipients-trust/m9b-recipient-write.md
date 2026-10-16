# M9b — Recipient writes & re-encryption

[← M9a](m9a-identity-recipient-read.md) · [P5 plan](index.md) · [#43](https://github.com/distillerylabs/gage-rs/issues/43) · next: [M10](m10-trust-cache.md)

> **Recommended model: Opus.** This milestone is about crash-safety. The
> re-encryption must leave HEAD untouched when it's interrupted, and the
> dirty-tree reset must run under the lock on every write. The tests
> simulate crashes, so they prove only the windows they reach. Task 3
> names every window and the test that reaches it. Review that table
> again at the end.

## Goal

Access can be granted and revoked, and neither can be half-done:

- `gage recipient add <pubkey> --device NAME` always re-encrypts every
  entry to the new list (A19/E1a). There is no `--reencrypt` flag, and
  passing one is a usage error. An actor that can't read every entry is
  refused with `CannotGrantFullAccess`. The refusal comes before the
  vault lock and before any confirmation, and it names how many entries
  are unreadable.
- `gage recipient remove <pubkey-or-name> --reencrypt` re-encrypts
  without the removed key and prints the "revokes future access only"
  warning. Without `--reencrypt` it is rejected before the unlock prompt.
  Removing the last recipient is refused outright. Removing this device's
  own key asks first.
- The recipient pair (`.age-recipients` and `config.toml`) and every
  re-encrypted entry land in **exactly one commit**, or none of it does.
- Every write (`insert`, `edit`, `generate`, `rename`, `rm`, and the
  recipient verbs) starts under the lock by resetting an unexpectedly
  dirty working tree to HEAD. It warns first, naming the discarded paths.

## Depends on

- **M9a:**
  - `recipients`, `verify_recipients`, `add_identity`
  - the `recipient` parent
  - the two-device fixture
- **P2 · M4:**
  - `with_write_lock` (`entry/crud.rs`)
  - the write path (`write_entry`, `commit_all`)
  - `read_entry`, `entry_ids`
- **P4 · M8a1:** `gitrepo::reset_hard`, which returns the discarded paths
  and already removes untracked files and changes outside `entries/`.
- **P4 · M8a2:**
  - `push_after_write` and `UNPUSHED_WRITE_CLAUSE`
  - `with_remote_syncer` (to check the unreachable-push wording)

## Design references

- Design doc, "Recipient / access management".
- Design doc, "`--reencrypt` is all-or-nothing, on purpose".
- Design doc, "Why adding a recipient always re-encrypts", and Go's A19:
  partial access is contagious and can't be repaired.
- Design doc, "Concurrent processes and the vault lock": the lock is held
  for the whole run, and contention reports rather than interleaves.

## Go sources

- Go plan `gage-cli-design/m9-recipients.md`:
  - the Recipients, `--reencrypt` atomicity and Dirty working tree
    bullets
  - the four resolved decisions (hold the lock throughout; `add` commits
    both files; push is the ordinary post-write push; the last recipient
    is refused and self-removal confirmed)
  - all of "Settled during implementation"
- Go plan `gage-cli-init-design/e1a-unconditional-reencrypt.md`: all of
  it, including the clean-tree gate on the pre-flight and the
  `--reencrypt` grep.
- Parity ledger rows:
  - §2 `3fd784c`/`fbc887f` (E1a, #46)
  - §3 M9 (with M9a)
  - §4 `recipient_add`, `recipient_remove`
- Go sources at the pinned SHA:
  - `internal/gage/recipient.go`: `AddRecipient`, `RemoveRecipient`,
    `confirmSelfRemoval`, `commitRecipientList`, `RequireFullAccess`,
    `reencryptTo`, `writeRecipientFiles`. Leave out E1b's
    `addRecipientsLocked`/`recipientWrite` and E4's
    `prunePendingEnrollments`.
  - `internal/gage/vault.go`: `withVaultWrite`, `resetDirtyWorkTree`.
    `withEncryptingWrite` is M10's.
  - `cmd/gage/recipient.go`: `add`, `remove`, `recipientChangeLines`,
    `shortHash`
- Test inventory, 37 rows:
  - `internal/gage/recipient_test.go` (17 of 21)
  - `internal/gage/reencrypt_test.go` (9)
  - `internal/gage/identity_add_test.go` (1 of 7)
  - `cmd/gage/recipient_cli_test.go` (10 of 15)

## Decisions to make first

Settled against the code at the start of M9b. Each item is marked
**as proposed** (kept from planning), **revised** (planning's proposal
changed, with the reason), or **new** (planning didn't raise it).

- **The library surface** (as proposed, with types filled in). In a new
  `recipient` write half next to M9a's read half:
  - `Vault::add_recipient(&self, device: &str, pubkey: &str, identity: &Identity, prompter: &dyn Prompter) -> Result<RecipientChange, RecipientError>`
  - `Vault::remove_recipient(&self, query: &str, reencrypt: bool, identity, prompter) -> Result<RecipientChange, RecipientError>`
  - `Vault::require_full_access(&self, identity) -> Result<(), RecipientError>`,
    public and callable on its own (E4 places it differently). It counts
    only `crypt::Error::NotARecipient`. A corrupt entry is a different
    problem, and the re-encryption pass names it.
  - `RecipientChange { device, pubkey, commit: String, reencrypted: usize }`
  - Lists are M9a's `vaultconfig::Recipient { device, pubkey }`. There is
    no second `VaultRecipient` type.
  - `RecipientError` variants and codes, mirroring Go:
    - `InvalidDeviceName` and `InvalidRecipient(agekey::Error)`: Usage
    - `RecipientExists`, one variant whose message says whether the key
      or the device name is already listed: Conflict
    - `RecipientNotFound { query }`: NotFound
    - `ReencryptRequired`: Usage
    - `LastRecipient { device }`: Conflict
    - `CannotGrantFullAccess { unreadable, total, device, vault }`:
      Conflict. The message gives the count and never an id.
    - `SelfRemovalDeclined { device }`: Conflict
    - `Confirm(PromptError)`: LockedOrAuth, as Go wraps a failed
      `Confirm`
    - `Reencrypt { id, source: entry::Error }`: the source's code, and
      the message names the entry
    - wrapped `Entry`, `Config`, `Recipients`, `Path`, `Lock`, `Git` and
      `Io` errors, each keeping its own code
- **The shape E1b widens** (new). The index says M9b builds the
  single-recipient write so that E1b widens it rather than rewriting it.
  Concretely, `add_recipient` is the pre-flight plus `with_vault_write`
  around a private `add_recipient_locked(&Recipient, message, …)`, and
  both verbs end in a shared private
  `commit_recipient_list(updated, message, identity, prompter)`. That
  function is Go's `commitRecipientList`: re-encrypt, write the
  recipient files, `commit_all`, `push_after_write`. E1b turns
  `&Recipient` into a slice and adds the delete paths and push clause.
  M10's checks slot in as the first statements inside the lock, where
  Go has them. M9b does not build Go's `recipientWrite` struct, the
  delete paths, or the custom push clause: they have no caller yet.
- **The prompter is a parameter** (as proposed). Go asks through
  `ident.frontend()`. The Rust `Identity` carries no prompter, so
  `prompter` is passed explicitly, as every write already does
  ([Q-RECIPIENT-PROMPT](../open-questions.md#q-recipient-prompt)). The
  self-removal question is the first production caller of
  `Prompter::confirm`. Its default answer is no, so a non-interactive
  prompter aborts with nothing written. Self-removal is detected as Go
  does: the removed key is `identity.recipient()`, or the removed device
  is `identity.device()`.
- **Encrypt to an explicit list** (as proposed). Add
  `pub(crate) fn write_entry_to(id, &Entry, &[crypt::Recipient])` beside
  `write_entry`, and make `write_entry` call it with the on-disk list.
  It is crate-private because nothing outside needs it. The
  re-encryption follows Go's order:
  1. decrypt every entry into memory first (a failure there writes
     nothing)
  2. parse the new list into `crypt::Recipient`s (a key this build can't
     encrypt to fails here, still before any write)
  3. write each entry to the new list, calling the crash seam after each
  4. write the recipient pair last, `.age-recipients` then `config.toml`
  5. one `commit_all`, then `push_after_write`

  This keeps the crash window's dirty surface to `entries/` for as long
  as possible. The decrypted entries are held as `Entry` values, as
  `list` already holds them. Zeroizing `Entry` is not in M9b's scope; Go
  doesn't do it either.
- **`with_vault_write`** (as proposed; the open point is settled).
  `with_vault_write(timeout, prompter, f)` is `with_write_lock`, then
  `gitrepo::reset_hard`, then `f`. When the reset discarded anything, it
  warns once through `Prompter::warn` on one line, naming every path:
  `gage: discarded uncommitted changes left by an interrupted write: <paths>`.
  `insert`, `remove`, `update` and `rename` move onto it, and so do both
  recipient verbs. `generate` and `edit` get it through `insert` and
  `update`. The sync paths (`pull`/`push`/`sync`, `sync_resolving`)
  keep the bare lock and keep *refusing* a dirty tree, as in Go.
  `update` already takes a prompter (P4 added it for the push warning),
  so nothing needs adding.
- **A reset tells the session's index** (new, Rust-specific). The index
  caches what `entries/` held. A reset that discarded something moved
  the working tree just as a pull does, so it raises the same
  `PullObserver`, and a session re-reads rather than serving metadata
  for files that are gone. Go's index happens not to need this. Ours
  is keyed off the observer, so it costs one line.
- **The crash seam** (revised:
  [Q-CRASH-SEAM](../open-questions.md#q-crash-seam) is amended). The seam
  is a `ReencryptHook` field on `Vault`. Following `DecryptCounter`'s
  pattern, it is zero-sized outside `cfg(test)`, so no constructor needs
  a `cfg`. In tests it holds an
  `Option<Arc<dyn Fn(usize) + Send + Sync>>`, which is called after each
  entry is written with the count so far. It compares equal always, as
  the other seams do, so `Vault`'s derives survive.

  **A test *panics* from the hook to stand in for the crash, rather than
  returning an error** (planning said the error). Go panics, and its
  reason applies unchanged here. A returned error runs the
  error-handling path, so any cleanup-on-failure added later would make
  every crash test prove that cleanup works rather than that an
  interruption is invisible. Unwinding runs only `Drop`s: the lock's
  `Drop` releases it, as the OS does for a killed process, and files on
  disk stay as they were. The test catches the panic with
  `catch_unwind(AssertUnwindSafe(..))` and re-raises any other panic, as
  Go's `runAndRecoverCrash` does. Nothing in the library may catch an
  unwind. The hook doesn't return a value, so the lock-observation test
  uses the same hook type.

  The recipient-files window is reached through the real
  `write_recipient_files`, called directly from a test (Go's review
  fix), not by hand-writing two lookalike files.
- **Lock tests use a child process** (new). The project rule is that
  lock properties are proven across processes. Two tests need another
  holder:
  - `TestFullAccessPreflightRunsBeforeTheVaultLock`: a child holds the
    lock for the whole call, and the test asserts the refusal arrives
    in well under the 10 s lock timeout.
  - `TestReencryptHoldsTheVaultLockThroughout`: the hook spawns a child
    that tries the lock once and reports `HELD` or `FREE`.

  Both use a new path-based child helper in a shared `#[cfg(test)]`
  module. `crud.rs`'s vault-shaped trampoline is left alone.
- **An M4 assertion changes** (new). `crud.rs`'s
  `an_insert_whose_commit_fails_leaves_the_entry_uncommitted` (not a Go
  row) pins that the *next* write's commit picks up an orphaned entry
  file. That is exactly the folding-in M9 removes: Go's post-M9 behavior
  is that the next write discards it, with the warning. The test is
  rewritten to that, keeping its name's first half: the failed insert
  leaves the file uncommitted, and the next write resets it and warns.
  Any other existing test that writes an entry without committing and
  then calls a write verb gets a commit in its setup, with a one-line
  comment. This is not an assertion change. Nothing in M8a1/M8a2's sync
  tests changes.
- **Commit messages** (as proposed): `gage: recipient add <device> (reencrypt)`
  and `gage: recipient remove <device> (reencrypt)`. The device name is
  already plaintext in the commit's own diff.
- **The CLI rejects `remove` without `--reencrypt` before the unlock**
  (as proposed), as Go does, so nobody is asked for a passphrase and then
  told the command line was wrong. The library check is the contract; the
  CLI check only orders the failure.
- **The CLI test prompter echoes warnings to stderr** (settled: it
  doesn't today). gage-cli's tests pass the library's `FakePrompter`,
  which records warnings and writes nothing. The harness wraps it in an
  `EchoingPrompter` that forwards every call, and writes each warning
  into the same shared buffer `Io.stderr` writes to, so ordering is
  kept. Existing assertions stay green, and any `stderr.is_empty()` check
  that now sees a real warning is reported in this doc, not silently
  loosened.
- **No recovery-label check** (interim, as for `identity add`): Go's
  `checkDeviceLabelFree` reserves the recovery key's device name. It
  arrives with P8, which adds it to `add_recipient_locked`.

## Tasks

Write each task's tests first, at task granularity. Library rows live in
`crates/gage/src/recipient_tests.rs` (tasks 2 and 4) and
`reencrypt_tests.rs` (tasks 1 and 3), task 6 in `twodevice_tests.rs`, and
the CLI rows in `crates/gage-cli/src/recipient_cli_tests.rs`. Each test
names its Go row.

Two changes from the plan, made during implementation:

- `TestAddIsNotRefusedOverATreeTheResetDiscards` moved from task 2 to
  task 4. It builds its state from a real interrupted `recipient remove`
  of this device, which doesn't exist until task 4. Building the state by
  hand instead is what Go's review warned against. Task 2 now has 9 rows
  and task 4 has 8; the total is still 37.
- The child-process lock helper is `remotesync_tests`'s existing
  path-based `Holder`, made `pub(crate)`, plus a one-shot probe mode
  (`lock_is_held_elsewhere`). Planning proposed a new module for this; it
  would have been a third copy of the same trampoline.

The crash tests were checked against mutations, so they are known to
fail when the property breaks:

- Writing the recipient files *before* the re-encryption fails
  `interrupted_reencrypt_leaves_head_untouched` and
  `reencrypt_aborts_whole_operation_when_one_entry_fails`.
- A reset that only acts when `entries/` is dirty fails
  `interrupted_recipient_write_leaves_no_half_migrated_state`.

### 1. The dirty-tree reset on every write — 2 rows

- [x] `with_vault_write` (`entry/crud.rs`), with `insert`, `remove`,
      `update` and `rename` moved onto it. `generate` and `edit` reach it
      through `insert` and `update`.
- [x] `TestDirtyWorkTreeIsResetBeforeAnySubsequentWrite` →
      `dirty_work_tree_is_reset_before_any_subsequent_write`: `insert`,
      `edit`, `rename` and (as in Go) `remove` each discard a leftover
      rather than folding it into their commit.
- [x] `TestDirtyWorkTreeResetWarnsNamingTheDiscardedPaths` →
      `dirty_work_tree_reset_warns_naming_the_discarded_paths`: one
      warning on one line, through the prompter, naming a modified entry
      and an untracked orphan; a clean tree stays silent.
- [x] Not a Go row: `a_reset_that_discards_something_tells_the_pull_observer`
      (the session-index decision).
- [x] The sync paths still refuse a dirty tree. They keep the bare lock,
      and every M8a1/M8a2 test passes unchanged. The M4 test
      `an_insert_whose_commit_fails_leaves_the_entry_uncommitted` is
      rewritten as decided: the next write discards the orphan and warns.
      Two test setups that left an entry uncommitted before a write now
      commit it: `rename_changes_only_title_and_bumps_updated` and
      gage-cli's `query_tests::write_with_id`.

### 2. `require_full_access` and `add_recipient` — 9 rows

- [x] `write_entry_to` and `reencrypt_to`. The ordering is described in
      "Decisions to make first".
- [x] `require_full_access`, gated on a clean tree before the lock. On a
      dirty tree it runs just after the reset, inside the lock, still
      before any write.
- [x] Rows:
  - `TestRequireFullAccessIsCallableOnItsOwn` →
    `require_full_access_is_callable_on_its_own`, plus (not a Go row)
    `require_full_access_does_not_count_a_corrupt_entry`
  - `TestRecipientAddRefusesAnActorThatCannotReadEveryEntry` →
    `recipient_add_refuses_an_actor_that_cannot_read_every_entry`: the
    state is built through the real `write_recipient_files` and committed,
    as Go's `hideOneEntryFrom` does. The error gives "1 of 3", is
    Conflict, names no id, the hook saw no rewrite, and nothing changed.
  - `TestFullAccessPreflightRunsBeforeTheVaultLock` →
    `full_access_preflight_runs_before_the_vault_lock`: a child holds
    the lock, and the refusal arrives in under 5 s against a 10 s
    timeout.
  - `TestAddIsStillRefusedOnADirtyTreeWhenAccessIsGenuinelyPartial` →
    `add_is_still_refused_on_a_dirty_tree_when_access_is_genuinely_partial`
  - `TestAddRecipientRejectsInvalidDeviceNameOrPubkey` →
    `add_recipient_rejects_invalid_device_name_or_pubkey`
  - `TestRecipientAddRejectsADuplicateKey` →
    `recipient_add_rejects_a_duplicate_key` (the key, and the device
    name)
  - `TestRecipientAddCommitsBothFilesTogether` →
    `recipient_add_commits_both_files_together`: the commit's paths are
    exactly both files plus every entry, and its message is
    `gage: recipient add phone-1 (reencrypt)`.
  - `TestRecipientAddHasNoPartialForm` →
    `recipient_add_has_no_partial_form`
  - `TestRecipientAddMakesHistoryReadable` →
    `recipient_add_makes_history_readable`: on the two-device fixture,
    B reads every entry A wrote before the add, after the pull on B's own
    unlock.

### 3. Re-encryption atomicity — 7 rows

Crash windows and the test that reaches each. **Reviewed at the end of
M9b:** each row was checked against the final `commit_recipient_list` /
`reencrypt_to` order, and none needs a new test. The order is: decrypt
all, parse the list, write the entries, write `.age-recipients`, write
`config.toml`, commit, push.

| Window | Test |
|---|---|
| Mid-entries (some entries rewritten, recipient files clean) | `TestInterruptedReencryptLeavesHEADUntouched`, `TestRetryingReencryptAfterAnInterruptionSucceeds` |
| One entry fails to decrypt or encrypt | `TestReencryptAbortsWholeOperationWhenOneEntryFails` (decrypt; it also asserts the tree stays clean, since nothing is written before every entry has decrypted and the list has parsed) |
| After the recipient files, before the commit (both `add` and `remove` shapes) | `TestInterruptedRecipientWriteLeavesNoHalfMigratedState` |
| Concurrent writer | `TestReencryptHoldsTheVaultLockThroughout` (child process, not a thread) |
| Between `.age-recipients` and `config.toml` (only the first written) | No dedicated test: the dirty surface is a subset of the row above, and M8a1's `reset_hard` tests pin that changes outside `entries/` are discarded |
| Inside `commit_all` (index staged, no commit yet) | No dedicated test: `reset_hard` is a hard reset, which restores the index too, and HEAD never moved |
| A failed write mid-loop (an `atomicfile` error, not a crash) | Same on-disk shape as the mid-entries row, reached through the error path; `atomicfile`'s temp files sit in the same directory and are removed on that path, and any that a crash leaves are untracked, which the reset removes |
| After the commit, before or during the push | Not a half-migration: HEAD holds all of it. `push_after_write` warns, and the CLI says "committed locally" |

- [x] `TestInterruptedReencryptLeavesHEADUntouched` →
      `interrupted_reencrypt_leaves_head_untouched`: HEAD's commit id
      is unchanged (a commit id hashes its content, so it is
      byte-identical), and the next write leaves the tree clean.
- [x] `TestRetryingReencryptAfterAnInterruptionSucceeds` →
      `retrying_reencrypt_after_an_interruption_succeeds`
- [x] `TestReencryptAbortsWholeOperationWhenOneEntryFails` →
      `reencrypt_aborts_whole_operation_when_one_entry_fails`: the error
      names the entry, and nothing is committed.
- [x] `TestReencryptLandsEverythingInExactlyOneCommit` →
      `reencrypt_lands_everything_in_exactly_one_commit`
- [x] `TestReencryptHoldsTheVaultLockThroughout` →
      `reencrypt_holds_the_vault_lock_throughout`: at each entry, a child
      process's attempt on the lock finds it held. Once the add is done,
      the same probe finds it free, which proves the probe can tell the
      two apart.
- [x] `TestInterruptedRecipientWriteLeavesNoHalfMigratedState` →
      `interrupted_recipient_write_leaves_no_half_migrated_state`: both
      shapes. The new entry is *unreadable* by the never-added device,
      which pins the reset ahead of the encrypt.
- [x] `TestDirtyWorkTreeResetIsAlsoRunByReencrypt` →
      `dirty_work_tree_reset_is_also_run_by_reencrypt`

### 4. `remove_recipient` — 8 rows

- [x] Check the last recipient before the self-removal confirmation, so a
      "yes" never meets a refusal. The test proves it: an unscripted
      `confirm` would panic the fake.
- [x] Rows:
  - `TestRecipientRemoveWithoutReencryptIsRejected` →
    `recipient_remove_without_reencrypt_is_rejected` (also before the
    lock: a child holds it, and the refusal doesn't wait)
  - `TestRecipientRemoveWithReencryptExcludesTheRemovedKey` →
    `recipient_remove_with_reencrypt_excludes_the_removed_key`
  - `TestRecipientRemoveWarnsThatRevocationIsFutureOnly` →
    `recipient_remove_warns_that_revocation_is_future_only`
  - `TestRecipientRemoveAcceptsEitherADeviceNameOrAPublicKey` →
    `recipient_remove_accepts_either_a_device_name_or_a_public_key`
  - `TestRecipientRemoveOfAnUnknownRecipientIsRejected` →
    `recipient_remove_of_an_unknown_recipient_is_rejected`
  - `TestRecipientRemoveRefusesTheLastRecipient` →
    `recipient_remove_refuses_the_last_recipient`
  - `TestRecipientRemoveOfThisDeviceNeedsConfirmation` →
    `recipient_remove_of_this_device_needs_confirmation`: declined means
    nothing written, and no warning is given before the answer.
    Confirmed works, here addressed by key, because every entry is
    decrypted with the still-valid identity first.
  - `TestAddIsNotRefusedOverATreeTheResetDiscards` →
    `add_is_not_refused_over_a_tree_the_reset_discards` (moved from
    task 2): a real self-removal, crashed after one entry, leaves a tree
    `require_full_access` refuses. The add re-encrypts HEAD's 3 entries
    regardless.

### 5. CLI — 10 rows

- [x] Register `recipient add` (`--device`, required; no `--reencrypt`)
      and `recipient remove` (`--reencrypt`), both modes, with the
      fixtures' `Short`s and long help. The result renders as Go's
      `recipientChangeLines` does, saying the re-encryption is committed
      locally.
- [x] The CLI's test prompter echoes library warnings to stderr:
      `lifecycle_tests::EchoingPrompter`, writing into the same buffer as
      `Io.stderr`, so the order is kept. All 423 earlier CLI tests stayed
      green with no assertion changed.
- [x] Rows:
  - `TestRecipientAddRejectsTheRemovedReencryptFlag` →
    `recipient_add_rejects_the_removed_reencrypt_flag` (and no unlock)
  - `TestRecipientAddShortDescribesTheReEncryption` →
    `recipient_add_short_describes_the_re_encryption`
  - `TestRecipientAddCommitsBothFilesInOneCommit` →
    `recipient_add_commits_both_files_in_one_commit`
  - `TestRecipientAddMakesHistoryReadable` →
    `recipient_add_makes_history_readable`: across `TwoDevices`, B
    `show`s what A wrote before the add.
  - `TestRecipientRemoveWithoutReencryptIsRejectedByTheCLI` →
    `recipient_remove_without_reencrypt_is_rejected_by_the_cli`: no
    unlock request is recorded.
  - `TestRecipientRemoveReencryptPrintsTheRevocationWarning` →
    `recipient_remove_reencrypt_prints_the_revocation_warning`: on
    stderr, not stdout. B can no longer `show` afterwards.
  - `TestRecipientCommandsAreAvailableInBothModes` →
    `recipient_commands_are_available_in_both_modes`
  - `TestRecipientListMatchesConfigAndReflectsChanges` →
    `recipient_list_matches_config_and_reflects_changes`
  - `TestShortHashLeavesShortStringsAlone` →
    `short_hash_leaves_short_strings_alone`
  - `TestIdentityAddPrintsANextCommandThatWorks` →
    `identity_add_prints_a_next_command_that_works`: the exact line B's
    `identity add` printed, run on device A, after which B reads A's
    entry.
- [x] Not a Go row: `recipient_remove_of_this_device_asks_and_a_no_writes_nothing`.
- [x] The P5 fixture-parity test
      (`p5_identity_commands_have_their_go_fixture_flags_and_descriptions`)
      covers `recipient add` and `recipient remove`: prose, flags, the
      parent's listing, and that `add --help` doesn't mention
      `--reencrypt`.

### 6. Lost-identity recovery — 1 row

- [x] `TestRecoveringALostIdentityFileNeedsNoFileRestore` →
      `recovering_a_lost_identity_file_needs_no_file_restore`: device B
      deletes its file, runs a fresh `identity add`, A runs
      `recipient add`, and B reads everything again.

### 7. The `--reencrypt` grep, docs, ledger

- [x] `rg -- --reencrypt` over the whole tree. Every hit in code names
      `remove`, says `add` has no such flag, or is a test pinning that
      `add` rejects it. `identity add`'s long help and printed next
      command, and the `clone` access lines, don't name it. Doc hits name
      it as removed (Q-PORT-STATE, the port plan, M9a) or describe
      `remove`/approval.
- [x] Parity ledger:
  - §2 `3fd784c`/`fbc887f`: `[x]`
  - §3 M9: M9b recorded as implemented. It becomes `[x]` after CI.
  - §4 `recipient_add`, `recipient_remove`: matched by the parity test,
    awaiting CI
- [ ] Mark M9b `[x]` in the [P5 index](index.md) after CI.

## Definition of done

Every box above is checked, and all 37 claimed rows are ported or excused
in writing. The crash-window table was reviewed at the end. `make lint`
and `make test` are clean locally and on Linux, macOS and Windows CI.

The milestone is done when two simulated devices can be added to a vault,
one can be removed with full re-encryption, an interrupted re-encryption
is provably invisible in committed history, and a device that can't read
everything is told so before anything happens.
