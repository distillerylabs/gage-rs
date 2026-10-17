# RK2 — The recovery identity and the swap, library

[← RK1](rk1-recovery-key-library.md) · [P8 plan](index.md) · [#76](https://github.com/distillerylabs/gage-rs/issues/76) · next: [RK3 — `init` gains the key; `recovery rotate`](rk3-init-and-rotate.md)

> **Recommended model: Opus ⚑.** This is a new, security-critical path. A
> full-read credential becomes an identity that must exist only briefly and
> only inside one function, and the change it makes must be atomic. The
> tests prove only what was thought to simulate. Run an **Opus review of
> the atomicity, the retired-key, the secret-in-errors and the M10 rows**
> (task 6) before calling it done.

## Goal

This is the library half of "the recovery key enrolls a fresh device, then
is retired", and of "replace the recovery key from an unlocked device". It
has four parts:

- **`recovery_identity`:** a recovery-scoped `Identity` that cannot leave
  `crates/gage`. It accepts only the key registered under
  `recovery-paper-key`.
- **`swap_recipients`:** one atomic change to the recipient list. It
  removes some labels and adds some recipients, then re-encrypts and makes
  one commit.
  - Every check the other recipient verbs make runs first, inside the lock
    and before the first byte is written: full access, M10's precondition
    and M10's question.
  - The final list is computed by a pure `apply_swap`, so every refusal
    writes nothing.
- **`Vault::recover_device`:** in one commit, add the new device, retire the
  recovery key used, and optionally add a replacement recovery key and
  remove the lost device.
- **`Vault::rotate_recovery_key`:** the same swap, driven by an ordinary
  unlocked identity. It replaces the recovery key, or adds one when there
  is none.

Nothing here prints. Questions go through the existing `Prompter`.

## Depends on

- **RK1:** `RECOVERY_DEVICE_LABEL`, `RecoveryError`, `new_recovery_key`
  (for test fixtures), `verify_recovery_key`, and the reservation.
- **P2 · M2:** `KeyLine::parse` and `Identity::new` (both `pub(crate)`),
  `Vault::locker`, and the page-lock warning wording in `Vault::unlock`.
- **P5 · M9b / P7 · E1b:** `commit_recipient_list` and `RecipientWrite`,
  `require_full_access`, `with_vault_write` and its dirty-tree reset.
  - This milestone widens `commit_recipient_list` from private to
    `pub(crate)`. Nothing else about it changes.
- **P5 · M10:** `require_recipients_in_sync`, `confirm_recipient_trust`,
  `note_recipients_reviewed`.
- **P4 · M8a2:** `sync_on_unlock`. `recover_device` skips `unlock`, so it
  pulls first, as Go's does.
- **P5:** the crash seam `with_reencrypt_hook`, and `reencrypt_tests.rs`'s
  `crashed`/`SimulatedCrash` helpers
  ([Q-CRASH-SEAM](../open-questions.md#q-crash-seam)).

## Design references

- TDD "Recovery key at `init`", the bullets on `recovery enroll`,
  `recovery rotate`, "Both are one commit" and "Retirement is
  future-only".
- E4's `enrollment/approve.rs`: the precedent for a bespoke operation built
  on the shared recipient write.

## Go sources

- Go plan `recovery-key/r3-recovery-identity-library.md`, all of it,
  especially these two sections:
  - **"Decisions to make first"**: four items. Full access always runs
    inside the lock; collisions are checked against the final list; a
    `--replaces` that matches no device is an error; the
    zero-the-caller's-buffer asymmetry.
  - **"Found by review"**: the retired key re-added by the swap that
    retired it.
- Go plan `recovery-key/r5-recovery-rotate-and-docs.md`, "Settled while
  implementing" and "Found by review":
  - `remove_if_present` and `inspect`
  - rotate refuses the key it is retiring
  - rotate refuses to evict the acting device
  - the future-only warning covers `remove_if_present`
- Go plan `recovery-key/index.md`, D8, D10, D12, and "Design facts Phase 2
  relies on".
- Parity ledger: §1 #16 (PR #20), and the library half of #18 (PR #22).
- Go sources at the pinned SHA: `internal/gage/recovery_enroll.go`.
- Test inventory, 25 rows of `internal/gage/recovery_enroll_test.go`, all
  except `TestTheRecoveryLabelIsReservedForTheRecoveryKey` (RK1's).

## Decisions to make first

Settle these in this doc, and get them reviewed, before writing code.

1. **Signatures.** Rust CRUD takes the identity and the prompter
   explicitly; Go reaches the prompter through `ident.frontend()`. Proposed:

   ```rust
   pub struct RecoverSpec {
       pub device: String,
       pub pubkey: String,
       pub replaces: Option<String>,
       pub new_recovery_pubkey: Option<String>,
   }

   pub fn recover_device(&self, spec: &RecoverSpec, secret: Zeroizing<String>,
                         p: &dyn Prompter) -> Result<RecoverResult, RecoveryError>;

   pub fn rotate_recovery_key(&self, new_pubkey: &str, identity: &Identity,
                              p: &dyn Prompter) -> Result<RotateResult, RecoveryError>;
   ```

   The result types:
   - `RecoverResult` carries `device`, `pubkey`, `retired_recovery`,
     `new_recovery_pubkey`, `replaced`, `commit` and `reencrypted`.
   - `RotateResult` carries `retired: Option<String>`, `created`, `pubkey`,
     `commit` and `reencrypted`.
2. **The secret is taken by value**
   ([Q-RECOVERY-SECRET-MEMORY](../open-questions.md#q-recovery-secret-memory)).
   - Go's `RecoverDevice` zeroes the caller's buffer on every path, because
     Go can't move it.
   - Here the `Zeroizing<String>` moves in and is dropped on every return.
     The parsed key lives in `KeyLine`'s `LockedBuf` and is zeroed by
     `Identity`'s `Drop`.
   - So `TestRecoverDeviceZeroesTheSecretItWasHanded` becomes a signature
     pin plus a no-copy check (task 3).
3. **`RecoveryError` grows, and keeps every wrapped code.** It gains:
   - `NotRecoveryKey { device: Option<String>, pubkey, vault }`, as
     `LockedOrAuth`. A device key names its label; a stranger names its
     pubkey.
   - `RecipientExists`-, `RecipientNotFound`- and `LastRecipient`-shaped
     variants for the swap's refusals (`Conflict`, `NotFound`, `Conflict`),
     or a `Recipient(#[from] RecipientError)` variant that reuses those.
     Proposed: reuse `RecipientError`'s variants through `#[from]`, so a
     caller matching on `RecipientExists` sees the same variant from every
     recipient verb.
   - `Usage` variants for an invalid device name or key, and for
     `replaces == RECOVERY_DEVICE_LABEL`.

   No variant's `Display` or `Debug` contains a secret. age's parse errors
   are dropped, not wrapped, because they can quote their input.
4. **Where `swap_recipients` lives.**
   - Proposed: `recovery/swap.rs`, with `recipient_swap` as a private
     struct holding `add`, `remove`, `remove_if_present`, `message` and an
     optional `inspect` closure.
   - `commit_recipient_list` widens to `pub(crate)`.
   - The future-only warning uses Go's swap wording ("…it still opens every
     version of every entry already in this vault's git history"). It does
     not use `remove_recipient`'s, which talks about what a device "already
     read".

## Tasks

Write each task's tests first. Rows go in
`crates/gage/src/recovery/swap_tests.rs`, with a shared fixture building
these, in the style of Go's `newRecoverFixture`:
- an owner device
- a recovery key, via `create`'s extras from RK1
- a remote
- a few entries

### 1. `recovery_identity` — 5 rows

- [ ] A private method, not `pub(crate)`. It runs these steps:
  1. Trim the secret, then `KeyLine::parse` it.
  2. Find its pubkey in `recipients()`. It must be the one labelled
     `RECOVERY_DEVICE_LABEL`.
  3. Build an `Identity` with device `RECOVERY_DEVICE_LABEL` through
     `Identity::new`. A page-lock failure warns through the prompter with
     `unlock`'s wording.
- [ ] Rows:
  - `TestRecoveryIdentityAcceptsOnlyTheRecoveryRecipient`: `recovery_identity_accepts_only_the_recovery_recipient`
  - `TestRecoveryIdentityRefusesADeviceKey`: `recovery_identity_refuses_a_device_key`. `NotRecoveryKey` (`LockedOrAuth`), naming the device's label, even though the key would decrypt.
  - `TestRecoveryIdentityRefusesStrangersAndGarbage`: `recovery_identity_refuses_strangers_and_garbage`. A stranger gives `NotRecoveryKey`; garbage gives `MalformedRecoveryKey` (`Usage`).
  - `TestRecoveryIdentityToleratesPastedWhitespace`: `recovery_identity_tolerates_pasted_whitespace`
  - `TestRecoveryErrorsNeverQuoteTheSecret`: `recovery_errors_never_quote_the_secret`. Every refusal path in this milestone is formatted with `{}`, `{:?}` and `{:#}`, and none contains the secret or any 20-character run of it.

### 2. `swap_recipients` and `recover_device` — 10 rows

- [ ] `swap_recipients`, under `with_vault_write`, in Go's order:
  1. `require_full_access`, always inside the lock (Go R3 decision 1)
  2. `require_recipients_in_sync`
  3. `confirm_recipient_trust`
  4. read the current list
  5. `inspect`
  6. `apply_swap`
  7. one future-only warning per label actually held, for both `remove` and `remove_if_present`
  8. `commit_recipient_list` with `RecipientWrite::message`
  9. `note_recipients_reviewed`
- [ ] `apply_swap`, pure:
  - remove the labels
  - an unmatched `remove` label is `RecipientNotFound`
  - `remove_if_present` skips silently
  - add each recipient, after validating its name and key, against the
    **final** list (duplicate key or label is `RecipientExists`)
  - refuse an empty result (`LastRecipient`)
- [ ] `recover_device`:
  1. `sync_on_unlock` first.
  2. `recovery_identity`.
  3. `check_recover_spec`, against the retired pubkey. It covers an invalid
     device, the reserved label as the device, an invalid key, the retired
     key as the device, the retired key as the replacement, and
     `replaces == RECOVERY_DEVICE_LABEL`.
  4. The swap: add the device, plus the new recovery key if one is given;
     remove `recovery-paper-key`, plus `replaces` if one is given.

  The commit message is
  `gage: recovery enroll <device>, retire recovery-paper-key (reencrypt)`.
- [ ] Rows:
  - `TestRecoverDeviceSwapsTheListInOneCommit`: `recover_device_swaps_the_list_in_one_commit`. The exact list is in both files, with exactly one new commit, and `replaces` removes the lost device in that same commit.
  - `TestRecoverDeviceRetiresTheUsedKeyForFutureWrites`: `recover_device_retires_the_used_key_for_future_writes`. The old key opens no entry written after the swap.
  - `TestRecoverDeviceLetsTheNewDeviceRead`: `recover_device_lets_the_new_device_read`. Every entry opens with the new device key and with the new recovery key.
  - `TestRecoverDeviceWithoutAReplacementLeavesNoRecoveryRecipient`: `recover_device_without_a_replacement_leaves_no_recovery_recipient`
  - `TestRecoverDeviceLeavesUpdatedByAlone`: `recover_device_leaves_updated_by_alone`
  - `TestRecoverDeviceRefusalsWriteNothing`: `recover_device_refusals_write_nothing`. One case per refusal: duplicate key, duplicate label, `replaces` naming an absent device, `replaces` naming the recovery label, and the reserved label as the device. Each pins its variant and code, HEAD unchanged and a clean tree.
  - `TestRecoverDeviceReusingTheReplacedName`: `recover_device_reusing_the_replaced_name`. `--device laptop-1 --replaces laptop-1` is accepted, because the check runs against the final list.
  - `TestRecoverDeviceRefusesToReAddTheRetiredKey`: `recover_device_refuses_to_re_add_the_retired_key`. The retired key is refused both as the device key and as the new recovery key (`RecipientExists`, `Conflict`), with nothing written.
  - `TestRecoverDeviceStillAsksM10sQuestion`: `recover_device_still_asks_m10s_question`. With an unreviewed change in the trust cache, the question is asked; a declining prompter aborts with nothing written.
  - `TestApplySwapRefusesLeavingNothingToEncryptTo`: `apply_swap_refuses_leaving_nothing_to_encrypt_to`

### 3. The secret's lifetime — 1 row

- [ ] Row:
  - `TestRecoverDeviceZeroesTheSecretItWasHanded`: `recover_device_takes_the_secret_by_value`, ported per decision 2.
    - A type assertion pins the parameter as `Zeroizing<String>`, by value: `let _: fn(&Vault, &RecoverSpec, Zeroizing<String>, &dyn Prompter) -> _ = Vault::recover_device;`.
    - A source scan of `recovery/` finds no `.clone()` or `to_string()` on the secret, and no `String` built from it outside `KeyLine::parse`. This is the same kind of scan as `crypt.rs`'s guard tests.
    - Go's row name is kept in the test's doc comment.

### 4. Atomicity — 1 row ⚑

- [ ] Row:
  - `TestInterruptedRecoverDeviceLeavesHEADUntouched`: `interrupted_recover_device_leaves_head_untouched`.
    - A crash via `with_reencrypt_hook` after the first re-encrypted entry and before the commit.
    - After it: HEAD unchanged, both recipient files unchanged, the recovery key still listed and the new device not.
    - The next write's dirty-tree reset leaves a clean tree.
    - The pattern is `enrollment/approve_atomic_tests.rs`'s.

### 5. `rotate_recovery_key` — 8 rows

- [ ] The swap with these settings:
  - add `recovery-paper-key → new_pubkey`
  - `remove_if_present: [RECOVERY_DEVICE_LABEL]`
  - message `gage: recovery rotate (reencrypt)`
  - an `inspect` that reads the retired pubkey under the lock, refuses
    `new_pubkey == retired`, refuses evicting the acting identity's own key
    (the backstop), and sets `created`
- [ ] Rows:
  - `TestRotateRecoveryKeySwapsInOneCommit`: `rotate_recovery_key_swaps_in_one_commit`. The old key stops opening new ciphertext, and the new one opens everything.
  - `TestRotateRecoveryKeyCreatesOneWhenThereIsNone`: `rotate_recovery_key_creates_one_when_there_is_none`. `created: true`, `retired: None`.
  - `TestRotateRecoveryKeyRefusesTheKeyItIsRetiring`: `rotate_recovery_key_refuses_the_key_it_is_retiring`
  - `TestRotateRecoveryKeyRefusalsWriteNothing`: `rotate_recovery_key_refusals_write_nothing`
  - `TestInterruptedRotateRecoveryKeyLeavesHEADUntouched`: `interrupted_rotate_recovery_key_leaves_head_untouched` ⚑
  - `TestRotateRecoveryKeyStillAsksM10sQuestion`: `rotate_recovery_key_still_asks_m10s_question`
  - `TestRotateRefusesToEvictTheActingDevice`: `rotate_refuses_to_evict_the_acting_device`.
    - The state can't be reached through any reserved path, so the fixture hand-writes `config.toml` and `.age-recipients` with the owner's key under the label, then notes the list reviewed, as Go's test does.
    - The result is `RecipientExists` (`Conflict`), with nothing written.
  - `TestRotateWarnsThatRetirementIsFutureOnly`: `rotate_warns_that_retirement_is_future_only`. The warning names the label, comes through `remove_if_present`, and is absent when there was nothing to retire.

### 6. Opus review ⚑

- [ ] An Opus pass over tasks 1–5's tests, recorded here as a table: what
  it checked, and what it found.
  - Can any error carry the secret?
  - Does a crash at any point (first entry, last entry, before the commit)
    leave HEAD untouched?
  - Is the retired key blocked in both shapes?
  - Does a declined M10 question write nothing?
  - Is `recovery_identity` unreachable from outside the crate? Check this
    with a `trybuild`-free argument: it is private, and `Identity::new` is
    `pub(crate)`. Record the reasoning.

### 7. Parity and ledger

- [ ] All 25 rows ported, or struck with a reason in this doc.
- [ ] Ledger §1 #16 and #18 noted `[~]`. #16's relative-path guard and #18's command are RK3's.
- [ ] `make lint`, `make test`; CI on Linux, macOS and Windows.

## Definition of done

- Every box above is checked and all 25 claimed rows are ported.
- RK1's and P1–P7's suites are still green.
- There is no exported way to obtain a recovery-scoped `Identity`.
- `make lint` and `make test` are clean locally and on all three CI platforms.
