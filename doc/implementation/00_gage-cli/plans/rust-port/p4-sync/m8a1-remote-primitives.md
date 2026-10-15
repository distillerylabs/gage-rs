# M8a1 — Remote primitives & auth

[P4 plan](index.md) · [#33](https://github.com/distillerylabs/gage-rs/issues/33) · next: [M8a2 — Sync wiring](m8a2-sync-wiring.md)

> **Recommended model: Opus.** libgit2's remote surface is documented
> thinly and its callbacks run through FFI. Classifying errors here is
> real problem-solving, not transcription. This milestone also carries
> two security properties a passing suite is unlikely to notice breaking:
> the token reaches the transport and no further, and the user's git
> config can't redirect a vault. Review those tests again at the end
> (task 9).

## Goal

The library half of Go's M8a. It builds everything that talks to a
remote or reasons about two histories, testable against bare repos and a
loopback recorder, with no `Vault` or `Session` changes:

- **libgit2 isolation and timeouts.**
- **`gitrepo`:** fetch, push, compare, ahead count, fast-forward,
  file-level merge, reset, the dirty-tree guard, clone and branch choice.
  It also gets a typed error.
- **`syncerr`:** sorts any failure into unreachable, auth or diverged.
- **`remoteauth`:** per-host HTTPS tokens, best-effort ssh-agent, and the
  hint telling the user how to fix an auth failure.
- **The `gittest` sync harness:** a second clone that plays "another
  device", and an HTTP recorder.
- **`gage auth login|status|logout`**, the one CLI surface that needs
  nothing from M8a2.

Nothing calls fetch or push automatically yet. Pull on unlock, push on
write and the sync verbs are M8a2's.

## Depends on

- **P1 · M0b/M1:** `gitrepo` (init, commit, `set_remote`, `remote_url`,
  `has_remote`, `is_clean`), `gittest::new_bare_remote`, `atomicfile`,
  `xdgpaths`/`paths`, `exitcode::{Unreachable, LockedOrAuth, Conflict}`,
  and the command registry with its `GROUP_SYNC`.
- **P2 · M2:** `Prompter::value`, the token prompt that `auth login`
  shares.
- **P3 · M6:** session availability in the registry, so `auth` works in
  both modes.

## Design references

- Design doc, "Sync model": the split between local durability and
  network risk.
- Design doc, "Git-specific commands": `auth`, and `--host` defaulting to
  the origin's host.
- Design doc, "Authentication": HTTPS + token, best-effort SSH, and why
  `~/.ssh/config` isn't honored.
- Design doc, "What \"diverged\" actually means": the deletion shapes that
  `merge_remote` classifies.
- Q-GIT-CRATE's spike table: fetch, push, `NotFastForward`, the `merge`
  shape, and token on the wire.
- [Q-SSH](../open-questions.md#q-ssh),
  [Q-GIT-ISOLATION](../open-questions.md#q-git-isolation),
  [Q-SYNC-CLASSIFY](../open-questions.md#q-sync-classify),
  [Q-REMOTE-TIMEOUT](../open-questions.md#q-remote-timeout),
  [Q-FILE-MERGE](../open-questions.md#q-file-merge),
  [Q-REMOTE-AUTH](../open-questions.md#q-remote-auth).

## Go sources

- Go plan `gage-cli-design/m8a-sync-transport.md`: the "Push failure
  vs. divergence", "Test harness", "Authentication" and "Local state a
  sync has to respect" sections, its "Corrections made during review",
  and #38's `TokenHost`.
- Parity ledger rows:
  - §3 M8a
  - §4 `auth`, `auth_login`, `auth_status`, `auth_logout`
- Go sources at the pinned SHA:
  - `internal/gage/gitrepo/{gitrepo,remote,merge}.go`:
    `RemoteState`, `Fetch`, `Push`, `Clone`, `remoteBranch`, `Compare`,
    `FastForward`, `AheadCount`, `authFor`, `annotateAuth`, `MergeRemote`,
    `PrepareMerge`/`PendingMerge.Commit` (M8a1 builds only their
    internals), `safeJoin`, `ErrDirtyWorkTree`, `DirtyPaths`,
    `requireCleanWorkTree`, `ResetHard`
  - `internal/gage/syncerr/syncerr.go`
  - `internal/gage/remoteauth/remoteauth.go`
  - `internal/gage/gittest/` (the second-clone helper and
    `RecordingTransport`)
  - `cmd/gage/auth.go`
- Test inventory, 57 rows:
  - `internal/gage/gitrepo/gitrepo_test.go` (1:
    `TestRequireCleanWorkTreeTruncatesTheNamedPaths`. M1 claimed the
    other 7.)
  - `remote_test` (15), `reset_test` (4), `worktree_state_test` (4),
    `auth_wiring_test` (4)
  - `internal/gage/syncerr/syncerr_test.go` (6)
  - `internal/gage/remoteauth/remoteauth_test.go` (11)
  - `cmd/gage/auth_test.go` (12)
- Help fixtures: `auth`, `auth_login`, `auth_status`, `auth_logout`.

## Decisions to make first

These are the recommended answers. Confirm them when this plan is
reviewed.

- **The `gitrepo` error.** Today it is one struct that always maps to
  Internal. It becomes an enum whose variants callers can match on:
  - `DirtyWorkTree { paths }`. One variant, shared by fast-forward and
    merge, so the layer above matches it in one place.
  - `NotFastForward`
  - `Unreachable`, `Auth { host }` and `Diverged`. These are `syncerr`'s
    buckets, re-exported or wrapped. Pick one.
  - `UnsafePath`
  - `AmbiguousBranch`
  - `Git { op, path, source }`, which covers everything else.

  `HasCode` maps them to Conflict, Unreachable, LockedOrAuth or Internal.
  Existing callers keep compiling, because the old struct's fields become
  the `Git` variant.
- **`syncerr` as a module of `gitrepo` or its own module.** Go keeps it
  as its own package. Keep that: the ledger and inventory map one to one,
  and M8a2's fake `RemoteSyncer` returns its values without touching
  git2.
  - Its error carries the bucket, plus a message that has already been
    stripped of credentials.
  - It never holds a `git2::Error` whose text could echo a URL with a
    token in it.
- **Classification table.** Pin it with tests that trigger each case for
  real, never from guessed `ErrorClass` values:
  - **Unreachable:** a loopback port that refuses connections, a host
    that doesn't resolve (`.invalid` per RFC 6761, which fails without
    any network), and gage's own deadline abort.
  - **Auth:** the recorder answering 401, 403 and 404.
  - **Diverged:** a real non-fast-forward push to a bare repo, seen both
    as `ErrorCode::NotFastForward` and as the message from
    `push_update_reference`.

  Write down what libgit2 1.9 actually returns for each case. That record
  is the Rust counterpart of Go's note that go-git's Push doesn't wrap
  `ErrNonFastForwardUpdate`.
- **How `push` detects a rejection.** libgit2 reports a ref the server
  rejected through `push_update_reference(refname, Some(status))`, and
  `push()` itself can still return `Ok`. Treat any status as diverged, so
  a rejected push is never reported as success.
- **The ssh-agent test, offline.** Port Go's approach: assert which path
  was taken, not the outcome. The method resolver either returns an
  agent credential, or an error that names ssh-agent. It is never the
  config-alias refusal, and never "unsupported".
  - Resolve the method without connecting, as Go's `Method` does.
  - Settle the known_hosts behavior of libgit2 1.9 with libssh2: does
    it check `~/.ssh/known_hosts`, or must gage supply
    `certificate_check`? Settle it by reading libgit2's source, and
    record the answer in Q-SSH.
- **Redirects.** `follow_redirects(RemoteRedirect::Initial)`, which is
  libgit2's default; set it explicitly anyway. Together with the host
  check in the `credentials` callback, a token is never offered to a host
  it wasn't stored for.
- **How `auth status` and `--host` find a host.** Port Go's
  `resolveHost`. The `--host` flag comes first. Then the current vault's
  origin, read from the *repository*, not from global config
  (`TestAuthLoginReadsTheOriginFromTheRepositoryWhenConfigLags`). A
  vault with no remote is refused, naming the vault.
- **Host validation.** A host becomes a path component under
  `$GAGE_STATE/tokens/`. Validate it as `devicename` validates a device
  name: no separators, no `..`, no empty string, nothing that escapes the
  directory (`TestHostCannotEscapeTheTokensDirectory`). Add
  `paths::token_file_path(state, host)` beside the other path builders,
  and validate before building the path.

## Tasks

Write each task's tests first, at task granularity.

### 0. Build and isolation (Q-SSH, Q-GIT-ISOLATION, Q-REMOTE-TIMEOUT)

- [ ] Enable git2's `ssh` feature with libssh2 vendored, and its OpenSSL
      vendored as well. Run `make audit` and confirm that `deny.toml`
      allows libssh2-sys's license (BSD-3-Clause); record it in Q-SSH.
      Check a cold CI build on all three OSes before writing more.
- [ ] Add one `#[allow(unsafe_code)]` `Once` in `gitrepo`:
  - `set_search_path` points System, XDG, Global and ProgramData at an
    empty directory.
  - Set the server connect and read timeouts.
  - It runs before every repository open, including the opens inside
    `gittest`.
- [ ] Test, in a child process with `HOME`, `XDG_CONFIG_HOME`,
      `USERPROFILE` and `APPDATA` pointing at a hostile global config.
      Assert first that libgit2 can see that config when isolation is
      off, so the test isn't vacuous. Then, with isolation on:
  - a hostile `url.<x>.insteadOf` doesn't redirect a fetch to a decoy
    bare repo
  - `core.autocrlf=true` doesn't touch a committed file
- [ ] Add a note to Q-GIT-CRATE saying the isolation landed here, not in
      M1.

### 1. `gitrepo` errors and the dirty-tree guard — 1 + 2 rows

- [ ] The typed `gitrepo::Error` described above. Existing M1/M4 callers
      and their tests stay green with assertions unchanged.
- [ ] `TestRequireCleanWorkTreeTruncatesTheNamedPaths`: `dirty_paths`,
      and `require_clean_work_tree` naming a bounded number of paths.
- [ ] `TestFastForwardAndMergeShareOneDirtyTreeSentinel`. Finish this
      after tasks 3 and 4.
- [ ] `TestMergeRemoteRefusesADirtyWorkTree`. This is Go's correction:
      an automatic merge must never pick up a human's uncommitted edits.

### 2. Compare, ahead count, fetch and push — 5 + 2 rows

- [ ] The `gittest` second-clone helper. It clones a `new_bare_remote`
      into a temp directory, commits there and pushes back, playing a
      second device.
- [ ] `RemoteState` and `TestRemoteStateStringCoversAllStatesAndUnknown`.
      `Display` gives the exact words, and an out-of-range value falls
      back to `state(<n>)` if the type allows one. If a Rust enum can't
      hold that value, strike the fallback half with a reason.
- [ ] Rows:
  - `TestCompareReportsTheFourStates`
  - `TestCompareReportsRemoteBehind`
  - `TestCompareWithNoRemoteTrackingRefIsRemoteNone`
  - `TestHasRemoteOnADirectoryThatIsNotARepo`
- [ ] Ahead count:
  - `TestAheadCountCountsAPlainRunOfCommits`
  - `TestAheadCountIsNotFooledByAMergeCommit`. Go's correction: count
    what the remote can't reach. A revwalk that hides the tracking ref
    does this.
- [ ] `fetch` and `push`, with the default refspec, the
      `push_update_reference` check and the deadline in the progress
      callbacks. Beyond Go's rows:
  - a push to an up-to-date remote reports that nothing was sent
  - a fetch that brings nothing reports no change
  - each of the deadline callbacks aborts with an unreachable error when
    the deadline has passed. Prove this with a deadline of zero, not a
    real timeout.

### 3. Fast-forward and reset — 2 + 4 rows

- [ ] `TestFastForwardOnlyMovesWhenItIsAFastForward`,
      `TestFastForwardRefusesOverUncommittedChanges`.
- [ ] `reset_hard` returns the paths it discarded, and removes untracked
      files explicitly. Rows:
  - `TestResetHardDiscardsModifiedAndUntrackedFiles`
  - `TestResetHardOnACleanTreeDiscardsNothing`
  - `TestResetHardLeavesHEADWhereItWas`
  - `TestResetHardDiscardsChangesOutsideEntries`

### 4. File-level merge (Q-FILE-MERGE) — 4 rows

- [ ] `prepare_merge`, `PendingMerge::commit` and `merge_remote`, which
      is the two composed with no choices. The merge works from the
      merge base and from tree diffs base→ours and base→theirs.
  - A path changed on both sides is a conflict, unless the two blob ids
    are identical.
  - Anything that conflicts writes nothing.
  - A partial failure rolls back, and always reaches the hard reset.
  - The merge commit has parents [local, remote] and the message
    `gage: merge origin`.
- [ ] Rows:
  - `TestMergeRemoteMergesDisjointChanges`
  - `TestMergeRemoteConflictsAndChangesNothing`
  - `TestMergeRemoteTakesADeletionAndConflictsOnDeleteModify`
  - `TestSafeJoinRefusesPathsThatEscapeTheVault`. Tree paths come from
    whoever can push.
- [ ] Beyond Go's rows: `.age-recipients` with one added line on each
      side conflicts *with no `.gitattributes` in the repository*. This
      is the property that `merge_trees` would have lost. The
      `Vault`-level row is M8a2's.

### 5. Clone — 3 rows

- [ ] `clone(url, dir)`, with `remote_branch` choosing explicitly:
  - a single branch is chosen as-is
  - otherwise `main` or `master`
  - any other ambiguity is refused

  Uses `Remote::connect` + `list` and the same auth wiring as fetch.
- [ ] Rows:
  - `TestCloneProducesAWorkingCopy`
  - `TestCloneWorksWhenTheRemoteHeadPointsNowhere`. A bare repo whose
    HEAD names a branch that doesn't exist.
  - `TestRemoteBranchPicksMainOrMasterAndRefusesAmbiguity`

### 6. `syncerr` (Q-SYNC-CLASSIFY) — 6 rows

- [ ] `classify_push` and `classify_fetch` turn a `git2::Error`, or
      gage's deadline abort, into an `Unreachable`, `Auth` or `Diverged`
      error, or pass it through as Internal. Rows:
  - `TestClassifyPushSortsTheThreeBuckets`
  - `TestClassifyPushCatchesGoGitsRealRejection`: as
    `classify_push_catches_libgit2s_real_rejection`, from a real
    non-fast-forward push.
  - `TestClassifyFetchNeverInventsADivergence`
  - `TestClassifyPassesNilThrough`: in Rust, an `Ok` passes through
    untouched. If that is meaningless for the typed API, strike it with
    a reason.
  - `TestOfflineAndDivergedDoNotShareAnExitCode`
  - `TestClassifyIsIdempotentOnAnAlreadyClassifiedUnreachable`
- [ ] The classification table from "Decisions to make first", written
      as the doc comment above the classifier, with the libgit2 version
      it was observed on.

### 7. `remoteauth` and the wire (Q-REMOTE-AUTH, Q-SSH) — 11 + 4 + 1 rows

- [ ] Token storage:
  - `paths::token_file_path`
  - `store` writes through `atomicfile` with mode 0600, and refuses an
    empty token
  - `load` refuses a file that others can read. On Windows the mode is
    not checked; record that, matching how other 0600 checks are
    handled.
  - `logout` is safe to run twice
  - `hosts` lists every host with a stored token

  Rows:
  - `TestStoreWritesTheTokenUnderStateAt0600`
  - `TestLoadRefusesATokenOthersCanRead`
  - `TestLogoutForgetsTheTokenAndIsSafeTwice`
  - `TestStoreRefusesAnEmptyToken`
  - `TestHostCannotEscapeTheTokensDirectory`
  - `TestTwoVaultsOnOneHostShareOneToken`
- [ ] URL parsing:
  - `host` handles both remote spellings: `https://host/...` and scp-like
    `user@host:path`
  - `token_host` returns a value only for HTTP(S) remotes
  - `auth_hint` names the host and `gage auth login`. A local path, which
    has no host, gets its own wording.

  Rows: `TestHostReadsBothRemoteSpellings`,
  `TestTokenHostOnlyNamesHTTPHosts`,
  `TestAuthHintNamesTheHostAndTheCommand`.
- [ ] SSH: the alias heuristic (a host with no dot) and an ssh-agent
      credential. Rows:
  - `TestSSHRemoteNeedingSSHConfigFailsWithTheRealReason`
  - `TestSSHRemoteWithARealHostIsAttempted`
- [ ] The loopback HTTP recorder in `gittest`, built on `std::net` with
      no new dependency:
  - it answers `401` with `WWW-Authenticate: Basic`
  - it records the `Authorization` header of the retry, then fails
  - it can also answer 403 or 404, for task 6
- [ ] The `credentials` callback:
  - it offers a token only for the host the token was stored for
  - a second call within one operation fails, so a rejected token can't
    loop
- [ ] Rows:
  - `TestFetchAndPushCarryTheStoredTokenToGoGit`: as `…ToLibgit2`, asserted
    on the wire for fetch (upload-pack) and push (receive-pack).
  - `TestFetchAndPushGoAnonymousWithNoStoredToken`
  - `TestPushAfterLogoutNamesTheHostAndTheCommand`
  - `TestNoTokenValueLeaksIntoASyncError`
  - `TestAuthFailuresNameTheHostAndTheFix`, from `remote_test`
- [ ] Beyond Go's rows:
  - a token is not offered to a different host that the recorder
    redirects to
  - a rejected token makes exactly one retry, not a loop

### 8. `gage auth` — 12 rows

- [ ] Register `auth` with `login`, `status` and `logout`, each taking
      `--host`, in the git group and available in both modes.
      Descriptions come from the fixtures. `login`'s long help covers
      fine-grained, single-repository scoping, and the token is prompted
      through `Prompter::value`, never taken as an argument.
- [ ] Rows:
  - `TestAuthLoginStoresATokenAndStatusReportsIt`
  - `TestAuthLogoutForgetsTheToken`
  - `TestHostDefaultsToTheCurrentVaultsOrigin`
  - `TestAuthOnALocalRemoteSaysNoTokenIsNeeded`
  - `TestAuthStatusWithNoTokensAndNoVaultSaysSo`
  - `TestAuthStatusWithNoTokensFallsBackToTheCurrentVaultsHost`
  - `TestAuthLoginReadsTheOriginFromTheRepositoryWhenConfigLags`
  - `TestAuthLoginOnAVaultWithNoRemoteNamesTheVault`
  - `TestAuthCommandsWorkInBothModes`
  - `TestNoTokenValueEverAppearsInOutput`: stdout, stderr and the session
    history file.
  - `TestExpiredTokenErrorNamesTheHostAndTheCommand`
  - `TestNoHostSpecificDependencies`: a `cargo metadata` check, beside
    the frontend-crate check. No host SDK, no OAuth or device-flow
    crate, and no client id string anywhere in `crates/`.
- [ ] `p4_auth_commands_have_their_go_fixture_flags_and_descriptions`.
      Add the `auth` error paths to the `errorprefix` table.

### 9. Review, docs and ledger

- [ ] Review again, on a fresh read: the wire-token tests (task 7), the
      hostile-config test (task 0), and the dirty-tree and ahead-count
      corrections (tasks 1–2). For each, mutation-check that the test
      goes red when its guard is removed. Record the result here.
- [ ] Parity ledger §4: `auth*`. The user docs (`git-and-auth.md`) wait
      for M8b, so they describe the whole shipped surface at once.
- [ ] P4 index status column.

## Definition of done

Every box above is checked and all 57 claimed rows are ported (or struck
with a reason in this doc). The review in task 9 is recorded. `make lint`
and `make test` are clean locally and on Linux, macOS and Windows CI,
with no network access.

The milestone is done when all of the following hold:

- A stored token is observed on the wire for fetch and for push, and
  only for its own host.
- The user's git config can't redirect a vault.
- Unreachable, auth and diverged failures arrive as three distinct typed
  errors with three exit codes.
- A recipient-file change on both sides is a conflict even with no
  `.gitattributes`.
