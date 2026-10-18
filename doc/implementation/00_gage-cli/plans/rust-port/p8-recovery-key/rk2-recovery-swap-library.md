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

Settled here, before any code, and reviewed. Decisions 1, 2 and 4 are
settled as proposed, with the details below added. Decision 3 changes the
plan's error shape for three refusals
([Q-RECOVERY-ERRORS](../open-questions.md#q-recovery-errors)).

1. **Signatures. Settled.** Rust CRUD takes the identity and the prompter
   explicitly; Go reaches the prompter through `ident.frontend()`.

   ```rust
   #[derive(Debug, Clone, PartialEq, Eq)]
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

   The result types use `Option` wherever Go uses an empty string:
   - `RecoverResult` carries `device`, `pubkey`, `retired_recovery`,
     `new_recovery_pubkey: Option<String>`, `replaced: Option<String>`,
     `commit` and `reencrypted`.
   - `RotateResult` carries `retired: Option<String>`, `created`, `pubkey`,
     `commit` and `reencrypted`.

   `rotate_recovery_key` validates `new_pubkey` before the lock, as Go
   does. Everything else it checks runs under the lock, in `inspect`.
2. **The secret is taken by value. Settled**
   ([Q-RECOVERY-SECRET-MEMORY](../open-questions.md#q-recovery-secret-memory)).
   - Go's `RecoverDevice` zeroes the caller's buffer on every path, because
     Go can't move it.
   - Here the `Zeroizing<String>` moves into `recover_device`, and from
     there **into `recovery_identity`**, which consumes it. It is dropped,
     and so zeroed, as soon as `KeyLine::parse` has copied it into a
     `LockedBuf`. That is earlier than "on every return": the
     unlocked heap copy does not stay alive across the lock, the
     re-encryption and the push. The parsed key lives in `KeyLine`'s
     `LockedBuf` and is zeroed by `Identity`'s `Drop`.
   - The trim is `str::trim` on the borrowed string, so it is a slice and
     makes no copy.
   - So `TestRecoverDeviceZeroesTheSecretItWasHanded` becomes a signature
     pin plus a no-copy check (task 3).
3. **`RecoveryError` grows, and keeps every wrapped code. Settled.**
   - **`Recipient(#[from] RecipientError)`**, transparent. The ordinary
     refusals reuse `RecipientError`'s variants, so a caller matching on
     `RecipientExists` sees the same variant from every recipient verb.
     That covers a key or label colliding with the final list
     (`RecipientExists`, `Conflict`), an unmatched `replaces`
     (`RecipientNotFound`, `NotFound`), an empty result (`LastRecipient`,
     `Conflict`), an invalid device name, the reserved label as the device,
     and an invalid key (`InvalidDeviceName`, `ReservedDeviceLabel`,
     `InvalidRecipient`, all `Usage`). It also covers everything the shared
     write can fail with: M10, the lock, git and re-encryption.
     - `swap_recipients` returns `RecoveryError`, so `inspect` can refuse
       with a recovery variant. `RecoveryError` therefore gets
       `From<LockError>` and `From<gitrepo::Error>`, both through
       `RecipientError`, which `with_vault_write` needs.
   - **`NotRecoveryKey { device: Option<String>, pubkey, vault }`**, as
     `LockedOrAuth`. A device key names its label; a stranger names its
     pubkey.
   - **`ReplacesRecoveryKey`**, as `Usage`: `replaces ==
     RECOVERY_DEVICE_LABEL`. Its message is Go's: the key is retired
     whether or not it is named, and `--replaces` is for the lost device.
   - **The change to the plan: two recovery-specific variants, both
     `Conflict`, instead of `RecipientExists`.** Go wraps
     `ErrRecipientExists` for three refusals whose remedy differs from an
     ordinary collision. Its messages say so: "generate a new key", and
     "re-register this device under its own name first".
     `RecipientExists`'s message would instead read "age1… is already
     listed as recovery-paper-key". That is true, but it hides the reason
     and the fix. Typed errors at the library boundary are this project's
     rule. So:
     - `RetiredKeyReused { pubkey, as_device: String }`. The key being
       retired is offered back, as the enrolled device's key or as its own
       replacement (`as_device == RECOVERY_DEVICE_LABEL`), in
       `recover_device` and in `rotate_recovery_key`.
     - `WouldEvictActingDevice { vault }`: rotate's backstop.

     Under this choice, the rows that say "`RecipientExists`, `Conflict`"
     for these refusals (`recover_device_refuses_to_re_add_the_retired_key`,
     `rotate_recovery_key_refuses_the_key_it_is_retiring`,
     `rotate_refuses_to_evict_the_acting_device`) pin the new variant and
     `Conflict` instead. The exit code is unchanged. Keeping
     `RecipientExists` was the alternative, rejected in review.

   No variant's `Display` or `Debug` contains a secret. age's parse errors
   are dropped, not wrapped, because they can quote their input.
   `agekey::Error` quotes its input, but that input is always a public key
   from the spec, never the pasted secret.
4. **Where the code lives. Settled.**
   - Everything is in one file, `recovery/swap.rs`, like Go's
     `recovery_enroll.go`. That covers `recovery_identity`, the private
     `RecipientSwap` struct (`add`, `remove`, `remove_if_present`,
     `message`, and an optional `inspect` closure), `swap_recipients`,
     `apply_swap`, `check_recover_spec`, `recover_device` and
     `rotate_recovery_key`.
   - The tests are its child module (`#[cfg(test)] #[path =
     "swap_tests.rs"] mod tests;`, the `remotesync.rs` pattern). That lets
     them call the private `recovery_identity` and `apply_swap` without
     widening either. "Private" therefore means private to
     `recovery::swap`, which is narrower than `pub(crate)`.
   - `commit_recipient_list` widens to `pub(crate)`.
   - `unlock`'s page-lock warning moves into one `pub(crate)` helper in
     `vault.rs`, so the recovery identity's warning is the same sentence by
     construction, not by copy.
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

- [x] A private method, not `pub(crate)`. It runs these steps:
  1. Trim the secret, then `KeyLine::parse` it.
  2. Find its pubkey in `recipients()`. It must be the one labelled
     `RECOVERY_DEVICE_LABEL`.
  3. Build an `Identity` with device `RECOVERY_DEVICE_LABEL` through
     `Identity::new`. A page-lock failure warns through the prompter with
     `unlock`'s wording.
- [x] Rows:
  - `TestRecoveryIdentityAcceptsOnlyTheRecoveryRecipient`: `recovery_identity_accepts_only_the_recovery_recipient`
  - `TestRecoveryIdentityRefusesADeviceKey`: `recovery_identity_refuses_a_device_key`. `NotRecoveryKey` (`LockedOrAuth`), naming the device's label, even though the key would decrypt.
  - `TestRecoveryIdentityRefusesStrangersAndGarbage`: `recovery_identity_refuses_strangers_and_garbage`. A stranger gives `NotRecoveryKey`; garbage gives `MalformedRecoveryKey` (`Usage`).
  - `TestRecoveryIdentityToleratesPastedWhitespace`: `recovery_identity_tolerates_pasted_whitespace`
  - `TestRecoveryErrorsNeverQuoteTheSecret`: `recovery_errors_never_quote_the_secret`. Every refusal path in this milestone is formatted with `{}`, `{:?}` and `{:#}`, and none contains the secret or any 20-character run of it.

### 2. `swap_recipients` and `recover_device` — 10 rows

- [x] `swap_recipients`, under `with_vault_write`, in Go's order:
  1. `require_full_access`, always inside the lock (Go R3 decision 1)
  2. `require_recipients_in_sync`
  3. `confirm_recipient_trust`
  4. read the current list
  5. `inspect`
  6. `apply_swap`
  7. one future-only warning per label actually held, for both `remove` and `remove_if_present`
  8. `commit_recipient_list` with `RecipientWrite::message`
  9. `note_recipients_reviewed`
- [x] `apply_swap`, pure:
  - remove the labels
  - an unmatched `remove` label is `RecipientNotFound`
  - `remove_if_present` skips silently
  - add each recipient, after validating its name and key, against the
    **final** list (duplicate key or label is `RecipientExists`)
  - refuse an empty result (`LastRecipient`)
- [x] `recover_device`:
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
- [x] Rows:
  - `TestRecoverDeviceSwapsTheListInOneCommit`: `recover_device_swaps_the_list_in_one_commit`. The exact list is in both files, with exactly one new commit, and `replaces` removes the lost device in that same commit.
  - `TestRecoverDeviceRetiresTheUsedKeyForFutureWrites`: `recover_device_retires_the_used_key_for_future_writes`. The old key opens no entry written after the swap.
  - `TestRecoverDeviceLetsTheNewDeviceRead`: `recover_device_lets_the_new_device_read`. Every entry opens with the new device key and with the new recovery key.
  - `TestRecoverDeviceWithoutAReplacementLeavesNoRecoveryRecipient`: `recover_device_without_a_replacement_leaves_no_recovery_recipient`
  - `TestRecoverDeviceLeavesUpdatedByAlone`: `recover_device_leaves_updated_by_alone`
  - `TestRecoverDeviceRefusalsWriteNothing`: `recover_device_refusals_write_nothing`. One case per refusal: duplicate key, duplicate label, `replaces` naming an absent device, `replaces` naming the recovery label, and the reserved label as the device. Each pins its variant and code, HEAD unchanged and a clean tree.
  - `TestRecoverDeviceReusingTheReplacedName`: `recover_device_reusing_the_replaced_name`. `--device laptop-1 --replaces laptop-1` is accepted, because the check runs against the final list.
  - `TestRecoverDeviceRefusesToReAddTheRetiredKey`: `recover_device_refuses_to_re_add_the_retired_key`. The retired key is refused both as the device key and as the new recovery key (`RetiredKeyReused`, `Conflict`), with nothing written.
  - `TestRecoverDeviceStillAsksM10sQuestion`: `recover_device_still_asks_m10s_question`. With an unreviewed change in the trust cache, the question is asked; a declining prompter aborts with nothing written.
  - `TestApplySwapRefusesLeavingNothingToEncryptTo`: `apply_swap_refuses_leaving_nothing_to_encrypt_to`

### 3. The secret's lifetime — 1 row

- [x] Row:
  - `TestRecoverDeviceZeroesTheSecretItWasHanded`: `recover_device_takes_the_secret_by_value`, ported per decision 2.
    - A type assertion pins the parameter as `Zeroizing<String>`, by value: `let _: fn(&Vault, &RecoverSpec, Zeroizing<String>, &dyn Prompter) -> _ = Vault::recover_device;`.
    - A source scan of `recovery/` finds no `.clone()` or `to_string()` on the secret, and no `String` built from it outside `KeyLine::parse`. This is the same kind of scan as `crypt.rs`'s guard tests.
    - Go's row name is kept in the test's doc comment.

### 4. Atomicity — 1 row ⚑

- [x] Row:
  - `TestInterruptedRecoverDeviceLeavesHEADUntouched`: `interrupted_recover_device_leaves_head_untouched`.
    - A crash via `with_reencrypt_hook` after the first re-encrypted entry and before the commit.
    - After it: HEAD unchanged, both recipient files unchanged, the recovery key still listed and the new device not.
    - The next write's dirty-tree reset leaves a clean tree.
    - The pattern is `enrollment/approve_atomic_tests.rs`'s.

### 5. `rotate_recovery_key` — 8 rows

- [x] The swap with these settings:
  - add `recovery-paper-key → new_pubkey`
  - `remove_if_present: [RECOVERY_DEVICE_LABEL]`
  - message `gage: recovery rotate (reencrypt)`
  - an `inspect` that reads the retired pubkey under the lock, refuses
    `new_pubkey == retired`, refuses evicting the acting identity's own key
    (the backstop), and sets `created`
- [x] Rows:
  - `TestRotateRecoveryKeySwapsInOneCommit`: `rotate_recovery_key_swaps_in_one_commit`. The old key stops opening new ciphertext, and the new one opens everything.
  - `TestRotateRecoveryKeyCreatesOneWhenThereIsNone`: `rotate_recovery_key_creates_one_when_there_is_none`. `created: true`, `retired: None`.
  - `TestRotateRecoveryKeyRefusesTheKeyItIsRetiring`: `rotate_recovery_key_refuses_the_key_it_is_retiring`. `RetiredKeyReused`, `Conflict`.
  - `TestRotateRecoveryKeyRefusalsWriteNothing`: `rotate_recovery_key_refusals_write_nothing`
  - `TestInterruptedRotateRecoveryKeyLeavesHEADUntouched`: `interrupted_rotate_recovery_key_leaves_head_untouched` ⚑
  - `TestRotateRecoveryKeyStillAsksM10sQuestion`: `rotate_recovery_key_still_asks_m10s_question`
  - `TestRotateRefusesToEvictTheActingDevice`: `rotate_refuses_to_evict_the_acting_device`.
    - The state can't be reached through any reserved path, so the fixture hand-writes `config.toml` and `.age-recipients` with the owner's key under the label, then notes the list reviewed, as Go's test does.
    - The result is `WouldEvictActingDevice` (`Conflict`), with nothing written.
  - `TestRotateWarnsThatRetirementIsFutureOnly`: `rotate_warns_that_retirement_is_future_only`. The warning names the label, comes through `remove_if_present`, and is absent when there was nothing to retire.

### 6. Opus review ⚑

- [x] An Opus pass over tasks 1–5's tests, recorded here as a table: what
  it checked, and what it found.
  - Can any error carry the secret?
  - Does a crash at any point (first entry, last entry, before the commit)
    leave HEAD untouched?
  - Is the retired key blocked in both shapes?
  - Does a declined M10 question write nothing?
  - Is `recovery_identity` unreachable from outside the crate? Check this
    with a `trybuild`-free argument: it is private, and `Identity::new` is
    `pub(crate)`. Record the reasoning.

  Done on Opus 5.5, as a reading of the tests and the code plus a mutation
  pass: each guard in `swap.rs` was disabled in turn, and the suite was
  checked to see whether it fails.

  | Checked | How | Found |
  |---|---|---|
  | Can any error carry the secret? | Read every variant reachable from `recover_device`. The secret exists only as the `Zeroizing<String>` moved into `recovery_identity` and dropped right after `KeyLine::parse`. After that it lives only inside `Identity`, whose `Debug` is hand-written and redacted. No `RecoveryError` or `RecipientError` variant holds an `Identity` or a key line. age's parse error is mapped to `MalformedRecoveryKey` and dropped. `agekey::Error` quotes only spec pubkeys. `assert_never_quotes` checks `{}`, `{:?}` and `{:#}` for the whole secret and every 20-character run of it, over `recovery_identity`'s refusals (a stranger, a mangled key, a truncated key) and every refusal in both `recover_device` tables. | Nothing. |
  | Does a crash leave HEAD untouched? | `interrupted_recover_device_leaves_head_untouched` and `interrupted_rotate_recovery_key_leaves_head_untouched` crash after the first entry and after the last. Each checks HEAD and both committed recipient files, that the recovery key is still listed, the reset and clean tree on the next write, and the lock released. The recover test also checks that the new device is not listed and that a retry succeeds. The window after the last entry and before the commit, when the recipient files are written, has no hook. The existing seam was kept, as Q-CRASH-SEAM requires. That window is in the shared `commit_recipient_list`, unchanged here, and M9b's `interrupted_recipient_write_leaves_no_half_migrated_state` proves it. | Nothing new. The before-the-commit point is covered by M9b's test, not by a new hook. |
  | Is the retired key blocked in both shapes? | `recover_device_refuses_to_re_add_the_retired_key` covers it as the device key and as the replacement. `rotate_recovery_key_refuses_the_key_it_is_retiring` covers rotate. Mutations disabling each of the three checks were each caught. | Nothing. |
  | Does a declined M10 question write nothing? | Both `*_still_asks_m10s_question` rows check that the question was asked once, the `ChangeDeclined` variant, HEAD, the list and a clean tree. Removing `confirm_recipient_trust` from the swap fails both. | Nothing. |
  | Is `recovery_identity` unreachable from outside the crate? | It is a private method in `recovery::swap`, so it is narrower than `pub(crate)` and only that module and its test child can call it. `Identity` has only private fields, and its constructors are `Identity::new` (`pub(crate)`) and `for_tests` (`cfg(test)`). The public `recover_device` returns a `RecoverResult` with no `Identity` in it, and the identity is dropped before it returns. `rotate_recovery_key` takes an `Identity` and never makes one. So nothing outside the crate can obtain a recovery-scoped `Identity`. | Nothing. |
  | Mutation pass, beyond the five questions | 12 mutations: both retired-key checks, both rotate checks, the M10 question, `remove_if_present` in the warning loop, the unmatched-`remove` refusal, final-list versus current-list collisions, accepting a device key, the empty-list refusal, the review note, and `require_full_access`. | **One gap:** deleting the swap's `require_full_access` left every test green. Go has no row for it either. Fixed with a test beyond Go's rows, `both_swaps_refuse_an_actor_who_cannot_read_every_entry`. It covers both verbs: `CannotGrantFullAccess`, nothing written, no warning before the refusal. It fails under the mutation. |

  Beyond Go's rows, two tests were added:
  `recovery_identity_warns_like_unlock_when_the_page_lock_fails` (decision
  4's shared warning), and the full-access test above.

### 7. Parity and ledger

- [x] All 25 rows ported; none struck. 27 tests: the 25 rows, plus the two above.
- [x] Ledger §1 #16 and #18 noted `[~]`. #16's relative-path guard and #18's command are RK3's.
- [x] `make lint`, `make test` (886 library tests); CI on Linux, macOS and Windows (PR [#81](https://github.com/distillerylabs/gage-rs/pull/81), commit 03a6d02: lint 39070794101, test 39070794133 and audit 39070794083 all pass).

## Definition of done

- Every box above is checked and all 25 claimed rows are ported.
- RK1's and P1–P7's suites are still green.
- There is no exported way to obtain a recovery-scoped `Identity`.
- `make lint` and `make test` are clean locally and on all three CI platforms.
