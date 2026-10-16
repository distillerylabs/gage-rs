# M11 — Cross-vault `mv`/`cp`

[← M10](m10-trust-cache.md) · [P5 plan](index.md) · [#45](https://github.com/distillerylabs/gage-rs/issues/45) · next: P6 Polish

> **Recommended model: Sonnet\* ⚑.** A small milestone: M2's encrypt
> primitive pointed at a second vault's recipients. The real wrinkles are
> the two-vault lock ordering and the session borrow. Run an **Opus review
> of the lock-order test (child processes) and the crash-between-commits
> test** (task 4) before calling it done. This milestone also writes the
> P5 user docs.

## Goal

`gage mv <query> --to-vault <name>` and `gage cp <query> --to-vault <name>`
decrypt an entry from the source vault and re-encrypt it to the
destination vault's recipients. With one flat recipient list per vault,
this is the whole sharing story.

- **The destination is never unlocked.** Encrypting to public keys needs
  no private key.
- The trust check runs against the **destination's** cache before
  encrypting there. Declining aborts the whole operation.
- Both vault locks are held for the whole operation, acquired in vault-id
  order.
- The write order is fixed:
  1. write and commit in the destination, and push
  2. then, for `mv`, remove and commit in the source, and push

  A crash between them leaves a recoverable duplicate, never a lost entry.
- The copy is a new entry: a fresh UUID, fresh `created`/`updated`, and
  `updated_by` set to this device's name in the source. Title,
  description, value and fields are carried over. A title collision in
  the destination is allowed (there is no `-f`).
- In a session, the source index drops the entry and the destination
  index gains it, with no rebuild.

## Depends on

- **M10:** `confirm_recipient_trust` against a `Vault` that holds no
  identity, and the dirty-tree reset (`with_vault_write`'s parts, used on
  both vaults under the two locks).
- **M9b:** `write_entry`, `commit_all` + `push_after_write`, and the
  `#[cfg(test)]` hook-field pattern
  ([Q-CRASH-SEAM](../open-questions.md#q-crash-seam)).
- **P3 · M5:** the resolver, which resolves in the source with candidate
  lists.
- **P3 · M6/M7:**
  - `Session::vault_without_unlocking`
  - `forget_entry`, `note_entry` and `refresh_entry`
- **P2 · M4:** `vaultlock`, `WRITE_LOCK_TIMEOUT`, and the child-process
  lock test pattern.

## Design references

- Design doc, "Why no per-directory sharing": the `mv --to-vault`
  example.
- Design doc, "`mv`/`cp` are now cross-vault, and that's the sharing
  mechanism".
- Design doc, "Local trust cache": the destination's cache is the one
  checked.

## Go sources

- Go plan `gage-cli-design/m11-cross-vault-sharing.md`: all four
  resolved decisions. The lock order is *amended*: Go's code after A20
  orders by vault id, not by name
  ([Q-TWO-VAULT-LOCK](../open-questions.md#q-two-vault-lock)).
- Parity ledger rows:
  - §3 M11
  - §4 `mv`, `cp`
  - §6 `identities-and-recipients.md`, `sharing.md`
- Go sources at the pinned SHA:
  - `internal/gage/move.go`: `Move`, `Copy`, `moveOrCopy`,
    `withTwoVaultLocks`, `ErrSameVault`
  - `cmd/gage/move.go`: `runMoveOrCopy`, the session's
    `VaultWithoutUnlocking` and `NoteEntry`
- Go user docs: `doc/cli/identities-and-recipients.md` and
  `doc/cli/sharing.md`, without the enrollment and recovery sections
  (P7, P8).
- Test inventory, 20 rows:
  - `internal/gage/move_test.go` (13)
  - `cmd/gage/move_test.go` (7)

## Decisions to make first

- **The library surface.** Proposed:
  - `Vault::move_to(&self, query, dest: &Vault, identity, prompter) -> Result<MoveResult, MoveError>`
  - `Vault::copy_to(&self, query, dest: &Vault, identity, prompter) -> Result<MoveResult, MoveError>`
  - `MoveResult { source_id, dest_id, entry }`
  - `MoveError::SameVault` (Usage). It is checked by **id** before any
    lock. Locking one vault twice in one process would wait out the full
    timeout, because each `flock` belongs to its own open file
    description.
- **The two-vault lock helper.** `with_two_vault_locks(a, b, f)` sorts by
  `id` and holds both locks for the whole of `f`. Each lock is released
  on every exit path. Check whether `with_write_lock`'s
  prepare-lock-file step can be shared rather than copied.
- **The session borrow.** `Session::vault` borrows `&mut self`, so the
  session can't hand out the source's `(&Vault, &Identity)` and the
  destination `Vault` at the same time. Options:
  - a `Session` method that returns the source pair together with the
    destination opened by `vault_without_unlocking`. The destination is
    registered with the session, so a later pull still invalidates its
    index. This is what Go does.
  - opening the destination through the session's opener as a value,
    outside the session's map

  Prefer the first. Pick one here and record why. Afterwards:
  `forget_entry(source)`, and `note_entry(dest)` only when the session
  holds the destination's index.
- **The resolve step** happens under both locks, after both resets, and
  before the destination's trust check, as in Go. An ambiguous query in
  one-shot mode fails with the candidate list. In a session it goes
  through the picker, as other mutations do (`Keys::mutation_query`).
- **The crash seam:** `#[cfg(test)] on_move_dest_committed` on the source
  `Vault`, called after the destination commit and push.

### Resolutions

Reviewed and implemented as below (the two edits made while
implementing are the crash seam and the session method's same-name check). Nothing below needs a new `Q-`
entry: each follows an existing one
([Q-TWO-VAULT-LOCK](../open-questions.md#q-two-vault-lock),
[Q-CRASH-SEAM](../open-questions.md#q-crash-seam)).

- **Library surface: as proposed.** `move_to` and `copy_to` are thin
  wrappers over one private `move_or_copy(remove_source)`.
  - `query` is a `&str`, resolved against the **source** titles inside the
    locks, as `remove` does, so the CLI passes `Keys::mutation_query`'s
    result unchanged.
  - The destination copy is built from the decrypted entry with a fresh
    `new_id()`, `created = updated = now` and
    `updated_by = identity.device()`; it is written with
    `dest.write_entry`, which encrypts to the destination's recipients.
  - One commit per vault, each named by that vault's entry id
    (`dest_id`, then `source_id`), matching `insert`/`remove`.
  - `MoveError` wraps `LockError`, `TrustError`, `ResolveError`,
    `gitrepo::Error`, `Error` (decrypt/encrypt) and an `Io` for the
    removal, plus `SameVault`. `SameVault` maps to `Code::Usage`.
- **Lock helper.** `Vault::with_two_vault_locks(a, b, timeout, f)` lives
  next to `with_write_lock` in `entry/crud.rs`.
  - It reuses `prepare_lock_file` (widened from private to `pub(crate)`),
    so the lock-file path and directory handling aren't copied.
  - It sorts by `id` and acquires in that order, then runs the same
    `reset_dirty_work_tree` on **both** vaults, then `f`.
  - Both locks are released on every path, in reverse order. The closure's
    error wins over a release error, as in `with_write_lock`.
  - The same-id check runs first, before `prepare_lock_file`.
- **Session borrow: the first option.** One new method,
  `Session::vault_and_destination(source, dest) -> Result<(&Vault, &Identity, &Vault), SessionError>`.
  - It `hold`s both names (so the destination is registered and a later
    pull still invalidates its index), unlocks only the source, and
    re-borrows both out of the map immutably.
  - It does not check `source == dest` itself: the CLI refuses a shared
    name before calling it, and the library's `SameVault` (by id) is the
    backstop.
  - After the move the CLI calls `forget_entry(source, source_id)` and
    `note_entry(dest, dest_id, &entry)`. `note_entry` is already a no-op
    for a vault with no index, which is exactly "only when the session
    holds the destination's index"; no new session code is needed for it.
  - Why not the second option: an opener value outside the map isn't
    registered, so a pull would leave the destination's index stale.
- **Resolve step: inside the library, under both locks, after both
  resets, before the destination trust check.** One-shot, an ambiguous
  query fails with the candidate list (`write_failure`'s mapping). In a
  session, `mutation_query` resolves through the picker first and hands
  the library the id, as `rm` does, so the library's second resolve can
  only match the chosen entry.
- **Trust check:** `dest.confirm_recipient_trust(prompter)`. It needs no
  identity on `dest` (it reads the destination's own cache and files),
  and it is called with the destination `Vault`'s `name`, so a decline
  reports the destination.
- **Crash seam:** `#[cfg(test)]` field on the source `Vault`
  (`MoveHook`), the same shape as M9b's reencrypt hook: a closure called
  after the destination's commit and push, which a test panics from
  (`SimulatedCrash`) so only `Drop`s run, as a killed process leaves. An
  `Err` would run the move's own error path and prove cleanup rather than
  an interrupted run. The test then asserts the entry is in both vaults.
- **CLI ordering (task 4):** flags first; then `--to-vault` is looked up
  in the registry and the destination `Vault` built, then names and
  vault **ids** compared, all before any unlock. So an unregistered
  destination and a destination that is the source (by name or by id)
  fail with nothing decrypted. The library's `SameVault` stays as the
  backstop for library callers.

## Tasks

Write each task's tests first, at task granularity.

### 1. The move itself — 7 rows

- [x] `move_to` and `copy_to`, `with_two_vault_locks`, `SameVault`.
- [x] Rows:
  - `TestMoveRemovesFromSourceAndDecryptsAtDestination`: the destination
    is never unlocked, and B's identity in the two-device fixture
    decrypts it.
  - `TestCopyLeavesSourceAndAddsDecryptableCopy`
  - `TestMoveProducesOneCommitInEachVault`
  - `TestMoveGivesTheDestinationCopyFreshProvenance`
  - `TestMoveAllowsATitleCollisionInTheDestination`
  - `TestMoveRejectsTheSourceVaultAsItsOwnDestination`
  - `TestMoveRejectsTwoRegistrationsOfOneVaultAsSourceAndDestination`:
    same id, two names, refused before any lock, and it returns at once
    rather than after the timeout.

### 2. Trust and locks — 4 rows

- [x] Rows:
  - `TestMoveChecksTheDestinationsTrustCacheNotTheSources`
  - `TestMoveDecliningTheTrustCheckAbortsTheWholeOperation`: the source
    is untouched, and nothing is written to the destination.
  - `TestTwoVaultLocksAreAcquiredInOneDeterministicOrder`
  - `TestTwoVaultLocksBothHeldForTheWholeOperation`: a child process
    sees both locks contended mid-operation.

### 3. Crash and session — 2 rows

- [x] Rows:
  - `TestMoveCrashBetweenTheTwoWritesLeavesARecoverableDuplicate`
  - `TestSessionIndexUpdatesBothVaultsWithoutARebuild`

### 4. CLI — 7 rows, and the Opus review

- [x] Register `mv` and `cp` in group Entry CRUD (after `rm`), both
      modes, with `<query>` and a required `--to-vault` plus `-u/--use`,
      and the fixtures' `Short`s. Extend the P5 fixture-parity test.
- [x] Check the order: an unregistered `--to-vault`, or a `--to-vault`
      naming the source, fails before any unlock.
- [x] Rows:
  - `TestMvThroughTheCLIMovesEntryBetweenVaults`
  - `TestCpThroughTheCLILeavesOriginalAndAddsCopy`
  - `TestMvRequiresToVaultFlag`
  - `TestMvToUnregisteredVaultFailsCleanly`: nothing is decrypted
    (`decrypt_count`).
  - `TestMvToItsOwnSourceVaultIsRejected`
  - `TestMvDecliningTheTrustCheckAbortsThroughTheCLI`
  - `TestMvInASessionNeverUnlocksTheDestination`
- [x] **Opus review (⚑)** of `TestTwoVaultLocksAreAcquiredInOneDeterministicOrder`,
      `…BothHeldForTheWholeOperation` and the crash test. Name the
      mutation each one kills:
  - order by argument instead of by id
  - release the first lock before the second write
  - source removal before the destination commit

  Record the table here.

### Opus review table (⚑)

Reviewed by Opus. Each mutation was scripted against the real code, run
against the whole `movecopy_tests` module (13 tests), and reverted; the
files were checked byte-identical afterwards. Each mutation fails exactly
the test named for it, and the other 12 pass.

| Test | Mutation it kills | Result |
|---|---|---|
| `two_vault_locks_are_acquired_in_one_deterministic_order` | M1, order by argument: `(first, second) = (a, b)` | fails (the `(hi, lo)` iteration); 12 pass |
| `two_vault_locks_are_both_held_for_the_whole_operation` | M2a, the helper calls `held_first.release()` before running `f` | fails; 12 pass |
| `two_vault_locks_are_both_held_for_the_whole_operation` | M2b, `move_or_copy` takes two sequential single-vault locks (`with_vault_write` on the source to resolve and read, on the destination to write, on the source again to remove) | fails; 12 pass |
| `move_crash_between_the_two_writes_leaves_a_recoverable_duplicate` | M3, the source removal, commit and push run before the destination's write (the hook stays between the two) | fails; 12 pass |

M2a and M2b replace the implementing model's earlier stand-in, a lock
taken on a throwaway path. M2a is a literal early release. M2b is the
regression most likely in practice: rewriting the move from the
single-vault helpers.

Notes from the review, none of which needs a change:

- Both lock probes (`lock_is_held_elsewhere`, `Holder`) are child
  processes running `lock_holder_trampoline`, so they test per-process
  file-lock semantics rather than threads, as CLAUDE.md requires.
- The order test only kills M1 because it runs both argument orders. The
  `(lo, hi)` iteration alone would pass under M1. Keep the loop over both
  orders.
- The crash seam fires only on `move`. `copy` has one write, so it has no
  window between writes to cover.
- A failed second acquire releases the first lock (`Lock`'s `Drop`). The
  order test asserts this after each iteration.

### 5. User docs and ledger

- [x] `doc/cli/identities-and-recipients.md` and `doc/cli/sharing.md`,
      ported from Go for the shipped surface: identities, recipients,
      `verify`/`--repair`, the trust cache and `--yes`, `mv`/`cp`. Leave
      out enrollment (P7) and the recovery key (P8), and say where they
      will go.
- [x] Update `sync.md` for the recipient-change confirmation.
- [x] Update `exit-codes.md` for the meanings P5 adds.
- [x] Update `configuration.md` for `known-config.toml` under
      `$GAGE_STATE/<vault-id>/`.
- [x] Update `README.md` and the command reference for P5's commands.
- [x] Parity ledger:
  - §3 M9/M10/M11: `[x]`
  - §4 the P5 row
  - §6 `identities-and-recipients.md`, `sharing.md`
- [x] Mark M11 and P5 `[x]` in the [P5 index](index.md) and the
      [port plan](../index.md) after CI (green on Linux, macOS and Windows
      at `6ce4d07` and `c3717b7`, [#50](https://github.com/distillerylabs/gage-rs/pull/50)). Tick P5 on the roadmap issue.

## Definition of done

Every box above is checked, and all 20 claimed rows are ported or excused
in writing. The review table is recorded and the P5 user docs describe
shipped behavior. `make lint` and `make test` are clean locally and on
Linux, macOS and Windows CI.

The milestone is done when an entry can be shared into a vault with a
different recipient list, without that vault being unlocked, without
either vault's index going stale, and without ever being lost from both.
