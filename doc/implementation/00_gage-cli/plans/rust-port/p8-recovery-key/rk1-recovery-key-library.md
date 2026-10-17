# RK1 — The recovery key, library

[P8 plan](index.md) · [#75](https://github.com/distillerylabs/gage-rs/issues/75) · next: [RK2 — the recovery identity and the swap](rk2-recovery-swap-library.md)

> **Recommended model: Sonnet\* ⚑.** It is small, additive and follows
> `vault.rs`'s existing patterns, as Go's R0 recommendation says. The part
> to watch is the label reservation, which has to reach every path where a
> device names itself and none that writes the recovery recipient. Run an
> **Opus review of the duplicate-check scoping and the reservation rows**
> (task 5) before calling it done.

## Goal

Everything `crates/gage` needs for a recovery recipient, with no CLI change.
Nothing here prints, and nothing reaches `gage-cli` except new types.

1. Generate a recovery keypair.
2. Register it at vault creation under the fixed label, in the right order,
   refusing collisions that involve it.
3. Check a pasted secret against a vault's recipients and return the label
   it is registered under.
4. Reserve `recovery-paper-key` so no device can take it.

## Depends on

- **P7:** `Vault::enroll` (E3) and `add_recipient_locked`'s batch form
  (E1b, used by approval), both of which gain the reservation.
- **P1 · M1:** `CreateSpec`, `validate_fields`, `write_skeleton`.
- **P5 · M9a:** `Vault::add_identity`, `Vault::recipients`.
- **P2 · M2:** `agekey`, and `KeyLine::parse` for deriving a public key.

## Design references

- TDD "Recovery key at `init`": "The label is fixed … and reserved".
- TDD "Local identity storage": "Losing this file is a recovery problem, not
  a backup problem".

## Go sources

- Go plan `recovery-key/r0-library.md`, all of it. It records two details
  settled while implementing:
  - **duplicate checking is scoped to the extras**
  - **the error taxonomy reuses existing sentinels**
- Go plan `recovery-key/r4-recovery-enroll-cli.md`, "Settled while
  implementing": `VerifyRecoveryKey` returns the matched label.
- Go plan `recovery-key/r5-recovery-rotate-and-docs.md`, "Found by review",
  first bullet: the reservation, and why it isn't part of `devicename`.
- Parity ledger: §1 #7 and #8 (PR #13), and the R5 reservation half of #18
  (PR #22). §3 `recovery-key/`.
- Go sources at the pinned SHA:
  - `internal/gage/recovery_key.go`
  - `checkDeviceLabelFree` and `ErrReservedDeviceLabel` in
    `internal/gage/recovery_enroll.go`
  - `buildRecipients` in `vault_create.go`
- Test inventory, 11 rows:
  - `internal/gage/recovery_key_test.go` (10)
  - `internal/gage/recovery_enroll_test.go`
    `TestTheRecoveryLabelIsReservedForTheRecoveryKey` (1)

## Decisions to make first

**Resolved** at the start of RK1: the proposals below were accepted as written, with the additions marked *Settled while implementing*.

1. **Where the code lives.**
   - Proposed: a new `crates/gage/src/recovery/` module, shaped like
     `enrollment/`. `mod.rs` holds the label, the error type and the
     reservation check. `key.rs` holds `RecoveryKey`, `new_recovery_key`
     and `verify_recovery_key`.
   - RK2 adds `swap.rs` to the same module.
   - *Settled while implementing:* the test files sit beside the code they
     cover, one per task: `key_tests.rs` (task 1), `create_tests.rs`
     (task 2), `verify_tests.rs` (task 3), `reservation_tests.rs` (task 4).
     The approval rows live in `enrollment/reserved_tests.rs`, because the
     approval fixtures are private to that module.
2. **The error shape for the reservation.**
   - Go has one sentinel, `ErrReservedDeviceLabel` (`Usage`), returned from
     four functions.
   - Here each of those functions returns its own enum: `CreateError`,
     `IdentityAddError`, `EnrollmentError`, `RecipientError`.
   - Proposed: a small `recovery::ReservedLabel` error, which
     `check_device_label_free` returns. Each enum gets a
     `ReservedDeviceLabel(#[from] ReservedLabel)` variant mapped to `Usage`.
   - That keeps one wording, and lets every caller match its own enum.
   - *Settled while implementing:* `EnrollmentError::ReservedDeviceLabel` is
     also returned by `check_enrollment_labels`, the approval pre-check, so
     a frontend learns before it renders or unlocks. The authoritative
     refusal for approval is still `add_recipient_locked`, surfaced as
     `EnrollmentError::Recipient(RecipientError::ReservedDeviceLabel)`.
     `Vault::add_recipient` also checks up front, ahead of its pre-flight.
3. **`RecoveryKey`'s shape.**
   - Proposed:
     `pub struct RecoveryKey { pub pubkey: String, pub secret: Zeroizing<String> }`
     ([Q-RECOVERY-SECRET-MEMORY](../open-questions.md#q-recovery-secret-memory)).
   - age's `Identity::to_string()` returns a `SecretString`. Its exposed
     value is copied into the `Zeroizing<String>`, and the `SecretString`
     is dropped, which zeroes it.
   - `Debug` is hand-written so it redacts `secret`.
4. **`verify_recovery_key`'s signature.**
   - Proposed:
     `Vault::verify_recovery_key(&self, secret: &str) -> Result<String, RecoveryError>`.
     It returns the matched label.
   - `RecoveryError` has two variants:
     - `MalformedRecoveryKey` (`Usage`): the text never contains the input,
       and the parse error is dropped, not wrapped.
     - `NotARecipient { pubkey, vault }` (`LockedOrAuth`).
   - It trims surrounding whitespace, and it neither retains nor zeroes the
     input. That asymmetry with RK2's `recover_device` is documented on
     both.
5. **Which collisions `create` refuses.** Go's R0, as settled:
   - **only when extras are present, and only collisions that involve an
     extra**
   - the errors: duplicate pubkey, duplicate label, an extra whose label
     equals the device's, and an extra that fails `agekey` validation
   - a duplicate is `RecipientExists`-shaped (`Conflict`), not `Usage`
   - a repeated `--recipient` stays legal
   - *Settled while implementing:* an extra's label is also checked against
     the `recipient-<n>` labels `write_skeleton` is about to generate, so an
     extra named `recipient-2` can't produce two recipients with one label.
     An invalid extra label is `InvalidDeviceName` (`Usage`).
   - The check runs against all of `recipients`, including index 0. Inside
     `create` that is the device's key, which is the one check that needs
     it. `init`'s earlier `validate_without_device_key` sees the flag keys
     only, so everything but that check still fails before the passphrase
     prompt.

## Tasks

Write each task's tests first. Library rows go in
`crates/gage/src/recovery/key_tests.rs`.

### 1. The keypair — 4 rows

- [x] `RECOVERY_DEVICE_LABEL = "recovery-paper-key"`, exported.
- [x] `new_recovery_key() -> Result<RecoveryKey, RecoveryError>`.
  - It does no I/O.
  - This is the library's second `x25519::Identity::generate` site,
    alongside `identityfile::create`. No guard counts those sites, and none
    is added.
- [x] Rows (`recovery_key_test.go`):
  - `TestNewRecoveryKeyReturnsAValidRecipient`: `new_recovery_key_returns_a_valid_recipient` (`agekey` accepts the pubkey)
  - `TestNewRecoveryKeySecretParsesBackToTheSamePubkey`: `new_recovery_key_secret_parses_back_to_the_same_pubkey`
  - `TestNewRecoveryKeyIsDifferentEachCall`: `new_recovery_key_is_different_each_call`
  - `TestZeroingRecoverySecretLeavesPubkeyIntact`: `zeroing_recovery_secret_leaves_pubkey_intact`. Zeroize `secret` in place; `pubkey` is unchanged.
- [x] Beyond Go: `recovery_key_debug_redacts_the_secret`.

### 2. Labelled extras at `create` — 5 rows

- [x] `CreateSpec.extra_recipients: Vec<LabelledRecipient { device, pubkey }>`.
  - `write_skeleton` labels the device first, then the extras with their
    own labels, then `recipient-N` for `recipients`.
  - Both files are written in the one initial commit.
- [x] `validate_fields` applies decision 5's collision rules. It runs inside
  `validate_without_device_key` where it can, so `init` refuses before the
  passphrase prompt.
  - The one check that needs the device key is "an extra's pubkey equals
    the device's". It runs in `create`.
- [x] A refused `create` leaves no vault directory.
- [x] Rows:
  - `TestCreateWritesLabelledExtraRecipientToBothFilesInOneCommit`: `create_writes_labelled_extra_recipient_to_both_files_in_one_commit`
  - `TestCreateOrdersDeviceThenLabelledExtrasThenGenericRecipients`: `create_orders_device_then_labelled_extras_then_generic_recipients`. Extras don't shift the `recipient-N` numbering. As in Go, N is the position in `recipients`, with the device key at index 0. So device + recovery + one `--recipient` gives `laptop-1`, `recovery-paper-key`, `recipient-2`, in that order, in both files.
  - `TestCreateRejectsDuplicateOrInvalidExtraRecipients`: `create_rejects_duplicate_or_invalid_extra_recipients`. Each case pins its variant and exit code, and that no directory is left behind.
  - `TestCreateWithoutExtraRecipientsStillAllowsRepeatedRecipientKeys`: `create_without_extra_recipients_still_allows_repeated_recipient_keys`. `create_repeated_recipients_all_written` stays green and unmodified.
  - `TestEntryEncryptedToTheVaultDecryptsWithTheRecoverySecretAlone`: `entry_encrypted_to_the_vault_decrypts_with_the_recovery_secret_alone`. It decrypts with an `age` identity parsed from `secret`, not through gage's unlock.

### 3. `verify_recovery_key` — 1 row

- [x] The method, per decision 4. It reads only the plaintext recipient
  list: no unlock, no decryption, no lock.
- [x] Row:
  - `TestVerifyRecoveryKey`: `verify_recovery_key`, with these cases:
    - the right key returns `"recovery-paper-key"`
    - a device's key returns that device's label
    - a valid stranger gives `NotARecipient` (`LockedOrAuth`)
    - garbage gives `MalformedRecoveryKey` (`Usage`)
    - surrounding whitespace is tolerated

    Neither error's `Display` contains the input.

### 4. The reservation — 1 row

- [x] `check_device_label_free(device)`, called in:
  - `CreateSpec::validate_fields`, for the device, unconditionally: even
    with no recovery key, because rotate can add one later
  - `Vault::add_identity`
  - `Vault::enroll`
  - `add_recipient_locked`, for each recipient in the batch. That covers
    `recipient add` and approval, including approval's `--device` relabel.
- [x] It is **not** added to `devicename::valid`. The recovery recipient
  itself is written under the label, by `create`'s extras and by RK2's
  swap. Neither goes through the paths above.
- [x] Row:
  - `TestTheRecoveryLabelIsReservedForTheRecoveryKey`: `the_recovery_label_is_reserved_for_the_recovery_key`, with Go's four subtests:
    - `create` refuses it even with no recovery key, leaving no directory
    - `add_identity` refuses it
    - `add_recipient` refuses it
    - the recovery recipient is still written by `create`'s extras
- [x] Beyond Go, because the TDD names these paths and Go's test does not
  cover them:
  - `enroll_refuses_the_recovery_label`
  - `approval_refuses_a_request_that_claims_the_recovery_label`
  - `approval_refuses_a_relabel_to_the_recovery_label`

  Each pins `Usage` and that nothing was committed.

### 5. Opus review ⚑

- [x] An Opus pass over tasks 2 and 4's tests and code.

| Checked | Found |
|---|---|
| A repeated `--recipient` stays legal, with and without extras; only collisions involving an extra are refused | Fine. Tests cover both. |
| Every device-naming path is reserved, including approval's relabel and `check_enrollment_labels` | Fine. Grep for `devicename::valid`, `identityfile::create` and `Recipient {` found no other writer. |
| No path that writes the recovery recipient trips the check | Fine. `create`'s extras never call it. |
| `verify_recovery_key` and secret handling | Fine. Debug redacts, the parse error is dropped, no Display echoes the input. |
| Pre-key `recipient-N` label range was off by one before the device key exists | **Fixed.** `validate_extras` takes `has_device_key`; test `a_recipient_n_label_collision_fails_before_the_device_key_exists`. |
| A key collision named the extra's label, not the holder's | **Fixed.** Test `a_key_collision_names_the_label_that_holds_the_key`. |
| `gage clone --device recovery-paper-key` is accepted (registers the reserved name; later `enroll` and `add_identity` refuse) | **Open, for RK3.** `gage-cli`, outside RK1's scope; decide whether to check it there. |
| `recipient approve --device recovery-paper-key` is refused only after codes are opened | **Open, for RK3/RK4.** Still before the confirmation and unlock; the CLI's early check could call `check_device_label_free`. |

### 6. Parity and ledger

- [x] All 11 rows ported (10 from `recovery_key_test.go`, 1 reservation row), plus the beyond-Go rows.
- [x] Ledger §1 #8 marked `[x]`. #7 is left `[ ]` until RK4.
- [x] `make lint`, `make test`; CI on Linux, macOS and Windows (PR [#80](https://github.com/distillerylabs/gage-rs/pull/80), run 39067750078: lint, test and audit all pass).

## Definition of done

- Every box above is checked.
- All 11 claimed rows are ported.
- P1–P7's suites are still green, with `create_repeated_recipients_all_written` unmodified.
- No printing has been added to `crates/gage`.
- `make lint` and `make test` are clean locally and on all three CI platforms.
