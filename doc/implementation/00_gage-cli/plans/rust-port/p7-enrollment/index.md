# P7 Enrollment — implementation plan

[← port plan](../index.md) · Parent issue
[#63](https://github.com/distillerylabs/gage-rs/issues/63) · Roadmap
[#2](https://github.com/distillerylabs/gage-rs/issues/2) · previous:
[P6 Polish](../p6-polish/index.md)

At the start of P7, a new device joins a vault only by the hand-carried-key
path: `identity add` here, copy the public key across, `recipient add`
there. By the end of P7:

- `gage identity enroll` creates or reuses this device's identity and
  seals its device name and public key to a freshly generated one-time
  code. It commits the sealed request under `.gage/pending/`, pushes it,
  and prints the code. It fails outright if the remote can't be reached,
  because an unpublished request accomplishes nothing.
- An interactive `gage clone` of a vault this device can't read offers to
  enroll with `[Y/n]`, the one default-yes question in gage.
- `gage recipient pending` lists waiting requests from their filenames
  alone (no unlock, no code, no device names).
- `gage recipient approve [ID...] [--code ...]` opens requests with their
  codes and shows what each one claims. It asks, and only then unlocks.
  It then fetches, re-verifies each seal under the write lock, adds every
  approved device, re-encrypts the vault once and removes the requests,
  all in one commit.
- `gage recipient deny <ID>` removes a request without granting anything.
- Expired requests are pruned by every recipient write.

The spec is the copied
[gage-cli-init-design.md](../../../tdds/gage-cli-init-design.md). All
nine `D-ENROLL-*` decisions are settled there, and this plan cites them
rather than re-arguing them.

This plan does not restate the shared ground. The layering rules, test
conventions, crate choices and cross-milestone contracts are in the
[port plan](../index.md). Decisions are recorded in
[open-questions.md](../open-questions.md).

---

## Why this phase looks the way it does

Go built enrollment as six milestones, E0–E4. E0 (vault-id keying) and
E1a (unconditional re-encryption) were amendments to shipped milestones,
and gage-rs folded them into P1/P2 and M9b. The remaining four port one to
one and keep their Go IDs
([Q-P7-SPLIT](../open-questions.md#q-p7-split)):

- **E1b** widens M9b's one-recipient write to N recipients, plus the paths
  to delete in the same commit and the caller's failed-push clause. It is
  a pure refactor whose test is that nothing observable changes. It goes
  first because it is library work on the most crash-sensitive path in
  the tree.
- **E2** is the sealed request, library only: the code, the seal, the
  filename scheme, expiry, opening, resolution and pruning. It is
  isolated the way M2 isolated crypto. The code is an offline-attackable
  secret and the seal is what makes the whole feature mean anything.
- **E3** is the joining side: `Vault::enroll`, `identity enroll`, and
  `clone`'s offer.
- **E4** is the approving side: three commands, the late unlock, the
  atomic batch approval, and the phase's user docs, so they describe
  shipped behavior.

E2 doesn't depend on E1b logically, and Go worked them in parallel. Here
they are stacked in Go's order, to keep one linear branch stack.

Go's plan went through four review passes, and every correction it
recorded is already in its milestone docs. Each Rust milestone doc carries
those corrections over as tests. Planning found these points, specific to
this tree, that Go's documents don't make:

- **The lock-held sync functions already exist.** Go's E3/E4 docs warn at
  length that enroll and approve must use the unexported `v.pull`/`v.push`,
  because `Pull`/`Push` take the vault lock themselves and the lock is not
  re-entrant. The gage-rs equivalents are `pull_locked` and `push_locked`
  (`crates/gage/src/remotesync.rs`). The public `pull()`/`push()`/
  `try_pull` are the ones to avoid.
- **gage-rs's write-path push only warns.** `push_after_write` never
  returns an error, and `push_locked` handles a rejected push itself by
  fetching and then fast-forwarding or merging. Enroll's "unreachable
  remote fails the command" and "no remote fails with its own error"
  therefore need explicit calls to the syncer, not error propagation from
  the existing write tail.
- **A diverged fast-forward is a value, not an error, here too.**
  `gitrepo::compare` returns `RemoteState::Diverged`. Enroll and approval
  must check for it, the same trap as Go's nil error with
  `SyncReport.Diverged` set.
- **P2's scrypt guards constrain E2's seal.** `crypt.rs`'s source-scan
  tests require exactly one `scrypt::Recipient::new` and one
  `scrypt::Identity::new` site in the library. Two of them already say
  their enrollment half is P7's. The seal is therefore built through the
  existing constructors at an explicit factor, and the guards are extended
  rather than given exceptions
  ([Q-ENROLL-SCRYPT-HOOK](../open-questions.md#q-enroll-scrypt-hook)).
- **A trait method without a default body is a compile break everywhere.**
  Go's `ConfirmDefaultYes` reached its embedding wrappers silently, and
  Go's E3 needed a test to catch that. Here
  `Prompter::confirm_default_yes` has no default, so every wrapper
  (`delegate_the_rest!`, `NoChoosePrompter`, `AssumeYes`) has to say what
  it does with it. E3 still ports Go's row
  ([Q-CONFIRM-DEFAULT-YES](../open-questions.md#q-confirm-default-yes)).
- **`clone::run` takes no prompter yet.** The offer needs one threaded in
  from `commands/mod.rs`, as `init` already has.
- **Go's `context.Context` parameters have no analogue.** `Vault.Enroll(ctx, …)`
  and `ApproveEnrollments(ctx, …)` take one because their fetch is a hard
  precondition. In gage-rs the deadline already lives in the
  `RemoteSyncer` calls, as P4's sync verbs do it.
- **`recipient pending` renders local time.** Go's test sets `TZ`. Here the
  zone reaches the command through `Io`, so no test touches the process
  environment ([Q-PENDING-TZ](../open-questions.md#q-pending-tz)).

## Resolved decisions

| ID | Decision |
|---|---|
| [Q-P7-SPLIT](../open-questions.md#q-p7-split) | Go's E1b, E2, E3 and E4, one to one. User docs land in E4. No `context.Context` analogue. |
| [Q-ENROLL-SCRYPT-HOOK](../open-questions.md#q-enroll-scrypt-hook) | The seal's work factor is a shipped const (14), a live atomic and its own test-only setter. It goes through the existing single scrypt constructors, so P2's guards are extended rather than excepted. The open path caps a claimed factor at `SCRYPT_MAX_WORK_FACTOR`. |
| [Q-CROCKFORD](../open-questions.md#q-crockford) | The code is hand-rolled Crockford base32 in the library: 16 characters from 80 bits of `OsRng`, `GAGE-` prefix. No new crate. |
| [Q-CONFIRM-DEFAULT-YES](../open-questions.md#q-confirm-default-yes) | `Prompter::confirm_default_yes` has no default body, and every wrapper delegates. |
| [Q-ENROLL-AMBIGUOUS](../open-questions.md#q-enroll-ambiguous) | `resolve_enrollment` returns its own typed candidate list (request ids and expiries), not M5's `CandidateList`. |
| [Q-PENDING-TZ](../open-questions.md#q-pending-tz) | `recipient pending`'s local time zone comes through `Io`. |
| [Q-ENROLLMENT-CODE-HISTORY](../open-questions.md#q-enrollment-code-history) | Go's accepted risk carries over. `approve --code` lands in shell and session history; leaving out `--code` uses the masked prompt. |

## Milestones

Legend: `[ ]` not started, `[~]` in progress, `[x]` done.

| # | Milestone | Model | Depends on | Inventory rows | Status |
|---|---|---|---|---|---|
| E1b | [The shared recipient write](e1b-shared-recipient-write.md) ([#64](https://github.com/distillerylabs/gage-rs/issues/64)) | Sonnet\* ⚑ | P6 | 15 | `[ ]` |
| E2 | [The sealed request](e2-sealed-request.md) ([#65](https://github.com/distillerylabs/gage-rs/issues/65)) | **Opus** ⚑ | E1b | 55 (+2 inbound) | `[ ]` |
| E3 | [Joining a vault](e3-joining-side.md) ([#66](https://github.com/distillerylabs/gage-rs/issues/66)) | Sonnet\* ⚑ | E2 | 60 (+2 inbound) | `[ ]` |
| E4 | [Approving a device](e4-approving-side.md) ([#67](https://github.com/distillerylabs/gage-rs/issues/67)) | **Opus** ⚑ | E3 | 71 | `[ ]` |

`⚑` means an Opus review pass over that milestone's named tests before it
is called done:

- **E1b:** the atomicity rows. A failure before, inside or after the
  deletion step must leave HEAD untouched. A lost guarantee here is
  invisible until a crash.
- **E2:** the seal, code, work-factor, size-bound and inertness rows. A
  round trip proves nothing about authentication, and a weak seal passes
  every functional test.
- **E3:** the lock held across the pull, the dirty-tree reset before the
  pull, the divergence-is-a-value refusal, and "no wrapper answers yes".
  Each failure looks like an unrelated remote or concurrency bug.
- **E4:** what is written comes from the re-verification under the lock;
  the unlock is placed after the confirmation and before the pre-flight;
  a malformed payload is never rendered.

`*` means borderline: start on Sonnet, and switch at the milestone boundary
if it turns awkward. E1b is Go's own recommendation. E3 moves up from
Go's Sonnet because of the traps listed above.

## Inventory accounting

P7 owns 201 rows of `tests/fixtures/go/test-inventory.tsv`. Planning
re-tags none of them out. Each Go file goes to the milestone that owns the
function its tests call:

| Milestone | Files (rows) |
|---|---|
| E1b | `recipient_batch` (15) |
| E2 | `enrollcode` (7), `enrollseal` (18), `enrollpayload` (6), `enrollexpiry` (6), `enrollinert` (4), `enrollname` (4), `enrollment` (9), `enrollprune` `TestPruningLeavesLiveRequestsAlone` (1) |
| E3 | `enroll` (10), `enrollcollide` (5), `enrollremote` (10), `enrollcrash` (4), `enrollmerge` (4), `enroll_cli` (18), `clone_enroll` (8), `enroll_prompter` (1) |
| E4 | `enrollapprove` (14), `enrollapproveatomic` (9), `enrollapprovefetch` (8), `enrolldeny` (6), `enrollprune` add/remove rows (2), `approve_cli` (19), `approve_scope_cli` (13) |

`enrollprune` is split because Go wires pruning into `recipient add` and
`remove` in E4. The prune primitive and its own row are E2's.

Rows that earlier phases left to P7:

| From | Row | Claimed by |
|---|---|---|
| P2 · M2 | The enrollment halves of `crypt` `TestOnlyTheTestHooksWriteTheWorkFactors` and `TestEveryScryptIdentityCapsItsWorkFactor` (ledger §2 `e52828a`) | E2 |
| P6 · M12a | The `HeadCommitterTime` half of `gitrepo/history` `TestLogAndHeadCommitterTimeOnARepoWithNoCommitsYet` | E3 |
| P2 → "P4 · M8a" | `prompter` `TestConfirmDefaultYesDefaultsToYes`. P2's index sent it to clone's offer under M8a. P4 kept the offer out of scope and never claimed the row, so it lands with the offer here. | E3 |

Obligations earlier phases left to P7 that aren't inventory rows:

| From | Obligation | Claimed by |
|---|---|---|
| P3 · M6 | `use` on a vault with no local identity names `identity enroll` in its hint | E3 |
| P3 · M13 | `enroll` among the `identity` children in completion | E3 |
| P3 · M13 | Enrollment-request ids as completion candidates for `approve`/`deny` (ledger §6 `addressing-entries.md`) | E4 |
| P4 · M8a2 | `clone`'s enrollment offer, and the `identity enroll` line in its access message | E3 |
| P5 · M9a | The `identity` and `recipient` parent-help child lists (`p5_identity_commands_have_their_go_fixture_flags_and_descriptions` lists only the children that exist) | E3 (`identity`), E4 (`recipient`) |
| P5 · M9b | The `add_recipient_locked`/`commit_recipient_list` widening that M9b deliberately left out | E1b |
| P5 · M11 | The enrollment sections of `identities-and-recipients.md` and `sharing.md` | E4 |
| P4, P3 docs | The enrollment exceptions in `sync.md` and `git-and-auth.md`, and enrollment's session behavior in `session-mode.md` | E4 |

Already done, so not claimed:

- Go's A22 (`pending/` in the base design doc's layout) is in the copied
  [gage-cli-design.md](../../../tdds/gage-cli-design.md), which Go
  updated when E3 landed. Go E3's "one documentation change" needs
  nothing here.
- `vaultid_keying` `TestVaultRemoveOnAPendingEnrollmentAsksBeforeDeleting`
  was ported by M12b, which simulated the pending state with `identity
  add`. E3 adds a test with a real pending request beyond Go's rows.

## Process notes

- Branches stack: `main` <- `63-p7-enrollment` (these docs) <- E1b <- E2
  <- E3 <- E4. Each milestone branch is cut from the previous one when the
  user says to start it.
- Tests come first, at task granularity. Each task's tests are written and
  implemented to green before the next task starts.
- **Quiet scope narrowing** is the failure mode to watch: most of a list
  done and the milestone reported done. Check the boxes and the claimed
  inventory rows, not a summary. A test that turns out not worth writing
  is struck from the list *in the doc*, with a one-line reason.
- **Go's milestone docs are the test lists' source**, at
  `doc/implementation/00_gage-cli/plans/gage-cli-init-design/` in the
  [Go repo](https://github.com/distillerylabs/gage/tree/64b6cf078f3e756efb9d868cc1518dc1d9060748/doc/implementation/00_gage-cli/plans/gage-cli-init-design).
  Each bullet there names a premise a passing test could have confirmed
  wrongly. When a Rust row's port seems redundant, re-read the Go bullet
  before striking it.
- **No network.** Remote behavior runs against `gittest` bare repos. An
  unreachable remote comes from the injected `RemoteSyncer`
  (`Vault::with_remote_syncer`), never a real timeout. Lock contention is
  proven with child processes, as M4's lock tests are.
- **The one wall-clock assertion** (E2's "a wrong code against a full
  directory is fast", at the shipped factor) gets a generous ceiling. Only
  a return to factor 19, a ~32× regression, should trip it. Windows is the
  slow runner.
- **Every new command or flag** gets a registry entry, its
  `commands/mod.rs` edits, and a row in a
  `p7_*_have_their_go_fixture_flags_and_descriptions` test, following the
  P4–P6 pattern.
- **Exit codes are part of every milestone's definition of done.** The
  mapping for every enrollment error is one table in the TDD's "Library
  surface". A milestone that adds an error adds its code and a test
  pinning it.
- Each milestone ends green on `make lint` and `make test`, locally and on
  all three CI platforms.
- If context runs low mid-milestone, write a `*-sessionhandoff.md` here.
