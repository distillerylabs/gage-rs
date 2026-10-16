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

- **The library surface.** Proposed:
  - `Vault::add_recipient(&self, device, pubkey, identity: &Identity, prompter: &dyn Prompter) -> Result<RecipientChange, RecipientError>`
  - `Vault::remove_recipient(&self, query, reencrypt: bool, identity, prompter) -> Result<RecipientChange, RecipientError>`
  - `Vault::require_full_access(&self, identity) -> Result<(), RecipientError>`,
    public and callable on its own (E4 places it differently)
  - `RecipientChange { device, pubkey, commit, reencrypted }`
  - `RecipientError` variants: `LastRecipient`, `RecipientExists`,
    `RecipientNotFound`, `ReencryptRequired`,
    `CannotGrantFullAccess { unreadable, total, device, vault }`
    (Conflict), `SelfRemovalDeclined`, plus wrapped lock, git and entry
    errors. Each maps to Go's exit code.
- **The prompter is a parameter.** Go asks through `ident.frontend()`.
  The Rust `Identity` carries no prompter, so `prompter` is passed
  explicitly, as every write already does
  ([Q-RECIPIENT-PROMPT](../open-questions.md#q-recipient-prompt)). The
  self-removal question is the first production caller of
  `Prompter::confirm`. Its default answer is no, so a non-interactive
  prompter aborts with nothing written.
- **Encrypt to an explicit list.** Add
  `Vault::write_entry_to(id, &Entry, &[crypt::Recipient])` beside
  `write_entry`, and make `write_entry` call it with the on-disk list. The
  re-encryption follows Go's order:
  1. decrypt every entry into memory first (a failure there writes
     nothing)
  2. write each entry to the new list
  3. write the recipient pair last
  4. one `commit_all`, then `push_after_write`

  This keeps the crash window's dirty surface to `entries/` for as long
  as possible.
- **`with_vault_write`.** It is `with_write_lock`, then `reset_hard`
  (when `dirty_paths` is non-empty: warn once through
  `Prompter::warn`, naming the paths), then the write. Every mutating
  entry method and both recipient verbs move onto it. The sync paths
  (`fast_forward`, merges, `sync_resolving`) keep the bare lock and keep
  *refusing* a dirty tree, as in Go. Confirm that `update`, which today
  takes no identity, has a prompter to warn through. If it doesn't, add
  one; that is what Go's call sites do.
- **The crash seam** ([Q-CRASH-SEAM](../open-questions.md#q-crash-seam)):
  `#[cfg(test)] on_reencrypt_entry: Option<Box<dyn Fn(usize) -> Result<(), _>>>`
  on `Vault`, called after each entry is written. A test returns an error
  from it to stand in for the crash, then drops the `Vault` without the
  commit, as Go's tests do. The recipient-files window is reached through
  the real `write_recipient_files`, called directly from a test (Go's
  review fix), not by hand-writing two lookalike files.
- **Commit messages:** `gage: recipient add <device> (reencrypt)` and
  `gage: recipient remove <device> (reencrypt)`. The device name is
  already plaintext in the commit's own diff.
- **The CLI rejects `remove` without `--reencrypt` before the unlock**,
  as Go does, so nobody is asked for a passphrase and then told the
  command line was wrong. The library check is the contract; the CLI check
  only orders the failure.

## Tasks

Write each task's tests first, at task granularity.

### 1. The dirty-tree reset on every write — 2 rows

- [ ] `with_vault_write`, with every mutating method moved onto it.
- [ ] `TestDirtyWorkTreeIsResetBeforeAnySubsequentWrite`: `insert`,
      `edit`, `generate` and `rename` each discard a leftover rather than
      folding it into their commit.
- [ ] `TestDirtyWorkTreeResetWarnsNamingTheDiscardedPaths`: one warning,
      through the prompter.
- [ ] The sync paths still refuse a dirty tree. The existing M8a1/M8a2
      tests stay green with no assertion changed.

### 2. `require_full_access` and `add_recipient` — 10 rows

- [ ] `write_entry_to` and `reencrypt_to`. The ordering is described in
      "Decisions to make first".
- [ ] `require_full_access`, gated on a clean tree before the lock. On a
      dirty tree it runs just after the reset, inside the lock, still
      before any write.
- [ ] Rows:
  - `TestRequireFullAccessIsCallableOnItsOwn`
  - `TestRecipientAddRefusesAnActorThatCannotReadEveryEntry`: build the
    state by hand-editing `.age-recipients`, as E1a says. The error
    names the count, is Conflict, and changes nothing.
  - `TestFullAccessPreflightRunsBeforeTheVaultLock`: hold the lock from
    a child process, and the refusal still arrives without waiting.
  - `TestAddIsNotRefusedOverATreeTheResetDiscards`: an interrupted
    self-removal leaves `entries/` unreadable while HEAD is fine.
  - `TestAddIsStillRefusedOnADirtyTreeWhenAccessIsGenuinelyPartial`
  - `TestAddRecipientRejectsInvalidDeviceNameOrPubkey`
  - `TestRecipientAddRejectsADuplicateKey`
  - `TestRecipientAddCommitsBothFilesTogether`: one commit holds both
    files and every entry.
  - `TestRecipientAddHasNoPartialForm`
  - `TestRecipientAddMakesHistoryReadable`: the new recipient reads
    every pre-existing entry, checked on device B of the two-device
    fixture.

### 3. Re-encryption atomicity — 7 rows

Crash windows and the test that reaches each:

| Window | Test |
|---|---|
| Mid-entries (some entries rewritten, recipient files clean) | `TestInterruptedReencryptLeavesHEADUntouched`, `TestRetryingReencryptAfterAnInterruptionSucceeds` |
| One entry fails to decrypt or encrypt | `TestReencryptAbortsWholeOperationWhenOneEntryFails` |
| After the recipient files, before the commit (both `add` and `remove` shapes) | `TestInterruptedRecipientWriteLeavesNoHalfMigratedState` |
| Concurrent writer | `TestReencryptHoldsTheVaultLockThroughout` (child process, not a thread) |

- [ ] `TestInterruptedReencryptLeavesHEADUntouched`: HEAD is
      byte-identical, and the next start leaves no dirty `entries/`.
- [ ] `TestRetryingReencryptAfterAnInterruptionSucceeds`
- [ ] `TestReencryptAbortsWholeOperationWhenOneEntryFails`: the error
      names the entry, and nothing is committed.
- [ ] `TestReencryptLandsEverythingInExactlyOneCommit`
- [ ] `TestReencryptHoldsTheVaultLockThroughout`: a concurrent write
      sees contention.
- [ ] `TestInterruptedRecipientWriteLeavesNoHalfMigratedState`: the
      plain-`add` shape. The new entry is *unreadable* by the never-added
      device, which pins the reset ahead of the encrypt.
- [ ] `TestDirtyWorkTreeResetIsAlsoRunByReencrypt`

### 4. `remove_recipient` — 7 rows

- [ ] Check the last recipient before the self-removal confirmation, so a
      "yes" never meets a refusal.
- [ ] Rows:
  - `TestRecipientRemoveWithoutReencryptIsRejected`
  - `TestRecipientRemoveWithReencryptExcludesTheRemovedKey`
  - `TestRecipientRemoveWarnsThatRevocationIsFutureOnly`
  - `TestRecipientRemoveAcceptsEitherADeviceNameOrAPublicKey`
  - `TestRecipientRemoveOfAnUnknownRecipientIsRejected`
  - `TestRecipientRemoveRefusesTheLastRecipient`
  - `TestRecipientRemoveOfThisDeviceNeedsConfirmation`: declined means
    nothing written. Confirmed works, because every entry is decrypted
    with the still-valid identity first.

### 5. CLI — 10 rows

- [ ] Register `recipient add` (`--device`, no `--reencrypt`) and
      `recipient remove` (`--reencrypt`), both modes, with the fixtures'
      `Short`s and long help. Render the result as Go's
      `recipientChangeLines` does, saying the re-encryption is committed
      locally.
- [ ] The CLI's test fake prompter echoes library warnings to stderr, as
      the real `TerminalPrompter` does (Go's M9 review fix). Check whether
      gage-cli's harness already does; if not, change it and keep earlier
      assertions green.
- [ ] Rows:
  - `TestRecipientAddRejectsTheRemovedReencryptFlag`
  - `TestRecipientAddShortDescribesTheReEncryption`
  - `TestRecipientAddCommitsBothFilesInOneCommit`
  - `TestRecipientAddMakesHistoryReadable`
  - `TestRecipientRemoveWithoutReencryptIsRejectedByTheCLI`: no unlock
    request is recorded.
  - `TestRecipientRemoveReencryptPrintsTheRevocationWarning`: on
    stderr.
  - `TestRecipientCommandsAreAvailableInBothModes`
  - `TestRecipientListMatchesConfigAndReflectsChanges`
  - `TestShortHashLeavesShortStringsAlone`
  - `TestIdentityAddPrintsANextCommandThatWorks`: run the exact line
    `identity add` printed, on device A.
- [ ] Extend the P5 fixture-parity test with `recipient add` and
      `recipient remove`.

### 6. Lost-identity recovery — 1 row

- [ ] `TestRecoveringALostIdentityFileNeedsNoFileRestore`: device B
      deletes its file, runs a fresh `identity add`, A runs
      `recipient add`, and B reads everything again.

### 7. The `--reencrypt` grep, docs, ledger

- [ ] `rg -- --reencrypt` over the whole tree. Every hit names `remove`,
      or is a test pinning that `add` rejects the flag. In particular,
      `identity add`'s long help and printed next command (M9a) and the
      `clone` access lines (M8a2) must not name it.
- [ ] Parity ledger:
  - §2 `3fd784c`/`fbc887f`: `[x]`
  - §3 M9: `[x]` after CI
  - §4 `recipient_add`, `recipient_remove`
- [ ] Mark M9b `[x]` in the [P5 index](index.md) after CI.

## Definition of done

Every box above is checked, and all 37 claimed rows are ported or excused
in writing. The crash-window table was reviewed at the end. `make lint`
and `make test` are clean locally and on Linux, macOS and Windows CI.

The milestone is done when two simulated devices can be added to a vault,
one can be removed with full re-encryption, an interrupted re-encryption
is provably invisible in committed history, and a device that can't read
everything is told so before anything happens.
