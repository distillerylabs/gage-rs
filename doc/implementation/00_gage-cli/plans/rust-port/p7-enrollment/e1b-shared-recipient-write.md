# E1b — The shared recipient write

[P7 plan](index.md) · [#64](https://github.com/distillerylabs/gage-rs/issues/64) · next: [E2 — The sealed request](e2-sealed-request.md)

> **Recommended model: Sonnet\* ⚑.** The change is additive: one function
> takes a slice where it took a value, and the commit gains two options.
> But it is M9b's central write path, and the property it must not lose is
> crash-safety: a failure partway leaves HEAD untouched. Run an **Opus
> review of the atomicity rows** (task 3) before calling it done. Switch
> to Opus outright if `commit_recipient_list` resists the extra paths.

## Goal

Give `add_recipient`'s body an N-recipient form. Adding several
recipients is then **one** lock, **one** trust question, **one**
re-encryption pass and **one** commit, and that commit can also delete
files handed to it.

- `add_recipient_locked` takes a slice of recipients. Its duplicate check
  covers the batch against itself as well as against the vault's list.
- `commit_recipient_list` takes the paths to delete in the same commit and
  the clause its failed-push warning should say. Both arrive in one
  options struct, not two mechanisms added in two sittings.
- `add_recipient` becomes the one-recipient caller. Its signature, its
  output and its commit message are unchanged.
- **No CLI surface changes.** No batch `recipient add` command; the form
  is crate-private and E4's approval is its only other caller.

## Depends on

- **P5 · M9b:** `add_recipient`, `add_recipient_locked`,
  `commit_recipient_list`, `reencrypt_to`, `write_recipient_files`,
  `with_vault_write`, and the `reencrypt_hook` crash seam. M9b split
  `add_recipient_locked` out "so E1b can widen it to several recipients
  and E4's approval can call it under a lock it took itself"
  (`crates/gage/src/recipient.rs`).
- **P5 · M10:** `confirm_recipient_trust`, `note_recipients_reviewed`.
- **P4 · M8a2:** `push_after_write` and `UNPUSHED_WRITE_CLAUSE`
  (`crates/gage/src/remotesync.rs`), and the fake `RemoteSyncer`.

## Design references

- Init TDD, "Approving several devices at once is one re-encryption pass".
- Init TDD, "Approval always re-encrypts".
- Init TDD, "Approval's failed push needs the same treatment": the
  message E4 passes through this milestone's clause.

## Go sources

- Go plan `gage-cli-init-design/e1b-shared-recipient-write.md`: all of it.
- Parity ledger: §2 `045bf24`, `9d72384` (#48); §3 E1b.
- Go sources at the pinned SHA: `internal/gage/recipient.go`
  (`addRecipientsLocked`, `recipientWrite`, `commitRecipientList`),
  `internal/gage/remotesync.go` (`pushAfterWriteSaying`).
- Test inventory, 15 rows: `internal/gage/recipient_batch_test.go`. These
  were tagged P5 by file name and re-tagged here
  ([Q-E1B-RETAG](../open-questions.md#q-e1b-retag)).

## Decisions to make first

- **The options struct's shape.** Go bundles the extra arguments into
  `recipientWrite`. Here, one struct passed to `commit_recipient_list`
  carries the recipients' list, the commit message, `delete_paths:
  &[PathBuf]` (vault-relative, validated through `gitrepo::safe_join`)
  and `push_clause: Option<&str>`. `None` means `UNPUSHED_WRITE_CLAUSE`.
  `remove_recipient` builds it with no delete paths and no clause. Decide
  whether `add_recipient_locked` takes the struct too or keeps separate
  parameters.
- **The push sibling.** `push_after_write(prompter)` keeps its signature
  and delegates to a new `push_after_write_saying(prompter, clause)`. The
  divergence branch (the report's summary) is shared and ignores the
  clause.
- **Where the deletion runs.** The working tree is written first, then
  everything lands in one commit. The deletions run after the
  re-encryption and the recipient files are written, and before
  `commit_all`. A failure after a deletion leaves the deletion
  uncommitted rather than undone, and the next write's dirty-tree reset
  restores it. Confirm `gitrepo::commit_all` stages deletions; if it
  doesn't, that is the place to fix, not a second commit.
- **The crash seam.** The atomicity rows need a failure inside and after
  the deletion step. Extend the existing test-only `reencrypt_hook`
  (`vault.rs`) with the extra points rather than adding a parallel seam.
  Go's note applies: the seam has to leave the tree stageable (corrupting
  `.gage/config.toml` does; replacing `.age-recipients` with a directory
  doesn't). Otherwise "nothing was committed" is the seam's doing, not
  the code's.

## Tasks

Write each task's tests first, at task granularity. Rows go in
`crates/gage/src/recipient_tests.rs` (or a new `recipient_batch_tests.rs`
beside it, if that reads better).

### 1. The N-recipient form — 9 rows

- [ ] Widen `add_recipient_locked` to a slice; `add_recipient` passes a
      one-element slice, no delete paths and no clause.
- [ ] Rows:
  - `TestSeveralRecipientsLandInOneCommit`: `several_recipients_land_in_one_commit`. Every one can read every pre-existing entry. Exercised at the library level; E4 is what puts a command on it.
  - `TestOneReencryptionPassForTheBatch`: `one_reencryption_pass_for_the_batch`. Assert the count of entry rewrites through the hook, not wall-clock time.
  - `TestOneTrustQuestionForTheBatch`: `one_trust_question_for_the_batch`. `confirm_recipient_change` is asked at most once.
  - `TestBatchTakesOneLockAndOneDirtyTreeReset`: `batch_takes_one_lock_and_one_dirty_tree_reset`. The discard warning appears once.
  - `TestADuplicateWithinTheBatchIsRefused`: `a_duplicate_within_the_batch_is_refused`. The same pubkey twice, or two members claiming one device name, refused before anything is written.
  - `TestABatchMemberAlreadyInTheVaultIsRefused`: `a_batch_member_already_in_the_vault_is_refused`. The pre-existing `RecipientExists`, unchanged.
  - `TestAddRecipientsLockedRevalidatesEveryBatchMember`: `add_recipients_locked_revalidates_every_batch_member`. Device name and key validation run per member under the lock.
  - `TestAddRecipientIsTheOneRecipientCaller`: `add_recipient_is_the_one_recipient_caller`. Same commit message (`gage: recipient add {device} (reencrypt)`), trust question and cache regeneration as before.
  - `TestExtraPathsAreDeletedInTheSameCommit`: `extra_paths_are_deleted_in_the_same_commit`. Use an ordinary file, not a pending request, so this milestone doesn't depend on E2.

### 2. The failed-push clause — 2 rows

- [ ] `push_after_write_saying`, with `push_after_write` delegating to it.
- [ ] Rows (injected fake `RemoteSyncer`):
  - `TestFailedPushClauseIsTheCallersToSay`: `failed_push_clause_is_the_callers_to_say`. A supplied clause reaches the warning, and `add_recipient`, which supplies none, produces exactly M9b's message.
  - `TestDivergedPushIgnoresTheSuppliedClause`: `diverged_push_ignores_the_supplied_clause`. A divergence warns with the report's summary whatever the clause.

### 3. Atomicity — 4 rows ⚑

- [ ] Extend the crash seam per "Decisions to make first".
- [ ] Rows:
  - `TestInterruptedBatchLeavesHEADUntouched` ⚑: `interrupted_batch_leaves_head_untouched`. No recipients added, no entries rewritten, no paths deleted.
  - `TestABatchWhoseDeletionFailsCommitsNothing` ⚑: `a_batch_whose_deletion_fails_commits_nothing`.
  - `TestABatchThatFailsAfterDeletingCommitsNothing` ⚑: `a_batch_that_fails_after_deleting_commits_nothing`. The deletion is uncommitted, not undone. No commit is salvaged, and the next write's reset restores the path rather than folding it into that write's commit.
  - `TestADeclinedTrustQuestionAbortsTheWholeBatch` ⚑: `a_declined_trust_question_aborts_the_whole_batch`. No commit, no partial re-encryption, no cache regeneration.

(Rows: 9 + 2 + 4 = 15.)

### 4. Nothing else moved

- [ ] M9b's `recipient add` and `recipient remove` suites
      (`recipient_tests.rs`, `reencrypt_tests.rs`, `recipient_cli_tests.rs`)
      pass **unmodified**. That is this milestone's primary assertion. A
      test "updated" to match is a regression, not a fix.
- [ ] `remove_recipient` is not a caller of the new form, and its
      behavior is unchanged.

### 5. Opus review

- [ ] Opus review of task 3's ⚑ rows, by mutation. Apply each mutation
      below, run the suite, and record what killed it. A survivor is a
      missing test.

| Mutation | Killed by |
|---|---|
| Deletions staged and committed in a second commit | |
| A failure after deletion commits what was written so far | |
| Duplicate check skips the within-batch comparison | |
| Trust question asked per member | |
| `push_after_write_saying` ignores the clause | |
| The clause replaces the divergence summary | |

### 6. Parity and ledger

- [ ] Parity ledger §2 `045bf24`/`9d72384` and §3 E1b point at this
      milestone, and are marked `[x]` after CI.
- [ ] Update the port plan's cross-milestone contract row for M9b ("E1b
      widens the primitive") to name what shipped.
- [ ] Mark E1b `[x]` in the [P7 index](index.md) after CI.

## Definition of done

Every box above is checked. All 15 claimed rows are ported, M9b's suites
pass unmodified, and the review table is filled in. `make lint` and
`make test` are clean locally and on Linux, macOS and Windows CI.

The milestone is done when:
- no user-visible behavior differs from before it
- a crate-private call can add N recipients and delete M paths in one
  commit, or leave HEAD exactly where it was
