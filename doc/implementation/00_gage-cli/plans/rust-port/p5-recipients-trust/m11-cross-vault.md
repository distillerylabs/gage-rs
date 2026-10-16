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

## Tasks

Write each task's tests first, at task granularity.

### 1. The move itself — 7 rows

- [ ] `move_to` and `copy_to`, `with_two_vault_locks`, `SameVault`.
- [ ] Rows:
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

- [ ] Rows:
  - `TestMoveChecksTheDestinationsTrustCacheNotTheSources`
  - `TestMoveDecliningTheTrustCheckAbortsTheWholeOperation`: the source
    is untouched, and nothing is written to the destination.
  - `TestTwoVaultLocksAreAcquiredInOneDeterministicOrder`
  - `TestTwoVaultLocksBothHeldForTheWholeOperation`: a child process
    sees both locks contended mid-operation.

### 3. Crash and session — 2 rows

- [ ] Rows:
  - `TestMoveCrashBetweenTheTwoWritesLeavesARecoverableDuplicate`
  - `TestSessionIndexUpdatesBothVaultsWithoutARebuild`

### 4. CLI — 7 rows, and the Opus review

- [ ] Register `mv` and `cp` in group Entry CRUD (after `rm`), both
      modes, with `<query>` and a required `--to-vault` plus `-u/--use`,
      and the fixtures' `Short`s. Extend the P5 fixture-parity test.
- [ ] Check the order: an unregistered `--to-vault`, or a `--to-vault`
      naming the source, fails before any unlock.
- [ ] Rows:
  - `TestMvThroughTheCLIMovesEntryBetweenVaults`
  - `TestCpThroughTheCLILeavesOriginalAndAddsCopy`
  - `TestMvRequiresToVaultFlag`
  - `TestMvToUnregisteredVaultFailsCleanly`: nothing is decrypted
    (`decrypt_count`).
  - `TestMvToItsOwnSourceVaultIsRejected`
  - `TestMvDecliningTheTrustCheckAbortsThroughTheCLI`
  - `TestMvInASessionNeverUnlocksTheDestination`
- [ ] **Opus review (⚑)** of `TestTwoVaultLocksAreAcquiredInOneDeterministicOrder`,
      `…BothHeldForTheWholeOperation` and the crash test. Name the
      mutation each one kills:
  - order by argument instead of by id
  - release the first lock before the second write
  - source removal before the destination commit

  Record the table here.

### 5. User docs and ledger

- [ ] `doc/cli/identities-and-recipients.md` and `doc/cli/sharing.md`,
      ported from Go for the shipped surface: identities, recipients,
      `verify`/`--repair`, the trust cache and `--yes`, `mv`/`cp`. Leave
      out enrollment (P7) and the recovery key (P8), and say where they
      will go.
- [ ] Update `sync.md` for the recipient-change confirmation.
- [ ] Update `exit-codes.md` for the meanings P5 adds.
- [ ] Update `configuration.md` for `known-config.toml` under
      `$GAGE_STATE/<vault-id>/`.
- [ ] Update `README.md` and the command reference for P5's commands.
- [ ] Parity ledger:
  - §3 M9/M10/M11: `[x]`
  - §4 the P5 row
  - §6 `identities-and-recipients.md`, `sharing.md`
- [ ] Mark M11 and P5 `[x]` in the [P5 index](index.md) and the
      [port plan](../index.md) after CI. Tick P5 on the roadmap issue.

## Definition of done

Every box above is checked, and all 20 claimed rows are ported or excused
in writing. The review table is recorded and the P5 user docs describe
shipped behavior. `make lint` and `make test` are clean locally and on
Linux, macOS and Windows CI.

The milestone is done when an entry can be shared into a vault with a
different recipient list, without that vault being unlocked, without
either vault's index going stale, and without ever being lost from both.
