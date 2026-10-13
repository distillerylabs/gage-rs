# M1 — Vault lifecycle and vault-id keying

[← M0b](m0b-primitives.md) · [P1 plan](index.md) · next: [P2 · M2](../p2-crypto-entries/m2-identity-crypto.md)

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

Resolved. The recommended answers below were confirmed when the plan was
reviewed and merged (PR #8), with two amendments made during
implementation because the recommendation contradicted Go's behavior,
which wins ([Q-COMPAT](../open-questions.md#q-compat)): repeated
recipients, and recipient validation (both marked *Amended*).

- **`init` into an already-registered name** fails with Conflict and leaves
  the existing vault's files and registration untouched (Go's behavior).
- **`vault info` with no argument** reports on `current`; with no current
  vault it fails with Usage ("no vault name given and no current vault is
  set"), as Go.
- **Recipient labels**: the first `--recipient` key is labelled with the
  device name, later ones `recipient-<n>` (n from 2, by position), as Go's
  `buildRecipients`. *Amended:* repeated keys are written as given, not
  refused. Go's `TestCreateRepeatedRecipientsAllWritten` writes a key
  repeated three times; its duplicate-key Conflict applies only to the
  labelled extra recipients (the recovery key), which arrive with P8.
- **Recipient validation** *(amended)*: Go's shape-only Bech32 check
  (`agekey`), ported as is, rather than the `age` crate's X25519 parser.
  Go deliberately accepts plugin recipients (`age1yubikey1…`) so a vault
  shared with a device using one round-trips, and Go's own Create tests use
  one. [Q-AGE-CRATE](../open-questions.md#q-age-crate) records the change.
  So the `age` crate is not an M1 dependency, and the "a stock `age -R`
  could use it" test asserts the recipients-file grammar directly; the
  round-trip through the `age` crate moves to P2 · M2, which adds the crate
  (and settles its dependencies' licenses under `deny.toml`).
- **Git defaults**: initial branch `main`; author and committer
  `gage <gage@localhost>`; message `gage: initialize vault`. Set explicitly
  so the user's `init.defaultBranch` and `user.*` never apply.
  ([Q-COMPAT](../open-questions.md#q-compat) frees the branch name from
  go-git's `master`.)
- **`created`** is the UTC date (`YYYY-MM-DD`) as a string, as Go.
- **File modes** (Unix): `entries/` `0700`, `.gage/` `0750`,
  `.gage/config.toml`, `.age-recipients`, `.gitignore`, `.gitattributes`
  `0644`; global config `0600` and its directory `0700`. A vault directory
  `init` creates is `0700`, as Go's `MkdirAll` leaves it. Directory modes
  are set explicitly, so the umask can't change them. Windows: default
  ACLs, no assertions on modes.
- **Version refusal is directional**: older than this build → re-create
  the vault (naming `make reset-local-state`); newer → upgrade gage. A
  missing `format_version` is treated as older.
- **Which command proves "refused on read"**: `vault info`, the only
  read command in P1. Later read commands inherit the check by going
  through the same vault resolver, which is what makes this enough.
- **Exit codes for a refused vault config** (Go's mapping): an unsupported
  `format_version`, an invalid committed id or device name, and a
  mismatched registration are Conflict; an unreadable or malformed file is
  Internal.
- **Where the logic lives**: the library holds the registry operations
  (`config::Global::{register, entry, name_or_current, unregister,
  set_default}`, `config::{load, save}`), the resolver
  (`Vault::from_registration`) and `Vault::info`, so a GUI gets the same
  behavior. `gage-cli` keeps what Go keeps in `cmd/gage`: flag parsing,
  hostname lookup, `resolve_device_name`, and rendering.
- **Injected seams**: `Io` gains `dirs` and `hostname` functions, so the
  CLI tests run in-process against a temp directory and a chosen hostname.
  Go sets `XDG_*` in the test process, which parallel Rust test threads
  can't share safely.
- **`git set-remote` and `vault remove`** don't go through the resolver,
  as in Go: neither reads the vault's config, and `vault remove` must work
  when the vault is gone.

## Tasks

Tests first, at task granularity.

### 1. Vault config — `vaultconfig` (12 rows)

- [x] `TestRoundTrip`, `TestReadMissingFile`, `TestCurrentFormatVersionIsTwo`,
      `TestRecipientsCarryOnlyDeviceAndPubkey` (no per-device `method`
      leaks into the committed file, Q-METHOD-SCOPE),
      `TestWriteWrapsAnAtomicfileFailure`.
- [x] Version: `TestReadRejectsUnrecognizedFormatVersion` (newer → upgrade),
      `TestReadRefusesAVaultOlderThanThisBuildWithARecreateMessage`,
      `TestReadRejectsMissingFormatVersion`, `TestFormatVersionCheckedBeforeTheID`,
      `TestFormatVersionCheckedBeforeDeviceNames`.
- [x] Untrusted input ⚑: `TestReadRejectsAnIDThatIsNotAUUID`,
      `TestReadRejectsPathTraversalDeviceNameBeforeUse` (`../../../etc/x`,
      an absolute path, a separator: rejected, never normalized).
- [x] Schema: `[vault]` `name`, `id`, `type`, `format_version`, `created`;
      `[method]` `default` (+ optional `plugin`); `[[recipients]]`
      `device`, `pubkey`. Read checks run in Go's order: version, id,
      device names.
- Rust notes: `TestRecipientsCarryOnlyDeviceAndPubkey` is an exhaustive
  destructuring pattern (adding a field stops it compiling) plus a check of
  the serialized keys, in place of Go's reflection. Errors are a typed enum
  (`UnsupportedFormatVersion { found, skew }`, `InvalidVaultId`,
  `InvalidDeviceName`), not wrapped sentinels.

### 2. Recipients and validation — 5 rows claimed from P2's `agekey_test.go`

Needed by task 4. The `recipients` package's own Go tests stay in P5.

- [x] Recipient validation, Go's `agekey` ported (amended above; see
      [Q-AGE-CRATE](../open-questions.md#q-age-crate)): a well-formed
      `age1…` key passes, plugin recipients included; malformed bech32,
      wrong HRP, and an `AGE-SECRET-KEY-1…` identity are rejected with
      Usage. Claims `TestValidateRecipientAccepts`,
      `TestValidateRecipientRejectsBadChecksum`,
      `TestValidateRecipientRejectsMalformed`,
      `TestValidateRecipientRejectsNonPrintableHRP`,
      `TestIsPrintableASCIIBoundaries`, since the package is built here.
- [x] `.age-recipients` writer: one key per line, no framing, trailing
      newline, mode 0644; plus the reader `Vault::create`'s tests need.
      Test: the bytes are exactly the recipients-file grammar
      (`recipients::tests::write_has_no_framing_one_per_line`). The
      round-trip through the `age` crate's own reader moves to P2 · M2
      with the crate.

### 3. Git surface — 7 rows claimed from P4's `gitrepo_test.go`

- [x] `TestInitAndCommitCreatesExactlyOneCommit`,
      `TestSetRemoteThenChangeIt`, `TestRemoteURLEmptyWhenUnset`,
      `TestRemoteURLAndHasRemoteWithNoConfiguredURLs`,
      `TestHasRemoteOnARepoWithNoOriginConfigured`,
      `TestIsCleanDetectsDirtyWorkingTree`,
      `TestIsCleanCountsUntrackedFilesAsDirty`.
- [x] `gitrepo::{init_and_commit, set_remote, remote_url, is_clean}` on
      the crate chosen in M0b, with the git defaults above. Test: commit
      author, committer and branch are gage's even when `HOME` holds a
      `.gitconfig` setting `init.defaultBranch` and `user.name`.
- [x] Not claimed: `TestRequireCleanWorkTreeTruncatesTheNamedPaths` and
      `worktree_state_test.go` stay in P4 (sync preconditions).
- Rust notes: the git-defaults test runs in a child process (the test
  binary re-executed, as `vaultlock`'s tests do), because `HOME` is
  process-wide and libgit2 reads it once. A mutation check (dropping the
  explicit initial branch) turns it red with `refs/heads/trunk`.
  `remote_url` returns `Option<String>` where Go returns `""`. Also ported:
  `has_remote` and `commit_count`, which the claimed rows use.

### 4. `Vault::create` — `vault_create` (23 rows)

- [x] Skeleton: `TestCreateWritesFullSkeleton`, `TestCreateCommitsTheSkeletonFiles`,
      `TestCreateGitattributesMarksRecipientFilesUnmergeable`
      (`.age-recipients -merge` and `.gage/config.toml -merge`),
      `TestCreateGitignoreHasOnlyOSCruft` (`.DS_Store`, `Thumbs.db`),
      `TestCreateVaultConfigRoundTrip`, `TestCreateRecipientsMatchAgeRecipientsFile`,
      `TestCreateRepeatedRecipientsAllWritten`.
- [x] Id: `TestCreateRecordsTheIDItWasHandedRatherThanMintingOne`,
      `TestCreateRefusesAnIDThatIsNotAUUID`.
- [x] Validation before any file is touched: `TestCreateRejectsEmptyName`,
      `TestCreateRejectsEmptyPath`, `TestCreateRejectsUnknownType`,
      `TestCreateRejectsUnknownMethod` (including `ssh`, `yubikey`,
      `age-key`, `secure-enclave`, `plugin:x`), `TestCreateRejectsInvalidDeviceName`,
      `TestCreateRejectsMalformedRecipient`, `TestCreateRequiresAtLeastOneRecipient`,
      `TestCreateAcceptsExplicitGitType`, `TestCreateAcceptsExplicitPassphraseMethod`.
- [x] Target: `TestCreateIntoNonEmptyDirectoryFailsWithoutTouchingFiles`,
      `TestCreateRejectsATargetThatIsARegularFile` (both Conflict).
- [x] Remote: `TestCreateWithRemoteSetsOrigin`,
      `TestCreateWithoutRemoteLeavesVaultRemoteless` (library only; no CLI
      flag yet).
- [x] Partial: `TestCreateDeviceNameLandsInRecipientAndIdentityPath`:
      the recipient-label half here; **the identity-path half moves to
      P2 · M2**.
- [x] Not ported here: Go's reserved recovery label check
      (`checkDeviceLabelFree`) and `ExtraRecipients` (the labelled
      recovery key, with its duplicate checks) arrive with **P8**.

### 5. `gage init` — `init` (4) and 16 `vault_lifecycle` rows

- [x] Device name: `TestResolveDeviceNameExplicitOverridesHostname`,
      `TestResolveDeviceNameExplicitFailingAllowlistIsUsageError`,
      `TestResolveDeviceNameFallsBackToNormalizedHostname`,
      `TestResolveDeviceNameHostnameNormalizesToNothingPromptsRatherThanInventing`
      (as Go now behaves: fails with Usage naming `--device`, never invents
      a name).
- [x] `TestInitCreatesFullSkeletonAndRegisters`,
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
- [x] Partial: `TestInitDeviceFlagAgreesEverywhere`: global config and
      `[[recipients]]` here; **the identity-file half moves to P2 · M2**.
- [x] Partial: `TestReinitAfterVaultRemoveIsAnOrdinaryRetry`: re-init
      succeeds and gets a new id here; **the identity-file halves move to
      P2 · M2**.
- [x] Interim (no Go row; Go M1's own bullet): `init` with no
      `--recipient` fails with Usage naming the flag. **M2 rewrites this
      test**, as Go's M2 did.
- [x] Moved to **P2 · M2**: `TestInitWithNoRecipientGeneratesThisDevicesIdentity`,
      `TestInitWritesTheWrappedIdentityFile`,
      `TestInitExtraRecipientFollowsTheDeviceKey`.
- [x] Vault id via `init` (`vaultid_keying`): `TestInitMintsADistinctUUIDv4PerVault`,
      `TestInitWritesFormatVersionTwo`.
- [x] Register `init` (one-shot only) in the registry with an accurate
      help text for what it does in M1; add its cases to the
      `errorprefix` table.
- Rust notes: the CLI tests are in `crates/gage-cli/src/lifecycle_tests.rs`
  under Go's names in snake case. Go's runs pass `--no-recovery-key`,
  which doesn't exist until P8. `init`'s long description is baked into
  its help template, so `-h`, `--help` and `gage help init` agree
  (`init_help_describes_this_milestone`). `--dir` is registered as given,
  as Go does.

### 6. Vault commands, resolution and the id check — 7 `vault_lifecycle` rows, 5 `vaultid_keying` rows

- [x] A single vault resolver used by every command that reads a vault:
      global config entry → the vault's `.gage/config.toml` → compare ids.
      If the vault's config is unreadable, global config's id stands alone
      (only for commands that don't need the vault's contents, i.e.
      `vault remove`).
- [x] `vault list`, `vault info [name]`, `vault remove <name>`,
      `vault set-default <name>`, `git set-remote <name> <url>`, registered
      with their parents (`vault`, `git`); the M0b parent-command help
      tests re-run against these real parents.
- [x] `TestVaultListIncludesFreshlyInitedVault`,
      `TestVaultInfoByNameReportsExpectedFields` (type, method, recipient
      count, remote, clean/dirty), `TestVaultInfoNoArgumentReportsOnCurrent`,
      `TestVaultInfoReportsDirtyWorkingTree`,
      `TestVaultRemoveDropsRegistrationButLeavesFiles`.
- [x] Untrusted input through a real command ⚑:
      `TestTamperedDeviceNameRefusedByRealCommand`,
      `TestTamperedFormatVersionRefusedByRealCommand`,
      `TestAVaultFromBeforeThisChangeIsRefusedWithARecreateMessage`,
      `TestAnUnsafeCommittedIDIsRejectedBeforeAnyPathIsBuilt` (hand-edited
      committed id with traversal, separator, absolute path),
      `TestAMismatchedRegistrationIsRefusedRatherThanSilentlyPreferred`
      (message names re-registration).
- [x] Partial: `TestAnUnreadableVaultFallsBackToGlobalConfigsIDAlone`:
      `vault remove` succeeds and drops the registration when the vault's
      files are gone; **the identity-file and stderr-names-the-identities-dir
      halves move to P6 · M12** (orphan cleanup).
- [x] Partial: `TestAVaultWithoutAnIDFailsLoudlyOnEveryDerivedPath`: a
      registration with an empty id fails `vault info` and every path
      function; **the `show`/`insert`/`identity list` halves move to
      P2 · M4 and P5** as those commands land.
- [x] `git set-remote` (Go M1 bullets, no P1 Go row; `TestSetRemoteThenChangeIt`
      covers the library): sets `origin` and `[vaults.<name>.git].origin`
      in one call; changing it updates both and `vault info` shows the new
      URL; missing either argument is a Usage error with no change made.
- [x] `vault set-default` (Go M1 bullet): updates `current`; an unknown
      name fails with NotFound and changes nothing.
- [x] Every error path added here gets its row in the `errorprefix` table
      (`lifecycle_tests::no_vault_command_doubles_gages_own_prefix`).
- Rust notes: `TestVaultInfoByNameReportsExpectedFields` expects 2
  recipients where Go's expects 3 (Go's include the device key M2
  generates). The mismatch test runs through `vault info`, M1's read
  command (Go's uses `show`). The id-rejection test adds an uppercase UUID
  and checks that nothing appears under the data *or* state roots.
- Opus review of the untrusted-input tests (done): beyond the listed
  cases, probed the real binary with an id and a device name carrying
  terminal escapes (rendered escaped, refused with Conflict),
  `format_version` as a string and as a duplicate key (refused as
  malformed, Internal, nothing built), and checked that every recipient,
  not only the first, is validated. The probes turned up one bug, fixed
  with a test: the CLI printed a library error's cause twice (and three
  times through `atomicfile`), because errors name their source in their
  message and also expose it as `source()`
  (`error::tests::a_cause_already_in_the_message_is_not_repeated`).

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

- [x] Port Go's `doc/cli/vaults.md`, `configuration.md` and
      `exit-codes.md` for the surface that exists, describing shipped
      behavior (including the interim `--recipient` requirement).
- [x] Parity ledger: P1 rows' status updated; §5's coarse-spots note gains
      the 7 `gitrepo` rows claimed here, and the 5 `agekey` rows.
- The docs live in `doc/cli/` with an index linked from the README.

## Definition of done

Every box above checked; all 89 P1 rows plus the 7 claimed `gitrepo` rows
and 5 claimed `agekey` rows
ported, or partially ported with the remainder named in this doc, or moved
in task 7's table; the Opus review of the untrusted-input tests done;
`make lint` and `make test` clean locally and on Linux, macOS and Windows
CI. You can create, list, inspect, re-point and forget vaults; nothing in
them is encrypted yet.
