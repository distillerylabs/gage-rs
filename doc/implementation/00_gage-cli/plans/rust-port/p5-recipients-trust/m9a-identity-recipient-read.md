# M9a — Identities & recipient read side

[← P4 · M8b](../p4-sync/m8b-conflict-resolution.md) · [P5 plan](index.md) · [#42](https://github.com/distillerylabs/gage-rs/issues/42) · next: [M9b](m9b-recipient-write.md)

> **Recommended model: Sonnet.** Additive and pattern-following. It
> reuses M2's identity creation, M1's `vaultconfig` and `recipients`, and
> the registry and parity-test patterns from P3/P4. Nothing here writes to
> the vault's git history. The one piece of design is the two-device
> fixture (task 1), which every later P5 milestone builds on, so get its
> shape reviewed before task 2.

## Goal

A second device can register its own identity for a vault, and anyone can
inspect a vault's recipients without unlocking it:

- `gage identity add [--device NAME] [--method M]` registers this
  device's identity under the vault id and prints its public key, plus the
  next command to run on a device that already has access. Each device
  gets its own wrapped identity file.
- `gage identity list` reports the identities this machine holds for a
  vault.
- `gage recipient list` prints every recipient's device name and key,
  exactly as `.gage/config.toml` lists them.
- `gage recipient verify` exits 0 with "in sync" when `.age-recipients`
  and `config.toml` agree, and exits 1 with the specific differences when
  they don't. It needs no identity file and no prompter.

## Depends on

- **P2 · M2:**
  - `identityfile::create` (`identityfile.rs`), which writes the wrapped
    file, its `vault-name.txt` marker and the reuse warning
  - `paths::identities_dir` / `identity_file_path`
  - `vault::ALLOWED_METHODS`
- **P1 · M1:**
  - `vaultconfig::{read, Recipient}`
  - `recipients::read`
  - `config::VaultEntry` (`device`, `method`, `pubkey`)
  - `devicename`
- **P4 · M8a2:**
  - `clone`, which registers a vault with an empty `pubkey`
  - `remotesync_tests::SyncVault`, the base of the two-device fixture
- **P3 · M13:** `TestSubcommandCandidatesListsIdentityChildren`, written
  against the `vault` parent until `identity` existed.

## Design references

- Design doc, "Recipient / access management": the command surface.
- Design doc, "Local identity storage": why losing an identity file is a
  recovery problem, not a backup problem.
- Design doc, "`identity` vs `recipient` stay separate".
- Go open question Q-METHOD-SCOPE: `--method` sets *this device's*
  method, recorded locally, never in the vault.

## Go sources

- Go plan `gage-cli-design/m9-recipients.md`: the Identity bullets, the
  `verify` bullets, `recipient list`, and the `AddIdentity` /
  `ListIdentities` / `Recipients` / `VerifyRecipients` surface.
- Parity ledger rows:
  - §3 M9 (with M9b)
  - §4 `identity`, `identity_add`, `identity_list`, `recipient`,
    `recipient_list`, `recipient_verify` (`--repair` is M10's)
- Go sources at the pinned SHA:
  - `internal/gage/identity_add.go` (less `checkDeviceLabelFree`, which
    is P8)
  - `internal/gage/recipient.go`: `Recipients`, `VerifyRecipients`,
    `RecipientVerification`
  - `internal/gage/recipients/recipients.go`
  - `cmd/gage/identity.go`
  - `cmd/gage/recipient.go`: `list` and `verify` without `--repair`
- Test inventory, 26 rows:
  - `internal/gage/identity_add_test.go` (6 of 7)
  - `cmd/gage/identity_cli_test.go` (7)
  - `internal/gage/recipients/recipients_test.go` (6)
  - `internal/gage/recipient_test.go` (4 of 21)
  - `cmd/gage/recipient_cli_test.go` (3 of 15)
- Inbound rows: see task 6.

## Decisions to make first

Resolved when implementation started (reviewed, "proceed as recommended"):

- The library surface is as proposed below, with `VaultRecipient`
  replaced by `vaultconfig::Recipient`. `identity_add.rs` and
  `recipient.rs` hold the new `Vault` methods; errors are
  `IdentityAddError`, `ListIdentitiesError` and `VerifyError`.
  `RecipientVerification` compares key sets, so order and repeats do not
  matter.
- Global config is written by the CLI only.
- The two-device fixture is `twodevice_tests.rs` (`Machine`,
  `edit_recipients_file`, `edit_config_recipients`, over `SyncVault` as
  device A) and, in `gage-cli`, `identity_cli_tests::TwoDevices` (two
  `Env`s over one bare origin).
- `recipients::read` has no line-length cap: the Go row exists only
  because `bufio.Scanner` caps a token at 64 KiB. It is ported as
  `read_returns_an_overlong_line_whole`, which pins that nothing is
  silently truncated.
- `recipient verify`'s long help carries only its first line until M10:
  the rest of Go's text documents `--repair`, which does not exist yet.

The original proposals, for reference:

- **The library surface.** Proposed:
  - `Vault::add_identity(&self, device: &str, prompter: &dyn Prompter) -> Result<String, IdentityAddError>`.
    It checks that the device name is valid and isn't already a
    `[[recipients]]` label, then calls `identityfile::create`.
  - `Vault::list_identities(&self) -> Result<Vec<LocalIdentity>, _>`,
    where `LocalIdentity { device, path, current }`. It skips
    subdirectories and file names that aren't valid device names.
  - `Vault::recipients(&self) -> Result<Vec<VaultRecipient>, _>`.
  - `Vault::verify_recipients(&self) -> Result<RecipientVerification, _>`,
    where `RecipientVerification { in_sync, only_in_recipients_file, only_in_config }`.
  - The typed error has a `DeviceNameTaken` variant that maps to
    Conflict, as Go's `ErrDeviceNameTaken` does.
  - `VaultRecipient` either stays a separate type or reuses
    `vaultconfig::Recipient`. Prefer the reuse unless a field differs.
- **Where the global-config write happens.** As in Go and in `init`: the
  CLI records `device`, `method` and `pubkey` in `config::VaultEntry`.
  The library never writes global config.
- **The two-device fixture.** Proposed: a library test module (e.g.
  `crates/gage/src/twodevice_tests.rs`, or an extension of
  `remotesync_tests::SyncVault`) that gives two gage installs. Each has
  its own `Dirs`, its own registration and its own identity file, over one
  bare origin. Helpers:
  - `a.vault()` and `b.vault()`
  - `a.unlock()`
  - `b.identity_add(device)`, which returns the pubkey
  - a hand-edit helper for `.age-recipients` and `config.toml`, for the
    divergence tests in M9a, M9b and M10

  `gage-cli` has an `Env`-based equivalent for the CLI rows that need two
  devices. Its shape must suit M9b's re-encryption and M10's pulled
  recipient change, so review it with those in mind.

## Tasks

Write each task's tests first, at task granularity.

### 1. The two-device fixture

- [x] Build it as above. One smoke test: device A inits and pushes,
      device B clones and runs `add_identity`, and B has its own file
      under `$GAGE_DATA/identities/<vault-id>/` while A's is unaffected.

### 2. The `recipients` file — 6 rows

`recipients::read` and `write` exist from M1. Port Go's tests against them
and fix whatever they find.

- [x] `TestReadMissingFile`
- [x] `TestReadRejectsALineLongerThanTheScannerCanBuffer`: Go's
      `bufio.Scanner` limit has no Rust equivalent. Decide whether
      `read` caps a line, and record the decision here, or excuse the row
      with the reason.
- [x] `TestReadSkipsBlankLines`
- [x] `TestRoundTrip`
- [x] `TestWriteHasNoFramingOnePerLine`
- [x] `TestWriteWrapsAnAtomicfileFailure`

### 3. `recipients` and `verify_recipients` (library) — 4 rows

- [x] `TestRecipientListMatchesConfigExactly`
- [x] `TestVerifyReportsInSyncAndTheSpecificDifferences`, in both
      directions: a key only in `.age-recipients`, and a key only in
      `config.toml`.
- [x] `TestVerifyNeedsNoIdentityFileOrPrompter`: a registration with no
      identity file at all, as right after `clone`.
- [x] `TestVerifyReadsNeitherIdentityNorPrompter`. The signature takes no
      `Prompter` and no `Identity`, so the compiler already proves part
      of this. Port it as the no-identity-file case, and note that in the
      test.

### 4. `add_identity` and `list_identities` (library) — 6 rows

- [x] `TestAddIdentityWritesItsOwnWrappedFileAndReturnsAPublicKey`
- [x] `TestEachDeviceHasItsOwnIdentityFile`: deleting one device's file
      leaves the other able to decrypt.
- [x] `TestAddIdentityRejectsANameAlreadyRegisteredAsARecipient`:
      `DeviceNameTaken`, with no file written.
- [x] `TestAddIdentityRejectsAnInvalidDeviceName`
- [x] `TestListIdentitiesReportsThisDevicesRegisteredIdentities`:
      `current` marks the registered device.
- [x] `TestListIdentitiesSkipsSubdirectoriesAndInvalidlyNamedFiles`
- `TestRecoveringALostIdentityFileNeedsNoFileRestore` is M9b's, because
  it needs `recipient add`.

### 5. CLI: `identity` and `recipient list/verify` — 10 rows

- [x] Register the `identity` and `recipient` parents (`ParentInfo`,
      groups Identity and Recipients), plus `identity add`,
      `identity list`, `recipient list` and `recipient verify`, all
      available in both modes. Each gets the three `commands/mod.rs`
      edits. Use the fixture's `Short` and long help (`identity add`'s
      long help names `gage recipient add <pubkey> --device NAME`
      without `--reencrypt`).
- [x] `identity add`:
  - `--device` defaults to the normalized hostname
  - `--method` defaults to the vault's `[method].default` and is checked
    against `ALLOWED_METHODS`
  - records `device`, `method` and `pubkey` in global config, never in
    the vault
  - prints the public key and the next command
    `gage recipient add <pubkey> --device <device>`. M9b proves that
    command works.
- [x] Rows:
  - `TestIdentityAddRegistersADeviceAndPrintsItsPublicKey`
  - `TestIdentityAddDeviceFlagOverridesTheHostnameDefault`
  - `TestIdentityAddMethodDefaultsToTheVaultsAndStaysLocal`
  - `TestIdentityAddRejectsAHostnameCollisionWithoutAnyoneTypingTheName`
  - `TestIdentityAddRejectsANameTheVaultAlreadyLists`
  - `TestIdentityListReportsThisDevicesIdentities`
  - `TestIdentityListOnADeviceHoldingNoneSaysSo`
  - `TestRecipientVerifyExitCodesAndDifferences`: 0 and 1, by the
    taxonomy.
  - `TestRecipientVerifyNamesAKeyMissingFromTheRecipientsFile`
  - `TestRecipientVerifyRunsWithNoIdentityAndNoPrompter`: run through
    `gage_cli::run` with a prompter that panics on any question.
- [x] Add `p5_identity_commands_have_their_go_fixture_flags_and_descriptions`
      for `identity`, `identity add`, `identity list`, `recipient`,
      `recipient list` and `recipient verify` (flags as they exist:
      `--repair` is M10's). `identity enroll` and the enrollment children
      of `recipient` are P7's, so the parent-help check lists only the
      children that exist.

### 6. Inbound rows

- [x] `vaultid_keying` `TestInitAndIdentityAddRecordThePubkeyAndCloneDoesNot`.
      The `init` half is already done (P2 · M2). Add the `identity add`
      half (the pubkey is recorded) and the `clone` half (it is not).
- [x] The `identity list` half of
      `TestAVaultWithoutAnIDFailsLoudlyOnEveryDerivedPath`: a registration
      with an empty id fails before any path is built.
- [x] M13's `TestSubcommandCandidatesListsIdentityChildren`: add the
      `identity` form now that the parent has children, and record the
      substitution in [M13's doc](../p3-query-session/m13-tab-completion.md).

### 7. Ledger

- [x] Parity ledger:
  - §4: the `identity*`, `recipient`, `recipient_list` and
    `recipient_verify` fixtures are matched
  - §3 M9: `[~]`
- [x] Mark M9a `[x]` in the [P5 index](index.md) after CI.

## Definition of done

Every box above is checked, and all 26 claimed rows plus the three inbound
ones are ported or excused in writing. `make lint` and `make test` are
clean locally and on Linux, macOS and Windows CI.

The milestone is done when two simulated devices share a vault's remote,
the second has registered its own identity and printed its key, and
`recipient verify` reports a hand-edited `.age-recipients` with exit 1
without any identity or prompter.
