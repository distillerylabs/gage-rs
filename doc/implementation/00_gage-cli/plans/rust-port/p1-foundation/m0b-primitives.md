# M0b — Git-crate spike, primitives, registry, help

[← M0a](m0a-toolchain.md) · [P1 plan](index.md) · next: [M1 — Vault lifecycle](m1-vault-lifecycle.md)

> **Recommended model: Opus for task 1** (the git-crate spike: a decision
> that is expensive to reverse after P4), **then Sonnet** for tasks 2–9,
> which are well specified and pattern-following. **Run an Opus review
> pass over this milestone's tests before moving on**: the `vaultlock`
> serialization test is the archetypal vacuous test. A thread-based
> version passes and proves nothing, because file-lock semantics are per
> open file description.

## Goal

Settle the git crate, then build every primitive that later milestones
publish against, before there is any real logic to misplace: the exit-code
taxonomy, atomic writes, XDG paths, device-name and vault-id validation,
the per-vault paths, `vaultlock`, the global config, `gittest`, the
`Prompter`/`Identity`/`Vault::unlock` skeletons, the command registry, help
and usage errors, `--version`, and bare-`gage` dispatch. Also
`reset-local-state`, which needs `xdgpaths`.

At the end `gage --help`, `gage help` and `gage --version` work and bare
`gage` dispatches by TTY. No vault commands exist yet; that is correct.

## Depends on

- **M0a**: workspace, Makefile, CI, purity lint, coverage gate.

## Design references

- Design doc ["Library architecture"](../../../tdds/gage-cli-design.md): why
  interactive decisions are typed values and `Identity` is a parameter.
- Design doc "Global config" (XDG roles, Windows mapping), "Local identity
  storage" (path shape only), "Concurrent processes and the vault lock".
- [Port plan, "Cross-milestone contracts"](../index.md): every M0b row.
- [Q-FILE-LOCK](../open-questions.md#q-file-lock),
  [Q-GIT-CRATE](../open-questions.md#q-git-crate),
  [Q-HELP-RENDER](../open-questions.md#q-help-render).

## Go sources

- Go plan `gage-cli-design/m0-scaffolding.md` (everything except M0a's
  build/CI bullets); `gage-cli-init-design/e0-vault-id-keying.md` for the
  id-keyed lock and identity paths and the id validator.
- Parity ledger rows: #23 / PR #24 (parent commands list their children),
  `db2731f` (usage rejection shows the command's usage), `ddce8b1` (no
  doubled `gage:` prefix), `c433791` (`reset-local-state`), E0 `0d710d4`
  (lock path half).
- Go sources at the pinned SHA: `internal/gage/{exitcode,atomicfile,
  xdgpaths,devicename,config,vaultlock,gittest}/`,
  `internal/gage/vaultconfig/id.go`, `internal/gage/paths.go`,
  `internal/gage/prompter.go`, `cmd/gage/{registry,help,version,main,run,
  globalconfig}.go`, `scripts/resetlocalstate/`.
- Test inventory: 97 P1 rows, listed per task below: `exitcode` (7),
  `atomicfile` (6), `xdgpaths` (8), `devicename` (10), `vaultconfig/id` (4),
  `paths` (6), `vaultlock` (7), `config` (6), `gittest` (2), `registry` (5),
  `help` (16), `usageerror` (5), `errorprefix` (2), `version` (8),
  `main_integration` (5).

## Decisions to make first

**Resolved** (confirmed at the start of implementation, [#6](https://github.com/distillerylabs/gage-rs/issues/6)),
as written below, with one amendment: `Io` also carries the `BuildInfo`
`--version` prints. In Rust the version is stamped at compile time
(`build.rs` passes `GAGE_VERSION`/`GAGE_COMMIT` to the crate), so `main`
passes `BuildInfo::stamped()` and tests inject any value; the signature
stays `run(args, &mut Io, &dyn Prompter) -> Code`.

- **Exit codes in Rust.** `gage::exitcode::Code` is an enum with Go's
  values (Success 0, Conflict 1, Usage 2, NotFound 3, Ambiguous 4,
  LockedOrAuth 5, Internal 6, Unreachable 7). Library error enums
  (`thiserror`) implement `exitcode::HasCode`. `gage-cli` handlers return
  `Result<(), CliError>`, where `CliError` carries a `Code` plus an
  `anyhow::Error` and has `From<E: HasCode + Error>`. An error that never
  went through `HasCode` maps to Internal, as Go's `CodeOf` does, so no
  command can surface an unenumerated status by accident.
- **`gittest` location.** A `gage::gittest` module behind a `test-support`
  cargo feature, enabled by `gage-cli`'s dev-dependency on `gage` and by
  `cfg(test)` inside the library. It keeps Go's name, so the ledger and
  inventory map one to one. It is excluded from the coverage gate.
- **`run` signature.** `gage_cli::run(args, &mut Io, &dyn Prompter) ->
  Code`, where `Io` holds stdin (reader), stdout and stderr (writers) and an
  `is_terminal` flag for stdin. `main` is the only place that touches the
  real process streams, calls `std::process::exit`, or reads build info.
- **Help rendering.** clap parses and renders. The root listing is grouped
  in registry group order, with aliases shown as `name, alias`. Rendering
  functions take the registry as a parameter, so tests can use a test
  registry where real commands don't exist yet (parent commands arrive in
  M1). See [Q-HELP-RENDER](../open-questions.md#q-help-render).
- **Bare `gage` on a TTY** dispatches to a session entry point that, until
  P3 · M6, fails with Internal and "session mode is not implemented yet".
  Bare `gage` with stdin redirected prints the root help and exits 0.
- **Version**: `gage <version> (commit <sha7>)`, where version is
  `GAGE_VERSION` at build time, else `dev`, and commit is `GAGE_COMMIT`,
  else `unknown`. No further fallback: Rust embeds no module version or VCS
  revision the way Go's `runtime/debug` does.
- **Holder info on Windows**: per [Q-FILE-LOCK](../open-questions.md#q-file-lock),
  holder-detail assertions run on Unix only; Windows asserts a contended
  error.
- **`Prompter` in M0b**: `unlock(UnlockRequest) -> UnlockResponse` (typed,
  carrying a `Kind`, only `Passphrase` today) and `confirm`. `value`,
  `confirm_default_yes` and candidate choice are added by the milestones
  that first need them.

## Tasks

Tests first, at task granularity: write a task's tests, get them green, move on.

### 1. Git-crate spike (Opus) — resolves [Q-GIT-CRATE](../open-questions.md#q-git-crate)

- [x] On a scratch `spikes/git/` workspace member, show for each candidate
      (`gix`, `git2` with vendored libgit2 and no system OpenSSL/libssh2
      requirement), offline and on all three CI platforms: init + commit
      with an explicit branch and signature that ignore the user's git
      config; fetch and fast-forward from a local bare repo; push to it;
      divergence detection; a merge commit with chosen per-file
      resolutions; a single-path history walk; a custom transport hook
      that records the credentials it receives (Go's
      `gittest/transport.go`); HTTPS support compiled in.
- [x] Record the evidence (what worked, what needed workarounds, links to
      the CI runs) and the decision in Q-GIT-CRATE; update the port plan's
      crates table.
- [x] Delete `spikes/` before review. The merged diff for this task is docs
      and the chosen dependency.

**Outcome:** `git2` (vendored libgit2 + OpenSSL, static zlib, no ssh); `gix`
0.89 has no push client. Evidence, the CI run, and the consequences carried
forward (process-wide isolation from the user's git config, since libgit2
honors `insteadOf`; `Unicode-3.0` in `deny.toml`) are in
[Q-GIT-CRATE](../open-questions.md#q-git-crate). The spike ran on the
throwaway `6-m0b-git-spike` branch, since deleted.

### 2. Exit codes — `exitcode` (7 rows)

- [x] `TestSuccessIsZero`, `TestAllCodesAreDistinct`, `TestNilErrorIsSuccess`
      (`Ok` maps to Success), `TestBareErrorIsInternal`,
      `TestEveryDefinedCodeRoundTripsThroughConstructors`,
      `TestUnregisteredCodeStringsAsItsNumber` (Rust: every variant has a
      stable name; the test asserts `Display` for all of them, since an
      out-of-range code can't be constructed), `TestWrapNilIsNil` (Rust:
      the `From` conversion is only reachable from an error; ported as a
      compile-time property and excused as a runtime test).
- [x] `exitcode::all()` lists every code, used later to assert each is
      produced by at least one path.

### 3. Atomic writes — `atomicfile` (6 rows)

- [x] `TestWriteFileCreatesAndReads`, `TestWriteFileOverwritesAtomically`,
      `TestWriteFileReplacesRatherThanTruncatesInPlace` (an open handle on
      the old file still reads the old bytes), `TestFailedWriteLeavesPreviousFileIntact`,
      `TestWriteFileFailedRenameLeavesNoTempFileBehind`,
      `TestWriteFileFailsForMissingDir`.
- [x] `atomicfile::write(path, bytes, mode)`: temp file in the same
      directory (`.<name>.tmp-*`), write, `sync_all`, set mode (Unix),
      rename. Failure removes the temp file.

### 4. Paths and validation — `xdgpaths` (8), `devicename` (10), `vaultconfig/id` (4), `paths` (6)

- [x] `xdgpaths`: `TestUnixDefaults` (macOS uses the Unix layout, not
      `~/Library`, so no `dirs` crate), `TestUnixXDGEnvOverride`,
      `TestWindowsDefaults`, `TestWindowsMissingEnvFallsBackToHome`,
      `TestWindowsXDGEnvOverride`, `TestEmptyHomeDirIsAnError`,
      `TestRealAccessorsResolve`, `TestUnknownRoleValuePanicsInAllThreeSelectors`
      (Rust: `Role` is a closed enum, so an unknown role can't exist;
      excused as a runtime test). Resolution is a pure function of (OS,
      env, home) so all platforms' rules are tested on every platform.
- [x] `devicename`: `TestNormalizeHostname`, `TestNormalizeCollapsesRuns`,
      `TestNormalizeCapsLength`, `TestNormalizeNothingUsable`,
      `TestNormalizeNeverProducesAnAllDotsName`, `TestValidAccepts`,
      `TestValidRejectsOverLength`, `TestValidRejectsPathTraversalAndSeparators`,
      `TestValidRejectsAllDotsNames`, `TestValidStillAcceptsNamesContainingDots`.
      Same allowlist (`a-z0-9._-`), max 63, and normalization rules as Go.
- [x] Vault id: `TestNewIDMintsADistinctUUIDv4`,
      `TestValidIDAcceptsTheCanonicalForm`,
      `TestValidIDRejectsAnythingThatIsNotACanonicalUUID` (nil UUID,
      uppercase, braces, URN form, whitespace all rejected),
      `TestValidIDsAreAlwaysSinglePathComponents`.
- [x] `paths`: `TestIdentityFilePathShape`, `TestIdentityFilePathIsSiblingOfVaultsDir`,
      `TestLockFilePathShape`, `TestTrustCacheDirShape`,
      `TestPerVaultPathsRefuseAnythingButAValidID` (the empty id included,
      with LockedOrAuth, as Go), `TestNoPerVaultLocalStateIsKeyedByTheLocalName`.
      Shapes: `$GAGE_DATA/vaults/`, `$GAGE_DATA/identities/<id>/<device>.age`,
      `$GAGE_STATE/locks/<id>.lock`, `$GAGE_STATE/<id>/`.

### 5. `vaultlock` (7 rows) ⚑

- [x] `TestAcquireRoundTripsOnThisPlatform`,
      `TestAcquireSerializesAcrossProcesses`, `TestReleaseLetsWaiterProceed`,
      `TestKilledHolderReleasesLock`, `TestAcquireZeroTimeoutFailsFastWhenContended`,
      `TestReleaseOnANilLockIsANoOp` (Rust: release is `Drop`, plus an
      explicit `release()` returning the unlock error; ported as "dropping
      after an explicit release is a no-op"), `TestMain` (the child-process
      trampoline: the test binary re-execs itself with a sentinel env var to
      act as the holder).
- [x] Holders are **child processes**, never threads. The holder prints
      `ACQUIRED`, then waits for stdin to close (graceful) or is killed.
- [x] `Lock::acquire(path, timeout)`: poll every 20 ms up to `timeout`; a
      zero timeout tries once; contention returns
      `Contended { path, holder }`; the holder record `pid=<n>\nsince=<rfc3339>`
      is written at offset 0 and read best-effort.

**Notes:** the holder announces `ACQUIRED` on stderr, because libtest writes
its own progress to stdout. Contended maps to Conflict and every other
failure (open, lock I/O) to Internal. That follows the comment on Go's
`acquireVaultLock`; Go's code actually wraps *every* `Acquire` failure as
Conflict. M4 owns that wrapper and should confirm the split when it lands.
The `since` timestamp uses `humantime` (RFC 3339 in and out), a narrow
choice that leaves the date-time crate for entry timestamps to P2.
Mutation check: with `try_lock` replaced by a no-op, the three
cross-process contention tests fail.

### 6. Global config — `config` (6) and `gittest` (2)

- [x] `config`: `TestRoundTrip` (`current`, `[vaults.<name>]` with `path`,
      `id`, `type`, `device`, `pubkey`, `method`, `[vaults.<name>.git]
      origin`, and `[shell]`), `TestReadMissingFile` (a distinguishable
      not-found variant), `TestReadRejectsMalformedTOML`,
      `TestGlobalConfigWriteIsAtomic`, `TestFailedGlobalConfigWriteLeavesPreviousConfigParseable`,
      `TestWriteThenModifyLeavesOldValueUntouchedUntilRename`. Written with
      `toml` + serde through `atomicfile`, mode `0600`; empty optional
      fields omitted as Go does.
- [x] `gittest`: `TestNewBareRemoteIsABareRepo`,
      `TestNewBareRemoteProducesIndependentRepos`. `new_bare_remote()`
      returns a guard owning a temp dir with a bare repo, using the crate
      chosen in task 1.

**Notes:** `config::read` returns `Error::NotFound` for a missing file (Go
returns the raw `os` error). A test also asserts that empty optional fields
are omitted. `gittest` lives behind the `test-support` feature;
`new_bare_remote` returns a guard that deletes the repo on drop.

### 7. `Prompter`, `Identity`, `Vault::unlock` skeletons

No inventory rows (Go's `vault_test.go` rows moved to P2 · M2); these are
the contracts M2 fills in.

- [x] `Prompter` trait as decided above; `UnlockRequest`/`UnlockResponse`
      with `Kind::Passphrase`. A `FakePrompter` in test support.
- [x] `Identity` type with no key material yet, and the signature
      `Vault::unlock(&self, &dyn Prompter) -> Result<Identity, UnlockError>`
      returning a typed "not implemented until M2" variant. Test: the
      signature compiles against a fake prompter and the variant maps to
      LockedOrAuth.

**Notes:** `UnlockResponse` is an enum with one variant per `Kind`
(`kind()` reads it back), and its `Debug` redacts the passphrase.
`PromptError` is `Cancelled` or `Io`. `FakePrompter` panics on an
unscripted question, so a test can't pass on a prompt it never expected.
`Vault` carries Go's `name`, `path` and `id`; M1 opens and creates them.

### 8. Registry, help, usage errors — `registry` (5), `help` (16), `usageerror` (5), `errorprefix` (2)

- [x] Registry entries: name, aliases, short, group, usage, availability
      (`Both`, `OneShotOnly`, `SessionOnly`); group order Session, Vault
      lifecycle, Identity, Recipients, Entry CRUD, Sync, Git-specific.
      Lives in `gage-cli`; the purity lint confirms nothing leaked into the
      library.
- [x] `registry`: `TestRegistryCompleteness` (every clap subcommand path is
      in the registry and vice versa), `TestRegistryCompletenessDetectsNestedDrift`,
      `TestFindCommandResolvesAliases`, `TestFindCommandUnknown`,
      `TestAliasResolvesToSameCommandAtCLILevel`.
- [x] `help`: `TestHelpFlagExitsZero`, `TestHelpCommandExitsZeroAndMatchesHelpFlag`,
      `TestHelpSubcommandPrintsUsageAndExitsZero`,
      `TestHelpUnknownTopicFailsWithUsageCode`, `TestHelpOutputIsGrouped`,
      `TestOneShotHelpMatchesRegistryExactly`, `TestEveryRegistryEntryHasShortAndGroup`,
      `TestNeitherHelpSpellingListsSessionOnlyCommands`,
      `TestAliasesListedAlongsideCanonicalName`,
      `TestCommandHelpListsAliasesSortedWithTheCanonicalName`,
      `TestGroupCommandsStillRendersAGroupMissingFromGroupOrder`,
      `TestParentCommandHelpListsItsChildren`,
      `TestParentCommandChildrenListedWithTheirShortText`,
      `TestParentCommandHelpMatchesAcrossBareDashHelpAndHelpCommand`,
      `TestLeafCommandHelpUnchangedByTheChildrenListing`. All structural,
      against a test registry; M1 adds the same assertions for the real
      `vault` and `git` parents. `TestListsCommandEntryIgnoresProse` is
      **excused**: it tests a Go test helper that parsed Cobra's help text;
      the Rust tests read the registry and clap's command tree instead.
- [x] `usageerror`: `TestWrongArgCountShowsCommandUsage`,
      `TestUnknownFlagShowsCommandUsage`, `TestUnknownCommandShowsFullHelpListing`,
      `TestCodedApplicationErrorsDoNotGrowUsageText` (only parse rejections
      get usage appended). All exit with Usage. `TestSessionWrongArgCountShowsCommandUsage`
      moves to **P3 · M6**: there is no session yet.
- [x] `errorprefix`: `TestNoCommandDoublesGagesOwnPrefix`,
      `TestWarningsDoNotDoubleGagesOwnPrefix`, as a table over the commands
      that exist (unknown command, unknown flag). **Process rule**: every
      later milestone that adds a command adds its failing cases to this
      table.

**Moved:** `TestWarningsDoNotDoubleGagesOwnPrefix` needs `Prompter::warn`,
which M0b doesn't have (the plan limits M0b's prompter to `unlock` and
`confirm`). Its rendering half is ported (`prefix_once` never doubles, in
`error.rs`). The end-to-end half, a real warning through `warn`, moves to
**P2 · M2**, which adds `warn` for the page-lock warning.

### 9. `--version`, dispatch, real-binary smoke tests, `reset-local-state` — `version` (8), `main_integration` (5)

- [x] `version`: `TestVersionFlagPrintsVersionAndCommit`,
      `TestVersionStringTaggedBuild`, `TestVersionStringUntaggedBuild`,
      `TestResolveBuildInfoPrefersLinkerValues`,
      `TestResolveBuildInfoKeepsExplicitDev`,
      `TestResolveBuildInfoWithoutEmbeddedInfo` (Rust: a pure
      `resolve_build_info(env_version, env_commit)`).
      `TestResolveBuildInfoFallsBackToModuleVersion` and
      `TestResolveBuildInfoFallsBackToVCSRevision` are **excused**: they
      test Go's `runtime/debug` build info, which Rust has no equivalent
      of; the `dev`/`unknown` fallbacks are covered above.
- [x] `main_integration` (`assert_cmd`): `TestRealBinaryVersionFlag`,
      `TestRealBinaryWithRedirectedStdinPrintsHelp`, `TestMain` (Rust needs
      no binary-building `TestMain`; excused), `TestMakefileLdflagsTargetTheRealSymbols`
      (Rust: the Makefile exports exactly the env var names the binary
      reads, asserted from one shared constant), `TestLinkTimeVersionInjection`
      (Rust: a CI step builds with `GAGE_VERSION=v9.9.9-testtag
      GAGE_COMMIT=cafef00` and asserts `--version` prints both; a cargo
      rebuild inside `make test` is too slow to be worth it).
- [x] Bare `gage` dispatch: with an injected TTY flag, a TTY goes to the
      session entry point and redirected stdin prints the root help.
- [x] `reset-local-state`: a `publish = false` binary crate
      (`crates/reset-local-state`) printing the three resolved directories
      and deleting them only after the user types `yes`; `make
      reset-local-state`. Never a `gage` subcommand. Test: answering
      anything but `yes` deletes nothing.

**Notes:** the variable names are defined once, in
`crates/gage-cli/src/build_env.rs`. `build.rs` `include!`s that file, and
the Makefile test reads it. The CI step is `make stamp-check`.

## Status

All nine tasks are implemented. `make lint`, `make test`, `make
lint-selftest` and `make audit` are clean locally (macOS), with line
coverage at 94%. All 97 claimed rows are accounted for: ported, excused
above, or moved (`TestSessionWrongArgCountShowsCommandUsage` to P3 · M6,
and the warning half of `TestWarningsDoNotDoubleGagesOwnPrefix` to
P2 · M2). Each ported test carries a `// Go: TestName` comment naming its
row.

Opus review of the tests (⚑): `vaultlock` holders are child processes,
the waiter is checked to still be blocked before the release, the kill
test reaps the holder first, and the mutation check above confirms the
contention tests fail without a real lock.

Remaining: green on Linux, macOS and Windows CI for the PR.

## Definition of done

Every box above checked; every one of the 97 claimed rows ported, excused
in this doc with its reason, or moved to the named milestone; the Opus
review of the tests done; `make lint` and `make test` clean locally and on
Linux, macOS and Windows CI; the port plan's crates table and Q-GIT-CRATE
updated.
