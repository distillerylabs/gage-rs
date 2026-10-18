# RK4 — `recovery enroll`, `recovery verify`, hints, docs

[← RK3](rk3-init-and-rotate.md) · [P8 plan](index.md) · [#78](https://github.com/distillerylabs/gage-rs/issues/78) · next: P9 Parity sign-off

> **Recommended model: Opus ⚑.** This is ordering and failure handling
> around a key that can read everything: the same class of risk as RK3.
> Go's R4 review found seven bugs after the milestone was first reported
> done, including:
> - a restore that could never restore
> - a replacement key discarded when registration failed
> - a hint pointing at a command that didn't exist
> - a task ticked but never implemented
>
> Run an **Opus review of the step-ordering, backup-restore and
> delivery-before-registration rows** (task 7) before calling it done.

## Goal

### `gage recovery enroll`

`gage recovery enroll` takes a user to a working device identity. The user
has lost their only identity file, or forgotten its passphrase, and holds
the paper recovery key. The command retires the used key and puts a fresh
recovery key in hand. It runs in this order, so nothing irreversible
happens before it must:

1. **Refuse everything that needs no secret.**
   - Resolve the vault without unlocking it, and resolve the device name.
   - Plan the new key with `plan_recovery_key("--no-new-recovery-key")`.
     That applies the terminal rule. In file mode, check the destination
     against RK3's guard.
   - Refuse the reserved label, with that as the reason given.
   - Refuse a name a recipient already uses, unless it is the
     `--replaces` target.
   - Refuse an existing local identity file for the device, without asking
     for its passphrase (`LocalIdentityExists`, `Usage`). The exception is
     `--replaces` naming that same device: then the file is backed up in
     memory, to be replaced.
2. **Paste the recovery key** through the masked `Prompter::value`. Check it
   with `verify_recovery_key`, and require the `recovery-paper-key` label. A
   wrong paste costs no passphrase entry.
3. **Generate the replacement recovery key,** unless
   `--no-new-recovery-key` was given.
4. **Clear the backed-up file, if any, then create the device identity.**
   This is the new-passphrase prompt. On failure, restore the backup.
5. **`recover_device`.** On failure, remove the new identity and restore the
   backup.
6. **Deliver the new key, with any error held.** This happens *before*
   anything that can still fail. The new key is committed and pushed by
   now, and its private half exists only in this process.
7. **`record_local_identity`.**
8. **The summary,** and the no-recovery-key warning when there is no new
   key. Then return. A delivery error outranks a registration error. A
   registration error names `gage identity add --device <name>` as the
   remedy.

### `gage recovery verify`

`gage recovery verify` checks a pasted key with no unlock, and reports two
separate claims:

- **"decrypts this vault":** true for any recipient's key, so it exits 0
- **"`recovery enroll` will accept it":** true only for the
  `recovery-paper-key` one

### The rest

- **The `recovery enroll` hints:**
  - `clone`'s access message
  - the missing-identity message that `unlock` and `use` surface
  - `identity add`'s long help
- **P8's user docs.**

## Depends on

- **RK1:** `verify_recovery_key` and the reservation.
- **RK2:** `recover_device`, `RecoverSpec`, and the errors.
- **RK3:**
  - `commands/recoverykey.rs`: `plan_recovery_key`, the destination guard,
    delivery, the re-type
  - the `recovery` parent
  - `init_vault_with_recovery_key_file`
  - `Io.stderr_is_terminal`
- **P5 · M9a:** `record_local_identity`; `identityfile::create` and
  `identityfile::remove`.
- **P4 · M8a2 / P7 · E3:** `clone`'s `access_lines`.

## Design references

- TDD "Recovery key at `init`": the bullets on `recovery verify`,
  `recovery enroll`, "Retirement is future-only" and "An `age-key` unlock
  method is deliberately not built".
- Go plan `recovery-key/index.md`: D8–D10, D12, and "Design facts Phase 2
  relies on", including R1's lessons applied again.

## Go sources

- Go plan `recovery-key/r4-recovery-enroll-cli.md`, all of it:
  - the two resolved decisions: verify's two claims, and overwriting only
    when `--replaces` names the same device
  - "Settled while implementing":
    - `plan_recovery_key` takes the opt-out flag's name
    - verify returns the label
    - the in-memory backup
    - `ErrLocalIdentityExists` lives in the CLI
  - the seven "Found by review" fixes, each carried here as a test
- Go plan `recovery-key/r2-verify-and-docs.md`, all of it, including the
  limit it recorded. That limit is now closed by enroll: the docs say what
  the key can and can't do.
- Go plan `recovery-key/r5-recovery-rotate-and-docs.md`: the docs tasks,
  and "The R4 stopgap wording is gone".
- Parity ledger:
  - §1 #7, #10 (PR #15), #17 (PR #21)
  - §4 `recovery`, `recovery_enroll`, `recovery_verify`
  - §6 `identities-and-recipients.md` (P8's recovery sections) and
    `vaults.md`
- Go sources at the pinned SHA:
  - `cmd/gage/recovery.go`: enroll and verify
  - `cmd/gage/clone.go`, `cmd/gage/repl.go`: the hints
  - Go's `doc/cli/identities-and-recipients.md` ("Storing your recovery
    key", "Using it", "If it leaks", "Skipping it") and `doc/cli/vaults.md`
- Help fixtures: `recovery.txt`, `recovery_enroll.txt`,
  `recovery_verify.txt`.
- Test inventory, 26 rows:
  - `cmd/gage/recovery_enroll_test.go`, 19 rows: the 16
    `TestRecoveryEnroll*` rows, `TestCloneAndSessionPointAtRecoveryEnroll`,
    `TestNoRecoveryKeyWarningNamesARealCommand`, and
    `TestRecoveryVerifyDistinguishesDecryptingFromEnrolling`
  - `cmd/gage/recovery_test.go` (7)
  - plus the `recovery enroll` subtest of RK3's
    `no_command_lets_a_device_take_the_recovery_label`
- Inbound: the hints, the `recovery` parent's full child list, the
  identity-add help and the docs (see the
  [index](index.md#inventory-accounting)).

## Decisions to make first

Settle these in this doc, and get them reviewed, before writing code.
*Status: resolved as below at the start of RK4; reviewed.*

1. **Who owns the identity-file backup.**
   ([Q-RECOVERY-IDENTITY-BACKUP](../open-questions.md#q-recovery-identity-backup))
   - Go's `identityBackup` reads the file in `cmd/gage` and restores it
     with `MkdirAll` plus `atomicfile`. Its review bug was the missing
     `MkdirAll`.
   - In gage-rs, `identityfile` owns the 0700 directory and 0600 file
     rules (`make_private_dir`, private).
   - **Resolved:** a small library pair in `identityfile`:
     - `backup(dirs, vault_id, device) -> Result<Option<IdentityBackup>, UnlockError>`
       reads the wrapped file into a `Zeroizing<Vec<u8>>`, plus the
       marker's bytes if a marker is there. `None` when there is no file.
       It validates both path components as `create` does, and only reads.
     - `IdentityBackup::restore(&self) -> Result<(), UnlockError>` (no
       `dirs`: the backup already holds its path)
       recreates the directory through `make_private_dir` (0700), writes
       the file back with `atomicfile::write` (0600), and rewrites the
       marker from the bytes it held. Go's restore didn't put the marker
       back. Restoring "exactly what was there" includes it, and it costs
       nothing. `restore` takes `&self` so a caller can report a failed
       restore without losing the bytes.
     - Clearing the displaced file is the existing `identityfile::remove`.
       It prunes the marker and the empty directory, which is why
       `restore` has to recreate both.
   - The *policy* stays in `gage-cli`, as Go settled: refuse unless
     `--replaces` names the device. The library never decides to overwrite.
     `backup` doesn't take a "replacing" flag. The CLI calls it only after
     deciding to replace, and does its own existence check first for the
     refusal.
2. **How `LocalIdentityExists` is matchable.**
   ([Q-RECOVERY-LOCAL-IDENTITY-ERROR](../open-questions.md#q-recovery-local-identity-error))
   - Go's test checks `errors.Is` against an exported sentinel. Go's review
     bug was formatting it with `%v`.
   - **Resolved:** a typed `LocalIdentityExists { path, device }`
     (`thiserror`) in `commands/recovery.rs`, implementing `HasCode` as
     `Usage`. It then converts through `CliError`'s existing `From`, and
     `anyhow` keeps the concrete type.
   - `CliError` gains one method, `downcast_ref::<E>() -> Option<&E>`. It
     returns the first `E` in the error's cause chain, so a typed error
     under added context is still found (Go's `errors.Is`). Nothing else
     about `CliError` changes. `thiserror` moves from a dev-dependency of
     `gage-cli` to a normal one. It is already in the lockfile through the
     library.
   - The name-collision refusal works the same way: a `DeviceNameTaken`
     wrapper whose source is the recipient verbs' own
     `RecipientError::RecipientExists`, plus the `--replaces` remedy. A
     test matches the `RecipientExists` through `downcast_ref`.
   - Tests reach the error through `Env::dispatch`, which runs the private
     `crate::dispatch` with the same `Io` that `Env::run_io` builds.
   - The message is Go's: the path, then "if this machine is being rebuilt
     under that name, say so with `--replaces <device>`; otherwise enroll
     under a different `--device NAME`".
3. **Verify's output and exit codes**, per Go's resolved decision:
   - exit 0 for any recipient's key. The first line names the vault and the
     label the key is registered under.
   - on `recovery-paper-key`: "it can decrypt this vault, and
     `gage recovery enroll` will accept it"
   - on any other label: "it can decrypt this vault, but
     `gage recovery enroll` will not accept it — that needs the key
     registered as `recovery-paper-key`" (Go's wording)
   - not a recipient: `LockedOrAuth`; malformed: `Usage`. Both come from
     `RecoveryError`'s existing codes, unchanged.
   - the pasted value is the `Zeroizing<String>` that `Prompter::value`
     returns, which is masked. It is never echoed, and it is dropped when
     the check returns.
   - The lines go to stdout, as Go's `writeOut(app.Out, …)` does. The
     prompt is "Paste the recovery key to check", given as `value`'s
     title.
4. **How the registration-failure row fails registration.** (The plan
   said "unreadable or replaced by a directory". Go's test shows that
   doesn't work, and why.)
   - Every step before registration reads the global config: `open_vault`
     resolves the vault from it. A config that can't be read would refuse
     at step 1, and the row would prove nothing.
   - **Resolved, as Go does it:** the global config *directory* is chmodded
     to 0500 after `init`. Reads still work. `config::save`'s atomic write,
     which creates a temp file beside the target, is the first thing that
     fails. Nothing between it and step 1 writes the config dir: identity
     files go under `$GAGE_DATA`, and the trust cache goes under
     `$GAGE_STATE`.
   - It is `#[cfg(unix)]` and skipped when running as root, as Go's is.
     Windows directory ACLs don't enforce this the same way. The ordering
     it proves (deliver at step 6, before registration at step 7) is
     platform-independent code, so Linux and macOS CI cover it.
   - No new seam, per CLAUDE.md: the real filesystem produces the real
     failure.

## Tasks

Write each task's tests first. CLI rows go in
`crates/gage-cli/src/recovery_cli_tests.rs`. Hint rows go in
`clone_enroll_tests.rs` and `repl_tests.rs`, beside the hints they pin.

### 1. `recovery verify` — 8 rows

- [x] Registry: `recovery verify`, group Recipients, one-shot only, with
  `-u/--use`. Long help from `recovery_verify.txt`.
- [x] The command, per decision 3.
- [x] Rows (`recovery_test.go`, and one from `recovery_enroll_test.go`):
  - `TestRecoveryVerifyAcceptsTheVaultsRecoveryKey`: `recovery_verify_accepts_the_vaults_recovery_key`
  - `TestRecoveryVerifyToleratesPastedWhitespace`: `recovery_verify_tolerates_pasted_whitespace`
  - `TestRecoveryVerifyNeedsNoUnlock`: `recovery_verify_needs_no_unlock`. No unlock request is recorded, and no lock is taken.
  - `TestRecoveryVerifyRejectsAKeyThatIsNotARecipient`: `recovery_verify_rejects_a_key_that_is_not_a_recipient`. `LockedOrAuth`.
  - `TestRecoveryVerifyRejectsAMalformedKey`: `recovery_verify_rejects_a_malformed_key`. `Usage`.
  - `TestRecoveryVerifyNeverEchoesTheKey`: `recovery_verify_never_echoes_the_key`
  - `TestRecoveryVerifyIsListedInHelp`: `recovery_verify_is_listed_in_help`
  - `TestRecoveryVerifyDistinguishesDecryptingFromEnrolling`: `recovery_verify_distinguishes_decrypting_from_enrolling`. Three cases:
    - the recovery key says both
    - another recipient's key is honest about both, and names its label
    - enroll really does refuse that key

### 2. `recovery enroll`: refusals before the paste — 5 rows

- [x] Registry: `recovery enroll`, group Recipients, one-shot only. Flags
  `--device`, `--replaces`, `--no-new-recovery-key`, `--recovery-key-out`,
  `-u/--use`, with the help text from `recovery_enroll.txt`.
- [x] `identityfile::backup` and `restore` (decision 1), with library tests:
  - `backup_of_a_missing_file_is_none`
  - `restore_recreates_a_pruned_identity_directory`. This is Go's review
    bug, tested where the fix lives.
- [x] Steps 1–2 of the goal's order.
- [x] Rows:
  - `TestRecoveryEnrollWithoutATerminalRefusesBeforeAnything`: `recovery_enroll_without_a_terminal_refuses_before_anything`. `Usage`, naming `--recovery-key-out` and `--no-new-recovery-key`. No prompt is asked.
  - `TestRecoveryEnrollRefusesACollidingNameBeforeThePaste`: `recovery_enroll_refuses_a_colliding_name_before_the_paste`. `RecipientExists` (`Conflict`). No value prompt, no passphrase.
  - `TestRecoveryEnrollLocalIdentityErrorIsMatchable`: `recovery_enroll_local_identity_error_is_matchable`. The error downcasts to `LocalIdentityExists`, its exit is `Usage`, and the identity's passphrase is never asked for.
  - `TestRecoveryEnrollChecksTheKeyBeforeTheNewPassphrase`: `recovery_enroll_checks_the_key_before_the_new_passphrase`. A wrong paste is refused with no `Purpose::Create` request recorded.
  - `TestRecoveryEnrollIsListedInHelp`: `recovery_enroll_is_listed_in_help`
- [x] RK3's `no_command_lets_a_device_take_the_recovery_label` gains its
  `recovery enroll` subtest. `Usage`, with the reservation as the reason
  given.

### 3. `recovery enroll`: the swap and its rollback — 7 rows ⚑

- [x] Steps 3–5, with the identity removed and the backup restored on every
  failure after step 4. Steps 6–8 landed with them, since the success rows
  here need a delivered key and a registered device. Task 4's rows pin
  their order.
- [x] Rows:
  - `TestRecoveryEnrollGetsAWorkingDeviceBack`: `recovery_enroll_gets_a_working_device_back`. End to end:
    1. `init --recovery-key-out`, then `insert`
    2. lose the identity file
    3. `recovery enroll`

    The new passphrase opens the vault. The old key is no longer a recipient, and the new one is. The newly shown key decrypts an entry.
  - `TestRecoveryEnrollWorksOnAFreshClone`: `recovery_enroll_works_on_a_fresh_clone`. A fresh XDG root plus `clone`.
  - `TestRecoveryEnrollRebuildsADeviceWhosePassphraseIsForgotten`: `recovery_enroll_rebuilds_a_device_whose_passphrase_is_forgotten`. `--device laptop-1 --replaces laptop-1` with the file still present. The old passphrase is never asked for.
  - `TestRecoveryEnrollReplacesADifferentLostDevice`: `recovery_enroll_replaces_a_different_lost_device`. `--replaces` removes the old device in the same commit.
  - `TestRecoveryEnrollRollsBackTheIdentityWhenTheSwapFails`: `recovery_enroll_rolls_back_the_identity_when_the_swap_fails`. No orphan identity, no registration change, no commit.
  - `TestRecoveryEnrollRestoresTheIdentityItDisplacedWhenAborted`: `recovery_enroll_restores_the_identity_it_displaced_when_aborted`.
    - The identity is kept, not deleted first. Go's review noted that every earlier rollback test deleted it first, so the restore never ran.
    - The abort happens at the new-passphrase prompt.
    - The original bytes are back, the passphrase still opens them, and the vault's only identity directory is recreated.
  - Beyond Go: `recovery_enroll_restores_the_displaced_identity_when_the_swap_fails`. The other path to the restore: step 4 succeeds over the displaced file, and then the swap refuses (the two recipient files disagree). The new identity is removed, the old one is restored byte for byte, and nothing is committed. Without this row, dropping the restore from step 5's failure path broke no test.
  - `TestRecoveryEnrollNoNewRecoveryKeyLeavesNone`: `recovery_enroll_no_new_recovery_key_leaves_none`. No key output, the warning is printed, and no `recovery-paper-key` recipient remains.

### 4. `recovery enroll`: delivery, registration, hygiene — 5 rows ⚑

- [x] Steps 6–8.
- [x] Rows:
  - `TestRecoveryEnrollShowsTheNewKeyEvenIfRegistrationFails`: `recovery_enroll_shows_the_new_key_even_if_registration_fails`.
    - Registration is made to fail through the global config directory, chmodded 0500 (decision 4). Unix only; skipped as root. No new seam is added.
    - The key was delivered, and the error names `gage identity add --device`.
  - `TestRecoveryEnrollStillPublishesWhenTheConfirmationFails`: `recovery_enroll_still_publishes_when_the_confirmation_fails`. `Conflict`; the remote has the commit; the key is not reprinted.
  - `TestRecoveryEnrollKeyOutWritesAPrivateFile`: `recovery_enroll_key_out_writes_a_private_file`
  - `TestRecoveryEnrollKeepsNoCopyOfEitherKey`: `recovery_enroll_keeps_no_copy_of_either_key`.
    - Neither key appears under any XDG root, in the vault tree, or in any git object.
    - The pasted key appears in the output zero times. The new key appears exactly once.
  - `TestNoRecoveryKeyWarningNamesARealCommand`: `no_recovery_key_warning_names_a_real_command`. Every `gage <words>` in the warning resolves in the registry.

### 5. The hints — 1 row

- [x] `clone`'s `access_lines` gain the `gage recovery enroll` line: it admits this device and retires the key it used. `assert_access_lines` is updated.
- [x] `vault.rs`'s missing-identity message adds "or `gage recovery enroll` if you hold its recovery key". This is the message `unlock` and the session's `use` surface.
- [x] `identity add`'s long help no longer calls it "the recovery path for a lost identity file". It points at `recovery enroll` for a vault this device can't otherwise get back into. This deviates from Go's prose at the pin ([Q-RECOVERY-IDENTITY-ADD-HELP](../open-questions.md#q-recovery-identity-add-help)). The P5 help parity test exempts that one sentence by name and checks the replacement instead.
- [x] Row:
  - `TestCloneAndSessionPointAtRecoveryEnroll`: `clone_and_session_point_at_recovery_enroll`. Both `clone`'s message and `use`'s message name it.

### 6. Help parity and docs

- [x] `p8_recovery_commands_have_their_go_fixture_flags_and_descriptions`
  covers all three `recovery` children and the parent's complete child
  list, against `recovery*.txt`.
- [x] `doc/cli/identities-and-recipients.md`: replace the "Not yet in
  gage-rs" note with a "Storing your recovery key" section, ported from
  Go's and covering what gage-rs ships:
  - the custody rules
  - "Using it" (`recovery verify`, `recovery enroll`, and the honest
    statement that it is not an unlock method)
  - "If it leaks" (`recovery rotate`, and that retirement is future-only,
    so rotate the secrets too)
  - "Skipping it" (`--no-recovery-key`, and `rotate` to add one later)
  - the reserved label
- [x] `doc/cli/vaults.md`: `init`'s synopsis and the recovery-key bullets,
  covering both flags, the terminal rule and the reserved label.
- [x] `doc/cli/exit-codes.md`: add the `recovery` commands' meanings where
  that page lists per-command meanings. There are no new codes.
- [x] `doc/cli/README.md` index, if it lists sections.

### 7. Opus review ⚑

- [x] An Opus pass over tasks 2–5's tests, recorded here as a table: what
  it checked, and what it found.
  - Does every refusal precede the paste, and the paste precede the
    passphrase?
  - Does the restore run, and succeed, when the displaced file was the
    vault's only identity?
  - Is the new key delivered before `record_local_identity` can fail?
  - Does each hint name a registered command?
  - Is every ticked box implemented? Go's R4 had a ticked hint task that
    wasn't, so this check walks the boxes against the diff.

Every ⚑ row was also mutation-checked: the code it guards was broken on
purpose, the row went red, and the code was restored.

| Checked | How | Found | Outcome |
|---|---|---|---|
| Refusals before the paste, paste before the passphrase | Read `enroll`'s step 1 against every refusal the library can make without a secret. Rows assert no value prompt and no `Create` request. | `--replaces recovery-paper-key` is knowable from the flags, but only the swap refused it, after a new passphrase (Go has the same order). A `--replaces` naming nobody is refused late too. | The first is fixed: refused in step 1 (`recovery_enroll_refuses_replacing_the_recovery_key_before_the_paste`), and the swap still refuses it as a backstop. The second is kept as Go has it. Go's rollback row relies on it, and only the swap's check, under the lock, is authoritative about who is listed. |
| The restore runs and succeeds when the displaced file was the vault's only identity | `restore_recreates_a_pruned_identity_directory` (library). `…_restores_the_identity_it_displaced_when_aborted` covers an abort at step 4. Each was mutation-checked by dropping `restore_after` on its path. | Neither Go row reaches the restore on step 5's path (the swap fails after the new identity exists). Dropping it there broke no test. | Added `recovery_enroll_restores_the_displaced_identity_when_the_swap_fails`. Mutation-checked. |
| The new key is delivered before `record_local_identity` can fail | `…_shows_the_new_key_even_if_registration_fails`, mutation-checked by registering first with `?`. | Step 8's precedence (a delivery error outranks a registration error) had no row. When both failed, the registration error was printed without its remedy. | The remedy is attached once, so both paths carry it. Added `recovery_enroll_reports_both_when_delivery_and_registration_fail` (Unix). |
| Each hint names a registered command | The hint rows matched text only. | Nothing named a missing command, but nothing checked it either, except RK3's own messages. | `assert_names_real_commands` resolves every `` `gage …` `` against the registry. It is now used by the clone and `use` hint row, the no-recovery-key warning and the registration remedy. |
| Every ticked box implemented | Walked the boxes against the diff. Mapped all 26 claimed inventory rows to a Rust test citing its Go row (19 + 7, by name). Checked the reservation subtest. | The common rebuild, a new machine under the lost one's name with the file gone, had no row (Go's rebuild row keeps the file). | Added `recovery_enroll_rebuilds_under_the_same_name_after_losing_the_file`. |
| Secret handling | Read every path the pasted and new keys take. | The paste is a `Zeroizing<String>`, moved into `recover_device` or dropped on refusal. The new key is dropped after delivery. No error message carries either. The keep-no-copy row walks the XDG roots and git objects. | Nothing to change. |

Accepted, as in Go: a process killed between clearing the displaced file
and restoring it loses that file. It exists only in memory by then. It is
only ever cleared when `--replaces` names this device, and that is the
forgotten-passphrase case, where the file is by premise unopenable.

### 8. Parity and ledger

- [x] All 26 rows ported, or struck with a reason in this doc. All 26
  ported, none struck. Five rows beyond Go came out of task 7. One
  deliberate prose deviation:
  [Q-RECOVERY-IDENTITY-ADD-HELP](../open-questions.md#q-recovery-identity-add-help).
- [x] Ledger:
  - §1 #7, #10 and #17 `[x]`
  - §3 `recovery-key/` `[x]`
  - §4 `recovery`, `recovery_enroll` and `recovery_verify` `[x]`
  - §6 P8's rows `[x]`
- [x] Port-plan P8 row and this index: status `[x]`.
- [x] `make lint` (with `lint-selftest`) and `make test` (1,666 tests); CI on
  Linux, macOS and Windows (PR [#83](https://github.com/distillerylabs/gage-rs/pull/83),
  commit 88cce28: lint 39103984672, test 39103984658 and audit 39103984666
  all pass, first run).

### Walkthrough

The real binary (`make build`), in a scratch XDG root, at the shipped
scrypt work factor, with stdin piped (one answer per line):

1. `init personal --device laptop-1 --recovery-key-out rk`, then
   `insert api-token --value-stdin`: exit 0 for both.
2. `rm -rf $XDG_DATA_HOME/gage/identities`. `show api-token` exits 5 with
   the missing-identity message, which now ends "or `gage recovery enroll`
   if you hold its recovery key".
3. `recovery enroll --device laptop-2 --recovery-key-out rk2`, pasting
   `rk`: exit 0. It printed "enrolled "laptop-2"", "retired the recovery
   key you used (age1cc5s…)", "new recovery key is age15ay7…" and
   "re-encrypted 1 entry".
4. `show api-token` with the new passphrase: `s3cr3t`, exit 0.
5. `recovery verify` with `rk`: "that key (age1cc5s…) is not a recipient",
   exit 5. With `rk2`: "registered as "recovery-paper-key"" and "…
   `gage recovery enroll` will accept it", exit 0.

`recipient list` afterwards: `laptop-1`, `laptop-2`, `recovery-paper-key`
(the new key).

Seen on the way, and not changed here: the terminal prompter frames every
`Prompter::value` title as `Value for "…": `. So the paste reads `Value
for "Paste the recovery key for personal": `, as RK3's re-type prompt
already did. Fixing that needs a prompt method of its own, which changes
the `Prompter` trait, so it is left for a follow-up rather than folded in
here.

## Definition of done

Every box above is checked, and all 26 claimed rows are ported. RK1–RK3's
and P1–P7's suites are still green. `make lint` and `make test` are clean
locally and on all three CI platforms.

A recorded walkthrough of the real binary in a scratch XDG root shows the
whole recovery path works, as Go's R4 did:
1. `init --recovery-key-out`, then `insert` an entry.
2. Delete the identity file.
3. `recovery enroll`.
4. `show` works with the new passphrase.
5. `recovery verify` refuses the retired key, and accepts the new one as
   enroll-capable.
