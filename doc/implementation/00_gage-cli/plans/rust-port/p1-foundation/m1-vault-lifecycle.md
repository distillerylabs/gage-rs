# M1 — Vault lifecycle and vault-id keying

[← M0b](m0b-primitives.md) · [P1 plan](index.md) · next: P2 (planned when it starts)

> **Recommended model: Sonnet.** Config schemas, validation and vault CRUD,
> with every decision recorded below. **Run an Opus review pass over the
> untrusted-input tests** (task 2's id and device-name rejection, task 6's
> tampered-config and mismatched-registration cases): they decide whether a
> committed, git-writable file can steer a filesystem path, and the
> dangerous cases are the ones a passing suite is least likely to construct.

## Goal

A vault's on-disk structure and the global vault registry, **with no crypto
anywhere**. `gage init` creates the directory, writes `.gage/config.toml`
(`format_version = 2`, a fresh `[vault].id`, `type = "git"`), writes
`.age-recipients` from public keys given as strings, writes `.gitignore`
and `.gitattributes`, makes one commit, and registers the vault (with its
id) in global config. Alongside: `vault list/info/remove/set-default` and
`git set-remote`. Ends with the P1 user docs.

`init` requires `--recipient` in this milestone, and has no `--remote`
flag ([Q-INIT-INTERIM](../open-questions.md#q-init-interim)). M2 makes
`--recipient` additive by generating this device's identity.

## Depends on

- **M0b**: exit codes, `atomicfile`, `xdgpaths`, `devicename`, vault-id
  validation, `paths`, global `config`, `gittest`, the git crate, the
  registry and help, `Prompter`.

## Design references

- Design doc "Vault types", "On-disk layout", "Global config",
  "Git-specific commands", "Local identity storage" (the untrusted-input
  rule).
- Go open questions Q-DEVICE-NAME, Q-METHOD-SCOPE, Q-METHOD-FLAG,
  Q-SYNC-CONFLICT (why `.gitattributes` marks the recipient files
  `-merge`), Q-IDENTITY-VAULT-NAME and A20 (vault-id keying).
- [Q-COMPAT](../open-questions.md#q-compat): the schema follows Go, the
  bytes are gage-rs's own.

## Go sources

- Go plans `gage-cli-design/m1-vault-lifecycle.md` and
  `gage-cli-init-design/e0-vault-id-keying.md` (the keying half, the
  two-ids comparison, the directional version message; the
  orphaned-identity half is P6 · M12).
- Parity ledger rows: Go plan §3 M1 (+ E0); E0 `0d710d4` (M1's half);
  user docs `vaults.md`, `configuration.md`, `exit-codes.md` (§6).
- Go sources at the pinned SHA: `internal/gage/vault_create.go`,
  `internal/gage/vault.go`, `internal/gage/vaultconfig/`,
  `internal/gage/recipients/` (the writer only), `internal/gage/agekey/`
  (recipient validation only), `internal/gage/gitrepo/gitrepo.go`
  (`InitAndCommit`, `SetRemote`, `RemoteURL`, `IsClean`),
  `cmd/gage/{init,vault,git,globalconfig}.go`.
- Test inventory: the 89 P1 rows of `vaultconfig` (12), `init` (4),
  `vault_create` (23), `vault_lifecycle` (29) and `vaultid_keying` (21),
  plus **7 rows claimed from P4's `gitrepo_test.go`** that test the git
  surface M1 builds (task 3). Every row is listed below as ported,
  partially ported with the rest named, or moved.

## Decisions to make first

Recommended answers; confirm or change them in review before task 1.

- **`init` into an already-registered name** fails with Conflict and leaves
  the existing vault's files and registration untouched (Go's behavior).
- **`vault info` with no argument** reports on `current`; with no current
  vault it fails with Usage ("no vault name given and no current vault is
  set"), as Go.
- **Recipient labels**: the first `--recipient` key is labelled with the
  device name, later ones `recipient-<n>` (n from 2), as Go's
  `buildRecipients`. Duplicate keys fail with Conflict.
- **Git defaults**: initial branch `main`; author and committer
  `gage <gage@localhost>`; message `gage: initialize vault`. Set explicitly
  so the user's `init.defaultBranch` and `user.*` never apply.
  ([Q-COMPAT](../open-questions.md#q-compat) frees the branch name from
  go-git's `master`.)
- **`created`** is the UTC date (`YYYY-MM-DD`) as a string, as Go.
- **File modes** (Unix): `entries/` `0700`, `.gage/` `0750`,
  `.gage/config.toml`, `.age-recipients`, `.gitignore`, `.gitattributes`
  `0644`; global config `0600`. Windows: default ACLs, no assertions on
  modes.
- **Version refusal is directional**: older than this build → re-create
  the vault (naming `make reset-local-state`); newer → upgrade gage. A
  missing `format_version` is treated as older.
- **Which command proves "refused on read"**: `vault info`, the only
  read command in P1. Later read commands inherit the check by going
  through the same vault resolver, which is what makes this enough.

## Tasks

Tests first, at task granularity.

### 1. Vault config — `vaultconfig` (12 rows)

- [ ] `TestRoundTrip`, `TestReadMissingFile`, `TestCurrentFormatVersionIsTwo`,
      `TestRecipientsCarryOnlyDeviceAndPubkey` (no per-device `method`
      leaks into the committed file, Q-METHOD-SCOPE),
      `TestWriteWrapsAnAtomicfileFailure`.
- [ ] Version: `TestReadRejectsUnrecognizedFormatVersion` (newer → upgrade),
      `TestReadRefusesAVaultOlderThanThisBuildWithARecreateMessage`,
      `TestReadRejectsMissingFormatVersion`, `TestFormatVersionCheckedBeforeTheID`,
      `TestFormatVersionCheckedBeforeDeviceNames`.
- [ ] Untrusted input ⚑: `TestReadRejectsAnIDThatIsNotAUUID`,
      `TestReadRejectsPathTraversalDeviceNameBeforeUse` (`../../../etc/x`,
      an absolute path, a separator: rejected, never normalized).
- [ ] Schema: `[vault]` `name`, `id`, `type`, `format_version`, `created`;
      `[method]` `default` (+ optional `plugin`); `[[recipients]]`
      `device`, `pubkey`. Read checks run in Go's order: version, id,
      device names.

### 2. Recipients and validation

No rows of their own (the Go tests live in P5's `recipients` package);
needed by task 4.

- [ ] Recipient validation with the `age` crate
      ([Q-AGE-CRATE](../open-questions.md#q-age-crate)): a well-formed
      `age1…` X25519 key passes; malformed bech32, wrong HRP, and an
      `AGE-SECRET-KEY-1…` identity are rejected with Usage.
- [ ] `.age-recipients` writer: one key per line, no framing, trailing
      newline. Test: the file parses with the `age` crate's recipients-file
      reader, i.e. a stock `age -R` could use it.

### 3. Git surface — 7 rows claimed from P4's `gitrepo_test.go`

- [ ] `TestInitAndCommitCreatesExactlyOneCommit`,
      `TestSetRemoteThenChangeIt`, `TestRemoteURLEmptyWhenUnset`,
      `TestRemoteURLAndHasRemoteWithNoConfiguredURLs`,
      `TestHasRemoteOnARepoWithNoOriginConfigured`,
      `TestIsCleanDetectsDirtyWorkingTree`,
      `TestIsCleanCountsUntrackedFilesAsDirty`.
- [ ] `gitrepo::{init_and_commit, set_remote, remote_url, is_clean}` on
      the crate chosen in M0b, with the git defaults above. Test: commit
      author, committer and branch are gage's even when `HOME` holds a
      `.gitconfig` setting `init.defaultBranch` and `user.name`.
- [ ] Not claimed: `TestRequireCleanWorkTreeTruncatesTheNamedPaths` and
      `worktree_state_test.go` stay in P4 (sync preconditions).

### 4. `Vault::create` — `vault_create` (23 rows)

- [ ] Skeleton: `TestCreateWritesFullSkeleton`, `TestCreateCommitsTheSkeletonFiles`,
      `TestCreateGitattributesMarksRecipientFilesUnmergeable`
      (`.age-recipients -merge` and `.gage/config.toml -merge`),
      `TestCreateGitignoreHasOnlyOSCruft` (`.DS_Store`, `Thumbs.db`),
      `TestCreateVaultConfigRoundTrip`, `TestCreateRecipientsMatchAgeRecipientsFile`,
      `TestCreateRepeatedRecipientsAllWritten`.
- [ ] Id: `TestCreateRecordsTheIDItWasHandedRatherThanMintingOne`,
      `TestCreateRefusesAnIDThatIsNotAUUID`.
- [ ] Validation before any file is touched: `TestCreateRejectsEmptyName`,
      `TestCreateRejectsEmptyPath`, `TestCreateRejectsUnknownType`,
      `TestCreateRejectsUnknownMethod` (including `ssh`, `yubikey`,
      `age-key`, `secure-enclave`, `plugin:x`), `TestCreateRejectsInvalidDeviceName`,
      `TestCreateRejectsMalformedRecipient`, `TestCreateRequiresAtLeastOneRecipient`,
      `TestCreateAcceptsExplicitGitType`, `TestCreateAcceptsExplicitPassphraseMethod`.
- [ ] Target: `TestCreateIntoNonEmptyDirectoryFailsWithoutTouchingFiles`,
      `TestCreateRejectsATargetThatIsARegularFile` (both Conflict).
- [ ] Remote: `TestCreateWithRemoteSetsOrigin`,
      `TestCreateWithoutRemoteLeavesVaultRemoteless` (library only; no CLI
      flag yet).
- [ ] Partial: `TestCreateDeviceNameLandsInRecipientAndIdentityPath`:
      the recipient-label half here; **the identity-path half moves to
      P2 · M2**.
- [ ] Not ported here: Go's reserved recovery label check
      (`checkDeviceLabelFree`) arrives with **P8**.

### 5. `gage init` — `init` (4) and 16 `vault_lifecycle` rows

- [ ] Device name: `TestResolveDeviceNameExplicitOverridesHostname`,
      `TestResolveDeviceNameExplicitFailingAllowlistIsUsageError`,
      `TestResolveDeviceNameFallsBackToNormalizedHostname`,
      `TestResolveDeviceNameHostnameNormalizesToNothingPromptsRatherThanInventing`
      (as Go now behaves: fails with Usage naming `--device`, never invents
      a name).
- [ ] `TestInitCreatesFullSkeletonAndRegisters`,
      `TestInitWithoutDirUsesGageDataVaultsDir`, `TestInitWithDirUsesGivenPath`,
      `TestInitDefaultsLandInVaultConfigAndGlobalConfig`,
      `TestInitExplicitTypeAndMethodEquivalentToDefault`,
      `TestInitUnknownTypeFailsWithUsageAndListsGit`,
      `TestInitUnknownMethodFailsWithUsage`,
      `TestInitMalformedRecipientFailsBeforeCreatingFiles`,
      `TestInitRepeatedRecipientWritesAll`,
      `TestInitDeviceFlagOverridesHostnameDefault`,
      `TestInitDeviceFlagFailingAllowlistRejectedBeforeCreatingFiles`,
      `TestInitRecordsThisDevicesDeviceAndMethod`,
      `TestInitIntoAlreadyRegisteredNameFailsAndLeavesExistingVaultUntouched`,
      `TestInitIntoNonEmptyNonGageDirectoryFailsWithoutTouchingFiles`.
      All run against `--recipient`.
- [ ] Partial: `TestInitDeviceFlagAgreesEverywhere`: global config and
      `[[recipients]]` here; **the identity-file half moves to P2 · M2**.
- [ ] Partial: `TestReinitAfterVaultRemoveIsAnOrdinaryRetry`: re-init
      succeeds and gets a new id here; **the identity-file halves move to
      P2 · M2**.
- [ ] Interim (no Go row; Go M1's own bullet): `init` with no
      `--recipient` fails with Usage naming the flag. **M2 rewrites this
      test**, as Go's M2 did.
- [ ] Moved to **P2 · M2**: `TestInitWithNoRecipientGeneratesThisDevicesIdentity`,
      `TestInitWritesTheWrappedIdentityFile`,
      `TestInitExtraRecipientFollowsTheDeviceKey`.
- [ ] Vault id via `init` (`vaultid_keying`): `TestInitMintsADistinctUUIDv4PerVault`,
      `TestInitWritesFormatVersionTwo`.
- [ ] Register `init` (one-shot only) in the registry with an accurate
      help text for what it does in M1; add its cases to the
      `errorprefix` table.

### 6. Vault commands, resolution and the id check — 7 `vault_lifecycle` rows, 5 `vaultid_keying` rows

- [ ] A single vault resolver used by every command that reads a vault:
      global config entry → the vault's `.gage/config.toml` → compare ids.
      If the vault's config is unreadable, global config's id stands alone
      (only for commands that don't need the vault's contents, i.e.
      `vault remove`).
- [ ] `vault list`, `vault info [name]`, `vault remove <name>`,
      `vault set-default <name>`, `git set-remote <name> <url>`, registered
      with their parents (`vault`, `git`); the M0b parent-command help
      tests re-run against these real parents.
- [ ] `TestVaultListIncludesFreshlyInitedVault`,
      `TestVaultInfoByNameReportsExpectedFields` (type, method, recipient
      count, remote, clean/dirty), `TestVaultInfoNoArgumentReportsOnCurrent`,
      `TestVaultInfoReportsDirtyWorkingTree`,
      `TestVaultRemoveDropsRegistrationButLeavesFiles`.
- [ ] Untrusted input through a real command ⚑:
      `TestTamperedDeviceNameRefusedByRealCommand`,
      `TestTamperedFormatVersionRefusedByRealCommand`,
      `TestAVaultFromBeforeThisChangeIsRefusedWithARecreateMessage`,
      `TestAnUnsafeCommittedIDIsRejectedBeforeAnyPathIsBuilt` (hand-edited
      committed id with traversal, separator, absolute path),
      `TestAMismatchedRegistrationIsRefusedRatherThanSilentlyPreferred`
      (message names re-registration).
- [ ] Partial: `TestAnUnreadableVaultFallsBackToGlobalConfigsIDAlone`:
      `vault remove` succeeds and drops the registration when the vault's
      files are gone; **the identity-file and stderr-names-the-identities-dir
      halves move to P6 · M12** (orphan cleanup).
- [ ] Partial: `TestAVaultWithoutAnIDFailsLoudlyOnEveryDerivedPath`: a
      registration with an empty id fails `vault info` and every path
      function; **the `show`/`insert`/`identity list` halves move to
      P2 · M4 and P5** as those commands land.
- [ ] `git set-remote` (Go M1 bullets, no P1 Go row; `TestSetRemoteThenChangeIt`
      covers the library): sets `origin` and `[vaults.<name>.git].origin`
      in one call; changing it updates both and `vault info` shows the new
      URL; missing either argument is a Usage error with no change made.
- [ ] `vault set-default` (Go M1 bullet): updates `current`; an unknown
      name fails with NotFound and changes nothing.
- [ ] Every error path added here gets its row in the `errorprefix` table.

### 7. Rows moved out of P1

Recorded here so each has an owner; the destination milestone's doc claims
it when that phase is planned.

| Row | Destination | Reason |
|---|---|---|
| `vault_lifecycle` `TestVaultRemoveDeletesAnOrphanedIdentityFile`, `TestVaultRemoveKeepsIdentityFileStillListedAsRecipient`, `TestVaultRemoveKeepsIdentityFileWhenRecipientsUnreadable` | P6 · M12 | Orphaned-identity cleanup (#39 as revised by E0) |
| `vaultid_keying` `TestNonInteractiveVaultRemoveNeverDeletesAnIdentity`, `TestVaultRemoveIdentifiesAStaleKeyWhoseNameNowLabelsAnother`, `TestVaultRemoveKeepsARelabelledRecipientsKey`, `TestVaultRemoveKeepsTheFileWhenNoPubkeyIsRecorded`, `TestVaultRemoveNeverDeletesAKeyBelongingToADifferentVault`, `TestVaultRemoveNeverDeletesWithoutConfirming`, `TestVaultRemoveOnAPendingEnrollmentAsksBeforeDeleting`, `TestVaultRemoveLeavesTheVaultsOwnFilesUntouched`, `TestOneRepositoryRegisteredTwiceSharesOneIdentitiesDirectory` | P6 · M12 | Same; they need identities and `vault remove`'s cleanup |
| `vaultid_keying` `TestTheIDInitFilesTheIdentityUnderIsTheIDItCommits` | P2 · M2 | Needs identity generation |
| `vaultid_keying` `TestTwoRegistrationsOfOneRepositoryShareOneLock` | P2 · M4 | Proven through `insert` contending; the shared lock *path* is covered by M0b's `paths` tests |
| `vaultid_keying` `TestCloneCopiesTheVaultsIDIntoGlobalConfig` | P4 · M8a | Needs `clone` |
| `vaultid_keying` `TestInitAndIdentityAddRecordThePubkeyAndCloneDoesNot` | P5 (init half in P2 · M2) | Needs `identity add` and `clone` |
| `vaultid_keying` `TestTwoVaultsUnderOneLocalNameShareNeitherDirectory` | P5 · M10 | Needs `clone` and the trust cache |

### 8. User docs and ledger

- [ ] Port Go's `doc/cli/vaults.md`, `configuration.md` and
      `exit-codes.md` for the surface that exists, describing shipped
      behavior (including the interim `--recipient` requirement).
- [ ] Parity ledger: P1 rows' status updated; §5's coarse-spots note gains
      the 7 `gitrepo` rows claimed here.

## Definition of done

Every box above checked; all 89 P1 rows plus the 7 claimed `gitrepo` rows
ported, or partially ported with the remainder named in this doc, or moved
in task 7's table; the Opus review of the untrusted-input tests done;
`make lint` and `make test` clean locally and on Linux, macOS and Windows
CI. You can create, list, inspect, re-point and forget vaults; nothing in
them is encrypted yet.
