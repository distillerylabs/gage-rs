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

These were the recommended answers, confirmed at review. Where
implementation settled or changed one, the outcome follows it under
"Resolved in M8a1".

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

### Resolved in M8a1

- **The `gitrepo` error** is the enum `gitrepo::Error`: `Git { op, path,
  source }` (the old struct's fields, so M1/M4 callers are unchanged),
  `DirtyWorkTree { paths, remedy }`, `Sync(syncerr::Error)` (wrapped, not
  re-exported: one place carries the bucket), `RemoteAuth(remoteauth::Error)`,
  `UnsafePath`, `AmbiguousBranch`, plus what the merge and clone needed:
  `NoBranches`, `NothingFetched`, `Unanswered`, `Io` and `RollbackFailed`.
  There is no `NotFastForward` variant. `fast_forward` declines a
  non-fast-forward quietly (`Ok(false)`), as Go's does, and a rejected
  push is `Sync` with `Kind::Diverged`; a variant nothing returns would be
  speculative. Codes: dirty tree Conflict, `Sync` and `RemoteAuth` their
  own, the rest Internal.
- **Classification table**: recorded with what libgit2 1.9.7 actually
  returned, in `syncerr`'s module doc and in
  [Q-SYNC-CLASSIFY](../open-questions.md#q-sync-classify). Two
  departures: 403/404 are untyped, so the whole `Http` class is auth; and
  a local path that doesn't exist is unreachable (libgit2 calls it an
  unsupported URL), where Go says auth.
- **Push rejection**: confirmed. A locked ref on the remote produces a
  `push_update_reference` status with `push()` returning `Ok`;
  `a_ref_the_remote_refuses_is_diverged_never_success` pins it.
- **ssh-agent, offline**: `remoteauth::method` resolves without
  connecting. On Unix an unset `SSH_AUTH_SOCK` is the error naming
  ssh-agent; on Windows libssh2 finds the agent itself. Host keys: libgit2
  checks `~/.ssh/known_hosts` and fails closed with no
  `certificate_check`, so gage installs none
  ([Q-SSH](../open-questions.md#q-ssh)).
- **Redirects**: `follow_redirects(Initial)` on fetch and push (clone's
  branch listing uses `connect_auth`, whose default is the same). Both
  redirect shapes are tested on the wire.
- **Host resolution and validation**: as planned.
  `paths::token_file_path` validates; `paths::Error::InvalidHost` is
  Usage, as Go's `checkHost`. It also refuses a colon (a Windows drive
  prefix) and surrounding whitespace.
- **`RemoteOptions`**: fetch, push and clone take
  `&RemoteOptions { dirs, timeout }`. The library has no process-global
  XDG lookup, so the token store comes from the caller's `Dirs`, and tests
  pass a zero timeout instead of a context. `gittest::remote_options()` is
  the local-remote version.

## Tasks

Write each task's tests first, at task granularity. (Order as built: 0, 1,
6, 7's storage and parsing, 2–5, 7's wire, 8, 9, since fetch and push
return `syncerr` and resolve auth through `remoteauth`.)

### 0. Build and isolation (Q-SSH, Q-GIT-ISOLATION, Q-REMOTE-TIMEOUT)

- [x] Enable git2's `ssh` feature with libssh2 vendored, and its OpenSSL
      vendored as well. Run `make audit` and confirm that `deny.toml`
      allows libssh2-sys's license (BSD-3-Clause); record it in Q-SSH.
- [ ] Check a cold CI build on all three OSes. Not possible before the
      branch is pushed (pushing is the user's call); the first CI run of
      this branch is that check.
- [x] Add one `#[allow(unsafe_code)]` `Once` in `gitrepo`:
  - `set_search_path` points System, XDG, Global and ProgramData at an
    empty directory.
  - Set the server connect and read timeouts.
  - It runs before every repository open, including the opens inside
    `gittest`.
- [x] Test, in a child process with `HOME`, `XDG_CONFIG_HOME`,
      `USERPROFILE` and `APPDATA` pointing at a hostile global config.
      Assert first that libgit2 can see that config when isolation is
      off, so the test isn't vacuous. Then, with isolation on:
  - a hostile `url.<x>.insteadOf` doesn't redirect a fetch to a decoy
    bare repo
  - `core.autocrlf=true` doesn't touch a committed file
- [x] Add a note to Q-GIT-CRATE saying the isolation landed here, not in
      M1.

### 1. `gitrepo` errors and the dirty-tree guard — 1 + 2 rows

- [x] The typed `gitrepo::Error` described above. Existing M1/M4 callers
      and their tests stay green with assertions unchanged.
- [x] `TestRequireCleanWorkTreeTruncatesTheNamedPaths`: `dirty_paths`,
      and `require_clean_work_tree` naming a bounded number of paths.
- [x] `TestFastForwardAndMergeShareOneDirtyTreeSentinel`. Finish this
      after tasks 3 and 4.
- [x] `TestMergeRemoteRefusesADirtyWorkTree`. This is Go's correction:
      an automatic merge must never pick up a human's uncommitted edits.

### 2. Compare, ahead count, fetch and push — 5 + 2 rows

- [x] The `gittest` second-clone helper. It clones a `new_bare_remote`
      into a temp directory, commits there and pushes back, playing a
      second device.
- [x] `RemoteState` and `TestRemoteStateStringCoversAllStatesAndUnknown`.
      `Display` gives the exact words. ~~The `state(<n>)` fallback~~ is
      struck: a Rust enum can't hold an out-of-range value.
- [x] Rows:
  - `TestCompareReportsTheFourStates`
  - `TestCompareReportsRemoteBehind`
  - `TestCompareWithNoRemoteTrackingRefIsRemoteNone`
  - `TestHasRemoteOnADirectoryThatIsNotARepo`
- [x] Ahead count:
  - `TestAheadCountCountsAPlainRunOfCommits`
  - `TestAheadCountIsNotFooledByAMergeCommit`. Go's correction: count
    what the remote can't reach. A revwalk that hides the tracking ref
    does this.
- [x] `fetch` and `push`, with the default refspec, the
      `push_update_reference` check and the deadline in the progress
      callbacks. Beyond Go's rows:
  - a push to an up-to-date remote reports that nothing was sent
  - a fetch that brings nothing reports no change
  - each of the deadline callbacks aborts with an unreachable error when
    the deadline has passed. Prove this with a deadline of zero, not a
    real timeout. As built: git2's pack-builder and push-transfer progress
    callbacks return `()` and can't abort, so the deadline lives in
    transfer progress, sideband progress, push negotiation and the
    credentials callback. The test proves a zero deadline aborts a real
    fetch and a real push (nothing fetched, nothing sent); which callback
    fires first is libgit2's business.

### 3. Fast-forward and reset — 2 + 4 rows

- [x] `TestFastForwardOnlyMovesWhenItIsAFastForward`,
      `TestFastForwardRefusesOverUncommittedChanges`.
- [x] `reset_hard` returns the paths it discarded, and removes untracked
      files explicitly. Rows:
  - `TestResetHardDiscardsModifiedAndUntrackedFiles`
  - `TestResetHardOnACleanTreeDiscardsNothing`
  - `TestResetHardLeavesHEADWhereItWas`
  - `TestResetHardDiscardsChangesOutsideEntries`

### 4. File-level merge (Q-FILE-MERGE) — 4 rows

- [x] `prepare_merge`, `PendingMerge::commit` and `merge_remote`, which
      is the two composed with no choices. The merge works from the
      merge base and from tree diffs base→ours and base→theirs.
  - A path changed on both sides is a conflict, unless the two blob ids
    are identical.
  - Anything that conflicts writes nothing.
  - A partial failure rolls back, and always reaches the hard reset.
  - The merge commit has parents [local, remote] and the message
    `gage: merge origin`.
- [x] Rows:
  - `TestMergeRemoteMergesDisjointChanges`
  - `TestMergeRemoteConflictsAndChangesNothing`
  - `TestMergeRemoteTakesADeletionAndConflictsOnDeleteModify`
  - `TestSafeJoinRefusesPathsThatEscapeTheVault`. Tree paths come from
    whoever can push.
- [x] Beyond Go's rows: `.age-recipients` with one added line on each
      side conflicts *with no `.gitattributes` in the repository*. This
      is the property that `merge_trees` would have lost. The
      `Vault`-level row is M8a2's.

### 5. Clone — 3 rows

- [x] `clone(url, dir)`, with `remote_branch` choosing explicitly:
  - a single branch is chosen as-is
  - otherwise `main` or `master`
  - any other ambiguity is refused

  Uses `Remote::connect` + `list` and the same auth wiring as fetch.
- [x] Rows:
  - `TestCloneProducesAWorkingCopy`
  - `TestCloneWorksWhenTheRemoteHeadPointsNowhere`. A bare repo whose
    HEAD names a branch that doesn't exist.
  - `TestRemoteBranchPicksMainOrMasterAndRefusesAmbiguity`

### 6. `syncerr` (Q-SYNC-CLASSIFY) — 6 rows

- [x] `classify_push` and `classify_fetch` turn a `git2::Error`, or
      gage's deadline abort, into an `Unreachable`, `Auth` or `Diverged`
      error, or pass it through as Internal. Rows:
  - `TestClassifyPushSortsTheThreeBuckets`
  - `TestClassifyPushCatchesGoGitsRealRejection`: as
    `classify_push_catches_libgit2s_real_rejection`, from a real
    non-fast-forward push.
  - `TestClassifyFetchNeverInventsADivergence`
  - ~~`TestClassifyPassesNilThrough`~~: struck. The classifiers take an
    error, not an `Option` of one, so there is no nil to pass and an `Ok`
    never reaches them (noted in `syncerr_tests`).
  - `TestOfflineAndDivergedDoNotShareAnExitCode`
  - `TestClassifyIsIdempotentOnAnAlreadyClassifiedUnreachable`
- [x] The classification table from "Decisions to make first", written
      as the doc comment above the classifier, with the libgit2 version
      it was observed on.

### 7. `remoteauth` and the wire (Q-REMOTE-AUTH, Q-SSH) — 11 + 4 + 1 rows

- [x] Token storage:
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
- [x] URL parsing:
  - `host` handles both remote spellings: `https://host/...` and scp-like
    `user@host:path`
  - `token_host` returns a value only for HTTP(S) remotes
  - `auth_hint` names the host and `gage auth login`. A local path, which
    has no host, gets its own wording.

  Rows: `TestHostReadsBothRemoteSpellings`,
  `TestTokenHostOnlyNamesHTTPHosts`,
  `TestAuthHintNamesTheHostAndTheCommand`.
- [x] SSH: the alias heuristic (a host with no dot) and an ssh-agent
      credential. Rows:
  - `TestSSHRemoteNeedingSSHConfigFailsWithTheRealReason`
  - `TestSSHRemoteWithARealHostIsAttempted`
- [x] The loopback HTTP recorder in `gittest`, built on `std::net` with
      no new dependency:
  - it answers `401` with `WWW-Authenticate: Basic`
  - it records the `Authorization` header of the retry, then fails
  - it can also answer 403 or 404, for task 6
- [x] The `credentials` callback:
  - it offers a token only for the host the token was stored for
  - a second call within one operation fails, so a rejected token can't
    loop
- [x] Rows:
  - `TestFetchAndPushCarryTheStoredTokenToGoGit`: as `…ToLibgit2`, asserted
    on the wire for fetch (upload-pack) and push (receive-pack).
  - `TestFetchAndPushGoAnonymousWithNoStoredToken`
  - `TestPushAfterLogoutNamesTheHostAndTheCommand`
  - `TestNoTokenValueLeaksIntoASyncError`
  - `TestAuthFailuresNameTheHostAndTheFix`, from `remote_test`
- [x] Beyond Go's rows:
  - a token is not offered to a different host that the recorder
    redirects to
  - a rejected token makes exactly one retry, not a loop

### 8. `gage auth` — 12 rows

- [x] Register `auth` with `login`, `status` and `logout`, each taking
      `--host`, in the git group and available in both modes.
      Descriptions come from the fixtures. `login`'s long help covers
      fine-grained, single-repository scoping, and the token is prompted
      through `Prompter::value`, never taken as an argument.
- [x] Rows:
  - `TestAuthLoginStoresATokenAndStatusReportsIt`
  - `TestAuthLogoutForgetsTheToken`
  - `TestHostDefaultsToTheCurrentVaultsOrigin`
  - `TestAuthOnALocalRemoteSaysNoTokenIsNeeded`
  - `TestAuthStatusWithNoTokensAndNoVaultSaysSo`
  - `TestAuthStatusWithNoTokensFallsBackToTheCurrentVaultsHost`
  - `TestAuthLoginReadsTheOriginFromTheRepositoryWhenConfigLags`
  - `TestAuthLoginOnAVaultWithNoRemoteNamesTheVault`
  - `TestAuthCommandsWorkInBothModes`: the `auth` half, plus a real
    session run. Its `sync`/`pull`/`push`/`clone` half moves to M8a2,
    which adds those commands.
  - `TestNoTokenValueEverAppearsInOutput`: stdout, stderr and the session
    history file.
  - `TestExpiredTokenErrorNamesTheHostAndTheCommand`
  - `TestNoHostSpecificDependencies`: a `cargo metadata` check, beside
    the frontend-crate check. No host SDK, no OAuth or device-flow
    crate, and no client id string anywhere in `crates/`.
- [x] `p4_auth_commands_have_their_go_fixture_flags_and_descriptions`.
      Add the `auth` error paths to the `errorprefix` table.

### 9. Review, docs and ledger

- [x] Review again, on a fresh read: the wire-token tests (task 7), the
      hostile-config test (task 0), and the dirty-tree and ahead-count
      corrections (tasks 1–2). For each, mutation-check that the test
      goes red when its guard is removed. Record the result here.

  **Review record.** Each guard was removed in turn, its tests run, and the
  guard restored:

  | Guard removed | Red test(s) |
  |---|---|
  | credentials callback not installed | `fetch_and_push_carry_the_stored_token_to_libgit2` |
  | host check in the callback | `a_token_is_not_offered_to_a_host_it_was_not_stored_for`, `a_redirect_after_the_token_does_not_carry_it_to_another_host` |
  | one-offer limit (retry guard) | `fetch_and_push_carry_the_stored_token_to_libgit2` (asserts exactly one retry) |
  | `set_search_path` isolation | `hostile_git_config_is_ignored`, `isolation_empties_every_config_search_path` |
  | dirty-tree guard in `prepare_merge` | `merge_remote_refuses_a_dirty_work_tree`, `fast_forward_and_merge_share_one_dirty_tree_sentinel` |
  | dirty-tree guard in `fast_forward` | `fast_forward_refuses_over_uncommitted_changes`, `fast_forward_and_merge_share_one_dirty_tree_sentinel` |
  | `ahead_count` replaced by Go's original walk (stop only at the merge-base hash) | `ahead_count_is_not_fooled_by_a_merge_commit` (7, want 2) |
  | `push_update_reference` status ignored | `a_ref_the_remote_refuses_is_diverged_never_success` |
  | explicit untracked removal in `reset_hard` | `reset_hard_discards_modified_and_untracked_files` |

  One mutation stayed green, for a sound reason: hiding the merge *base*
  instead of the tracking ref. `Revwalk::hide` hides every ancestor of
  what it hides, so that variant counts correctly too. Go's bug was its
  hand-written walk, which is the mutation in the table above.
  The redirect tests were also checked to be non-vacuous: the redirected
  host is asserted to have received a request.
- [x] Parity ledger §4: `auth*`. The user docs (`git-and-auth.md`) wait
      for M8b, so they describe the whole shipped surface at once.
- [x] P4 index status column.

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
