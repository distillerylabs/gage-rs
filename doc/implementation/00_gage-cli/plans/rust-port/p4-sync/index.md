# P4 Sync — implementation plan

[← port plan](../index.md) · Parent issue
[#32](https://github.com/distillerylabs/gage-rs/issues/32) · Roadmap
[#2](https://github.com/distillerylabs/gage-rs/issues/2) · previous:
[P3 Query & session](../p3-query-session/index.md)

At the start of P4 a vault can have an `origin`, but nothing fetches from
it or pushes to it, and there is nothing to authenticate with. By the end
of P4:

- Every unlock fetches from the remote and fast-forwards.
- Every write is pushed.
- `sync`, `pull`, `push`, `clone`, `auth login|status|logout` and
  `init --remote` exist.
- A divergence where the two sides touched different entries merges
  without asking.
- A divergence where both sides changed the same entry is resolved
  interactively: keep local, keep remote or keep both.
- A divergence in `.age-recipients` or `.gage/config.toml` is always a
  conflict, never a union.

P4 does not include the recipient commands or the trust cache (P5), and
it does not include `log`/`history` or `--script` (P6).

This plan does not restate the shared ground: layering rules, test
conventions, crate choices and cross-milestone contracts live in the
[port plan](../index.md). Decisions are recorded in
[open-questions.md](../open-questions.md).

---

## Why this phase looks the way it does

P4 ports Go's M8a (transport and divergence detection) and M8b (conflict
resolution).

**Three milestones instead of Go's two.** Go's M8a claims about 111 rows
here, too many to review as one change. It splits cleanly by layer:

- **M8a1** builds the library's remote primitives and token auth. They are
  testable against bare repos and a loopback recorder, with no `Vault` or
  `Session` changes.
- **M8a2** wires those primitives into unlock, writes, `Session` and the
  CLI.

M8b keeps Go's scope. All three run on Opus: each involves security
ordering, credential handling or merge correctness.

Planning found four things that Go's documents don't say, because they
are specific to libgit2 or to this repository:

- **The P1 config isolation never landed.**
  [Q-GIT-CRATE](../open-questions.md#q-git-crate) found that libgit2
  honors the user's `url.<base>.insteadOf`, and it assigned an isolation
  step to M1: `set_search_path` behind a `Once`, with a test that a hostile
  `insteadOf` is ignored. `gitrepo.rs` has no such call. The gap was
  harmless while gage-rs never contacted a remote. From M8a1 onward it
  would let a user's global config redirect a vault's push.
- **`git2::merge_trees` would weaken a security property.** libgit2 merges
  text files line by line. Two devices that each add a recipient would be
  unioned, and the only thing stopping that would be the committed
  `.gitattributes`. Go's guarantee doesn't depend on that file, because
  `gitrepo.MergeRemote` is gage's own file-level merge. gage-rs keeps the
  file-level merge ([Q-FILE-MERGE](../open-questions.md#q-file-merge)).
- **libgit2 types the cases go-git leaves untyped.** A non-fast-forward
  push arrives as `ErrorCode::NotFastForward`, and as a rejection through
  `push_update_reference`. Go classifies divergence by elimination because
  go-git doesn't wrap its sentinel. gage-rs detects it positively and keeps
  elimination as the fallback
  ([Q-SYNC-CLASSIFY](../open-questions.md#q-sync-classify)).
- **libgit2 has no `context`.** Timeouts are libgit2's own server timeouts
  plus a deadline checked in the progress callbacks
  ([Q-REMOTE-TIMEOUT](../open-questions.md#q-remote-timeout)).

## Resolved decisions

| ID | Decision |
|---|---|
| [Q-SSH](../open-questions.md#q-ssh) | Add git2's `ssh` feature (vendored libssh2) for Go's best-effort ssh-agent auth. Credentials come only from `Cred::ssh_key_from_agent`, and libgit2's exec-`ssh` backend is never used. A remote that relies on a `~/.ssh/config` host alias is refused with the real reason. Amends Q-GIT-CRATE's "No `ssh`". |
| [Q-GIT-ISOLATION](../open-questions.md#q-git-isolation) | `set_search_path` points System, XDG, Global and ProgramData at an empty directory, behind a `Once` that runs before every repository open. A child-process test proves a hostile `insteadOf` and `core.autocrlf` are ignored. Lands in M8a1, before the first network operation. |
| [Q-SYNC-CLASSIFY](../open-questions.md#q-sync-classify) | Go's three buckets and exit codes: unreachable 7, auth 5, diverged 1. A `git2::Error` is classified by class and code, never by its message text. Divergence is detected positively, and any other push failure falls to diverged by elimination. A fetch failure outside the buckets passes through as Internal. |
| [Q-REMOTE-TIMEOUT](../open-questions.md#q-remote-timeout) | `REMOTE_OP_TIMEOUT` is 30s and public. libgit2's connect and read timeouts are set, and a per-operation deadline is checked in the progress callbacks. There is no cancellation token. |
| [Q-FILE-MERGE](../open-questions.md#q-file-merge) | gage's own file-level three-way merge, computed from tree diffs. `merge_trees` is never used. `merge_remote` is `prepare_merge` followed by `PendingMerge::commit` from M8a1 on. |
| [Q-REMOTE-AUTH](../open-questions.md#q-remote-auth) | The token is supplied by the `credentials` callback, not `custom_headers`. The callback checks the host, and one rejected attempt ends the operation. Redirects are followed on the initial request only. Tokens are stored at `$GAGE_STATE/tokens/<host>` with mode 0600. Tests use a loopback HTTP recorder built on `std::net`. |
| [Q-CONFLICT-PROMPTER](../open-questions.md#q-conflict-prompter) | `Prompter::resolve_conflict` has a default implementation that returns `PromptError::NotInteractive`, so refusing is the default. |
| Phase map | `gitrepo/history_test.go` (6 rows) is re-tagged to P6 · M12. Go's `gitrepo.Log` shipped with Go's M12. |
| Flag scope | `clone --device` is ported in M8a2 for the device name. Go's enrollment offer on `clone` (`clone_enroll_test`) is P7's. `init --no-recovery-key` and `--recovery-key-out` stay with P8. |

## Milestones

Legend: `[ ]` not started, `[~]` in progress, `[x]` done.

| # | Milestone | Model | Depends on | Inventory rows | Status |
|---|---|---|---|---|---|
| M8a1 | [Remote primitives & auth](m8a1-remote-primitives.md) ([#33](https://github.com/distillerylabs/gage-rs/issues/33)) | **Opus** | P3 | 57 | `[x]` |
| M8a2 | [Sync wiring](m8a2-sync-wiring.md) ([#34](https://github.com/distillerylabs/gage-rs/issues/34)) | **Opus** | M8a1 | 54 | `[ ]` |
| M8b | [Conflict resolution](m8b-conflict-resolution.md) ([#35](https://github.com/distillerylabs/gage-rs/issues/35)) | **Opus** | M8a2 | 48 | `[ ]` |

Each milestone runs on Opus throughout, so there is no separate `⚑`
review pass. Each milestone doc names the tests that most need a second
look at the end:

- **M8a1:** the token-on-the-wire tests and the hostile-config test.
- **M8a2:** the lock-ordering tests.
- **M8b:** the no-secret-lost tests.

## Inventory accounting

P4 owned 172 rows of `tests/fixtures/go/test-inventory.tsv`. Planning
re-tagged 6 of them to P6, which leaves 166. P1 · M1 already claimed 7 of
those. The other 159 are claimed by M8a1 (57), M8a2 (54) and M8b (48).
By file:

| Milestone | Files (rows) |
|---|---|
| M8a1 | `gitrepo_test` (1 of 8), `remote_test` (15), `reset_test` (4), `worktree_state_test` (4), `syncerr` (6), `remoteauth` (11), `auth_wiring` (4), `auth` CLI (12) |
| M8a2 | `remotesync` (18), `sync_index` (3), `sync_cli` (16), `clone` CLI (13), `init_remote_auth` (4) |
| M8b | `syncresolve` (30), `syncresolve_cli` (8), `merge_test` (10) |

Rows already claimed by an earlier phase:

| Claimed by | Rows |
|---|---|
| P1 · M1 | `gitrepo_test`: `TestInitAndCommitCreatesExactlyOneCommit`, `TestSetRemoteThenChangeIt`, `TestRemoteURLEmptyWhenUnset`, `TestRemoteURLAndHasRemoteWithNoConfiguredURLs`, `TestHasRemoteOnARepoWithNoOriginConfigured`, `TestIsCleanDetectsDirtyWorkingTree`, `TestIsCleanCountsUntrackedFilesAsDirty` |

Rows re-tagged out of P4:

| Destination | Rows | Why |
|---|---|---|
| P6 · M12 | `gitrepo/history_test.go` (6): `TestLogAcrossAConflictResolvedByTakingASide`, `TestLogAcrossAMergeOfUnrelatedEdits`, `TestLogAndHeadCommitterTimeOnARepoWithNoCommitsYet`, `TestLogOnAPathGitHasNeverSeenIsEmpty`, `TestLogReportsADeletionOnce`, `TestLogReportsEachRealChangeOnce` | They test `gitrepo.Log`, which Go added in M12 for `log`/`history`. M8b still proves that a merge commit can be walked (`TestHistoryWalksThroughAMergeCommit`, `TestLocalCommitGranularitySurvivesTheMerge`). M12's two merge rows walk the merge commits M8b creates. |

No rows move into P4 from earlier phases. Earlier phases left these items
to P4:

- `ddce8b1`'s warning half: push warnings name gage once (M8a2).
- `.gitattributes` content: M1 writes it, and M8a2 claims the Go row.
- Q-INIT-INTERIM's `--remote` (M8a2).

## Process notes

- Branches stack: `main` <- `32-p4-sync` (these docs) <- M8a1 <- M8a2 <- M8b.
  Each milestone branch is cut from the previous one when the user says to
  start it.
- Tests come first, at task granularity. Each task's tests are written and
  then implemented to green before the next task starts.
- The failure mode to watch is quiet scope narrowing: most of a list done
  and the milestone reported done. Check the boxes and the claimed
  inventory rows, not a summary. A test that turns out not worth writing is
  struck from the list *in the doc*, with a one-line reason.
- Go's M8a "Corrections made during review" each name a test that was
  ticked before the behavior under it was right. The Rust milestone docs
  list each correction as an explicit test:
  - `AheadCount` through a merge
  - auth that never reaches the transport
  - offline and diverged sharing an exit code
  - a merge that sweeps up uncommitted edits
- **No network.** `make test` stays offline:
  - Remotes are local bare repos created by `gittest`.
  - "Unreachable" comes from an injected `RemoteSyncer`, or from a
    loopback port that refuses connections.
  - Auth is observed on a loopback HTTP recorder.
  - SSH stops at the ssh-agent attempt, and the test pins which path was
    taken.
- M8a1 adds libssh2 to a vendored C build that runs on three OSes.
  Before M8a1's first test task, check that the CI build time and
  `make audit` (libssh2-sys advisories and license) are still acceptable.
- Each milestone ends green on `make lint` and `make test`, locally and on
  all three CI platforms.
- If context runs low mid-milestone, write a `*-sessionhandoff.md` here.
