# E4 — Approving a device

[← E3](e3-joining-side.md) · [P7 plan](index.md) · [#67](https://github.com/distillerylabs/gage-rs/issues/67) · next: P8 Recovery key

> **Recommended model: Opus ⚑.** Three things at once, each easy to get
> subtly wrong:
> - an atomic multi-request commit
> - an unlock in the *middle* of the command rather than around it
> - a re-verification under the lock of something already shown to a human
>
> Run an **Opus review of the re-verification, late-unlock-ordering and
> never-render-a-malformed-name rows** (task 9) before calling it done.

## Goal

`gage recipient pending`, `approve` and `deny`: list what is waiting,
authenticate a request with an out-of-band code, and on confirmation add
the device as a recipient, re-encrypt the vault and clear the request, all
in one commit. This is the milestone that makes enrollment grant access.

- **`recipient pending [-u NAME]`:** no unlock, no code, id and local-time
  expiry only. Device names are sealed and never shown. Exits 0 with a
  message when nothing is pending.
- **`recipient approve [ID...] [--code CODE]... [--device NAME] [-u NAME]`**,
  ordered:
  1. Gather the codes: `--code`, or else `Prompter::value` with a retry
     loop owned by `gage-cli`.
  2. Open: the scope is the `pending_enrollments()` result, or the
     `resolve_enrollment` match for each ID.
  3. The collision pre-check (a library pass): `NameTaken`, before any
     unlock.
  4. Render what each request claims, then `confirm` (`[y/N]`).
  5. **Unlock.**
  6. `require_full_access`: `CannotGrantFullAccess`, still before the lock
     and M10's prompt.
  7. Lock.
  8. Fetch through `pull_locked`. A divergence refuses with the ordinary
     `gage sync` advice; an unreachable remote warns and proceeds.
  9. **Re-open each seal under the lock**; what is written comes from
     that open.
  10. One E1b write: add the new recipients, re-encrypt, delete the
      approved requests' files, prune.
  11. Commit.
  12. Push, warning with approval's own clause if it fails.

  A request whose pubkey is already listed is a no-op that clears it.
  An all-no-op batch takes a clear-only commit: no re-encryption, no
  trust question.
- **`recipient deny <ID> [-u NAME]`:** no code, no unlock. It takes the
  write lock, deletes the file, prunes, commits, and warns if the push
  fails.
- **Pruning rides every recipient write:** `add`, `remove`, `approve` and
  `deny`, in the same commit.
- **The phase's user docs.**

## Depends on

- **E1b:** the lock-held N-recipient body, its delete paths and its
  failed-push clause.
- **E2:** `pending_enrollments`, `open_enrollment`, `resolve_enrollment`,
  pruning, `EnrollmentError`. E4 reimplements none of them.
- **E3:** `Vault::enroll` builds this milestone's fixtures.
- **P5 · M9b:** `require_full_access`, `RecipientExists`,
  `SelfRemovalDeclined`'s shape for a declined confirmation.
- **P5 · M10:** the trust cache, `confirm_recipient_change`,
  `note_recipients_reviewed`.
- **P4 · M8a2:** `pull_locked`, `gitrepo::compare`, the fake `RemoteSyncer`.
- **P3 · M6:** `Session`, `vault_without_unlocking`, and
  `sync_resolving`'s lazy unlock (`session.rs`), the model for a session's
  late unlock.
- **P6 · M12b/M12d:** the orphan cleanup (keyed by pubkey, so `approve
  --device`'s relabel cannot make it delete a live key), and the script
  wrappers.

## Design references

Init TDD sections:

- "The flows", the approving device
- "Approval always re-encrypts, and there is no `--reencrypt` flag"
- "The approver unlocks, and the unlock comes late"
- "Approval fetches before it commits"
- `D-ENROLL-COLLISIONS`
- `D-ENROLL-PROMPTER`
- "The request names the vault it is for"
- "Expiry, revocation, and pruning"
- "Interaction with the local trust cache"
- "Library surface"

## Go sources

- Go plan `gage-cli-init-design/e4-approving-side.md`: all of it,
  especially "Settled at the start of implementation" (the codes as their
  own parameter and the genuine second open, the clear-only commit, the
  collision pre-check as a library pass, and the retry loop ending on
  empty input).
- Parity ledger: §2 `f800af6`, `187c810`, `faf49b9` (#52) and `df3c0fe`;
  §3 E4; §5 `recipient_pending`, `recipient_approve`, `recipient_deny`;
  §6 `enrollment.md` and the enrollment rows of the other pages.
- Go sources at the pinned SHA: `internal/gage/enrollapprove.go`,
  `cmd/gage/approve.go`, `cmd/gage/recipient.go`, and Go's
  `doc/cli/enrollment.md`.
- Help fixtures: `recipient_pending.txt`, `recipient_approve.txt`,
  `recipient_deny.txt`, `recipient.txt`.
- Test inventory, 71 rows:
  - `internal/gage/enrollapprove_test.go` (14)
  - `enrollapproveatomic_test.go` (9)
  - `enrollapprovefetch_test.go` (8)
  - `enrolldeny_test.go` (6)
  - `enrollprune_test.go` `TestRecipientAddPrunesExpiredRequests`,
    `TestRecipientRemovePrunesExpiredRequests` (2)
  - `cmd/gage/approve_cli_test.go` (19)
  - `approve_scope_cli_test.go` (13)
- Inbound obligations:
  - enrollment-id completion (P3 · M13)
  - the `recipient` parent-help child list (P5 · M9a)
  - the docs left by P3, P4 and P5 (see the [index](index.md#inventory-accounting))

## Decisions to make first

[Q-PENDING-TZ](../open-questions.md#q-pending-tz),
[Q-ENROLL-AMBIGUOUS](../open-questions.md#q-enroll-ambiguous) and
[Q-ENROLLMENT-CODE-HISTORY](../open-questions.md#q-enrollment-code-history)
are settled. What is left before code:

- **The library surface.**
  - `approve_enrollments(&self, approvals: &[Approval], codes: &[…], identity: &Identity, prompter) -> Result<ApprovalResult, EnrollmentError>`,
    with `Approval { request, label }` and a per-request outcome
    (`added: bool`).
  - `deny_enrollment(&self, id, prompter)`. It takes a `Prompter` but no
    `Identity`; the TDD records why.
  - `check_enrollment_labels(&self, approvals)`, the collision pre-check,
    which needs no identity and no network.
  - Go's `context.Context` is dropped ([Q-P7-SPLIT](../open-questions.md#q-p7-split)).
- **The session's late unlock.** In one-shot mode, `gage-cli` holds a
  vault without unlocking (`with_resolved_vault`) and unlocks after the
  confirmation. In a session, an already-unlocked vault's cached identity
  is reused with no prompt, and a never-unlocked one is unlocked lazily
  and kept. Model it on `sync_resolving`'s `OnceCell` unlock; don't add a
  second way to put an identity on `HeldVault`.
- **Where pruning hooks in.** Through E1b's `commit_recipient_list`
  options: the files pruning removes join `delete_paths`, so `add`,
  `remove` and `approve` prune in their own commit. `deny` calls pruning
  directly in its own write.
- **The failed-push clause** approval passes to E1b. Per the TDD's
  "Approval's failed push needs the same treatment", it says the approval
  is local-only and names the push. `deny` has its own.
- **The retry loop.** Only on the prompted path. It loops while the
  answer is non-empty and the refusal is `CodeWrong`, with a runaway
  guard mirroring the unlock's attempt cap. A bare Enter aborts with
  nothing done. `--code` never retries.
- **A declined approve** is `Conflict`, with a message saying nothing was
  written, following `SelfRemovalDeclined`. It is not a silent exit 0.
- **Rendering `pending`'s expiry** through `Io`'s time zone
  ([Q-PENDING-TZ](../open-questions.md#q-pending-tz)). `main` fills it
  from the system zone, and tests set two zones.

## Tasks

Write each task's tests first, at task granularity. Library rows go in
`crates/gage/src/enrollment/approve_tests.rs` (and siblings); CLI rows in
`crates/gage-cli/src/approve_cli_tests.rs`.

### 1. `approve_enrollments` — 14 rows

- [ ] The library method over E1b's body, with the re-open under the lock,
      the already-a-recipient filter and the clear-only commit.
      `check_enrollment_labels`.
- [ ] Rows (`enrollapprove_test.go`):
  - `TestApprovalAddsOneRecipientAndKeepsTheFilesInSync`: `approval_adds_one_recipient_and_keeps_the_files_in_sync`. `verify` passes after.
  - `TestApprovalMakesEveryPreExistingEntryReadable`: `approval_makes_every_pre_existing_entry_readable`
  - `TestApprovalLandsInExactlyOneCommit`: `approval_lands_in_exactly_one_commit`
  - `TestBatchApprovalIsOnePassAndOneCommit`: `batch_approval_is_one_pass_and_one_commit`
  - `TestApprovalRefusesAnApproverWhoCannotReadEveryEntry`: `approval_refuses_an_approver_who_cannot_read_every_entry`. `CannotGrantFullAccess`, naming the count; nothing committed; M10's prompt never shown.
  - `TestApprovalRelabelsWithoutChangingTheKey`: `approval_relabels_without_changing_the_key`
  - `TestApprovingAKeyThatIsAlreadyARecipientIsANoOp`: `approving_a_key_that_is_already_a_recipient_is_a_no_op`. The request is cleared; zero entries re-encrypted; `added: false`.
  - `TestApprovingTheSameDeviceTwiceLeavesOneRecipient`: `approving_the_same_device_twice_leaves_one_recipient`
  - `TestAMalformedRequestProducesNothingToRender`: `a_malformed_request_produces_nothing_to_render`
  - `TestClockSkewNamesTheRequestingDevicesClock`: `clock_skew_names_the_requesting_devices_clock`
  - `TestCheckEnrollmentLabelsAcceptsARelabel`: `check_enrollment_labels_accepts_a_relabel`
  - `TestCheckEnrollmentLabelsIgnoresTheDevicesOwnEntry`: `check_enrollment_labels_ignores_the_devices_own_entry`
  - `TestCheckEnrollmentLabelsNeedsNoIdentityOrNetwork`: `check_enrollment_labels_needs_no_identity_or_network`
  - `TestCheckEnrollmentLabelsRefusesANameAnotherRecipientTook`: `check_enrollment_labels_refuses_a_name_another_recipient_took`

### 2. Atomicity and the trust cache — 9 rows ⚑

- [ ] Rows (`enrollapproveatomic_test.go`):
  - `TestApprovalWritesWhatItVerifiedUnderTheLockNotWhatWasDisplayed` ⚑: `approval_writes_what_it_verified_under_the_lock_not_what_was_displayed`. The seal is swapped between the open and the commit.
  - `TestInterruptedApprovalLeavesHEADUntouchedAndRequestsPending` ⚑: `interrupted_approval_leaves_head_untouched_and_requests_pending`
  - `TestTheAuthoritativeDuplicateCheckStillFiresUnderTheLock` ⚑: `the_authoritative_duplicate_check_still_fires_under_the_lock`. `RecipientExists`, not `NameTaken`.
  - `TestDecliningTheTrustPromptAbortsWithNothingCommitted`: `declining_the_trust_prompt_aborts_with_nothing_committed`
  - `TestApprovalRefusesAnEmptyBatch`: `approval_refuses_an_empty_batch`
  - `TestApprovalPrunesExpiredRequestsInTheSameCommit`: `approval_prunes_expired_requests_in_the_same_commit`. Covers both an expired filename epoch and one beyond the ceiling.
  - `TestASecondDeviceGetsTheOrdinaryWarningAfterSomeoneElseApproves`: `a_second_device_gets_the_ordinary_warning_after_someone_else_approves`
  - `TestTheApprovingDeviceDoesNotWarnItselfAboutItsOwnApproval`: `the_approving_device_does_not_warn_itself_about_its_own_approval`
  - `TestAPendingRequestAloneTriggersNoWarning`: `a_pending_request_alone_triggers_no_warning`

### 3. Approval fetches first — 8 rows ⚑

- [ ] The lock, then `pull_locked` with the divergence check, then the re-open, then E1b.
- [ ] Rows (`enrollapprovefetch_test.go`):
  - `TestApprovalFastForwardsBeforeItWrites`: `approval_fast_forwards_before_it_writes`
  - `TestApprovalRefusesADivergedRemoteBeforeWritingAnything`: `approval_refuses_a_diverged_remote_before_writing_anything`. The message says `gage sync`, the deliberate difference from E3.
  - `TestApprovalFindsDivergenceInTheReportRatherThanAnError` ⚑: `approval_finds_divergence_in_the_report_rather_than_an_error`
  - `TestApprovalOverAnUnreachableRemoteWarnsAndProceeds`: `approval_over_an_unreachable_remote_warns_and_proceeds`
  - `TestUncontendedApprovalNeverReportsContention` ⚑: `uncontended_approval_never_reports_contention`
  - `TestApprovalReportsARequestResolvedByAnotherDevice`: `approval_reports_a_request_resolved_by_another_device`. `NoSuchRequest` (`NotFound`), naming the likely cause.
  - `TestApprovalWarnsWhenItsPushFails`: `approval_warns_when_its_push_fails`. Approval's clause, not the generic one.
  - `TestReapprovalAfterAFailedPushIsTheNoOp`: `reapproval_after_a_failed_push_is_the_no_op`

### 4. Deny and pruning — 8 rows

- [ ] `deny_enrollment`; pruning wired into `add_recipient`/`remove_recipient` through E1b's options.
- [ ] Rows (`enrolldeny_test.go`, `enrollprune_test.go`):
  - `TestDenyRemovesTheRequestAndGrantsNothing`: `deny_removes_the_request_and_grants_nothing`
  - `TestDenyOnAnAmbiguousIDDeletesNothing`: `deny_on_an_ambiguous_id_deletes_nothing`
  - `TestDenyOnAnUnmatchedIDDeletesNothing`: `deny_on_an_unmatched_id_deletes_nothing`
  - `TestDenyResolvesAnEightCharacterPrefix`: `deny_resolves_an_eight_character_prefix`
  - `TestDenyPrunesExpiredRequestsInTheSameCommit`: `deny_prunes_expired_requests_in_the_same_commit`
  - `TestDenyWarnsWhenItsPushFails`: `deny_warns_when_its_push_fails`
  - `TestRecipientAddPrunesExpiredRequests`: `recipient_add_prunes_expired_requests`
  - `TestRecipientRemovePrunesExpiredRequests`: `recipient_remove_prunes_expired_requests`

### 5. The commands — 19 rows ⚑

- [ ] `recipient pending`, `approve` and `deny`: registry entries (group
      Recipients, `Availability::Both`), `commands/mod.rs`, the flags
      (`--code` repeatable with `ArgAction::Append`). The CLI's ordering
      of open, check, render, confirm and unlock, and the retry loop.
- [ ] Reword `recipient list`'s description if it doesn't already match
      its fixture (D-ENROLL-VERBS).
- [ ] Rows (`approve_cli_test.go`):
  - `TestApproveEndToEndLetsTheDeviceRead`: `approve_end_to_end_lets_the_device_read`
  - `TestApproveCostsNoUnlockOnAWrongCode` ⚑: `approve_costs_no_unlock_on_a_wrong_code`. Prompt count zero; the same holds for an expired request.
  - `TestDecliningTheApprovalCostsNoUnlock` ⚑: `declining_the_approval_costs_no_unlock`. Still pending; `Conflict`.
  - `TestApproveRefusesAWrongCodeWithoutRetryingWhenItCameFromAFlag`: `approve_refuses_a_wrong_code_without_retrying_when_it_came_from_a_flag`
  - `TestApproveReadsTheCodeThroughThePrompterWhenNoFlagIsGiven`: `approve_reads_the_code_through_the_prompter_when_no_flag_is_given`
  - `TestApproveWithAnEmptyPromptedCodeApprovesNothing`: `approve_with_an_empty_prompted_code_approves_nothing`
  - `TestApproveDeduplicatesARequestOpenedByTwoCodes`: `approve_deduplicates_a_request_opened_by_two_codes`
  - `TestApproveNarrowsToAPositionalID`: `approve_narrows_to_a_positional_id`
  - `TestApproveWithADeviceFlagRelabels`: `approve_with_a_device_flag_relabels`
  - `TestApproveRejectsADeviceFlagAgainstSeveralRequests`: `approve_rejects_a_device_flag_against_several_requests`. `Usage`, naming the ID form.
  - `TestApproveRejectsAnInvalidDeviceNameBeforeAnythingElse`: `approve_rejects_an_invalid_device_name_before_anything_else`
  - `TestApproveSurfacesAWrongVaultRequestWithoutRetrying` ⚑: `approve_surfaces_a_wrong_vault_request_without_retrying`. Names the vault; asserts the prompt count.
  - `TestApproveUnderScriptStaysARealQuestion`: `approve_under_script_stays_a_real_question`. `--yes` answers only the trust question.
  - `TestDenyRemovesARequestAndNeedsNothing`: `deny_removes_a_request_and_needs_nothing`
  - `TestDenyOnAnAmbiguousIDRendersTheCandidates`: `deny_on_an_ambiguous_id_renders_the_candidates`. `Ambiguous`.
  - `TestDenyOnAnUnmatchedIDIsNotFound`: `deny_on_an_unmatched_id_is_not_found`
  - `TestPendingAndDenyWorkUnderScript`: `pending_and_deny_work_under_script`
  - `TestRecipientPendingNeedsNoUnlockAndShowsNoDeviceNames`: `recipient_pending_needs_no_unlock_and_shows_no_device_names`
  - `TestRecipientPendingWithNothingPendingExitsZero`: `recipient_pending_with_nothing_pending_exits_zero`

### 6. Scope, sessions and the two questions — 13 rows ⚑

- [ ] Rows (`approve_scope_cli_test.go`):
  - `TestApprovalPromptsOnceForTheApproversPassphrase`: `approval_prompts_once_for_the_approvers_passphrase`
  - `TestTheFullAccessRefusalCostsOneUnlockNotZero` ⚑: `the_full_access_refusal_costs_one_unlock_not_zero`
  - `TestApproveInASessionReusesTheCachedIdentity`: `approve_in_a_session_reuses_the_cached_identity`
  - `TestTheApproverAnswersTwoDistinctQuestions`: `the_approver_answers_two_distinct_questions`. Declining the second aborts with nothing committed.
  - `TestApproveRefusesATakenNameBeforeTheConfirmationAndBeforeAnyUnlock` ⚑: `approve_refuses_a_taken_name_before_the_confirmation_and_before_any_unlock`
  - `TestApproveSurfacesARequestExpiredByItsSeal`: `approve_surfaces_a_request_expired_by_its_seal`
  - `TestApproveOnAnAmbiguousIDRendersTheCandidates`: `approve_on_an_ambiguous_id_renders_the_candidates`
  - `TestApproveWithNoIDAgainstAStuffedPendingRefuses`: `approve_with_no_id_against_a_stuffed_pending_refuses`. `TooManyPending`; the message names `approve <ID>`; nothing prompted.
  - `TestNamingAnIDIsTheWayOutOfAStuffedPending`: `naming_an_id_is_the_way_out_of_a_stuffed_pending`. `deny` and `pending` are unaffected.
  - `TestTheBoundFollowsTheScopeNotTheFlag` ⚑: `the_bound_follows_the_scope_not_the_flag`. The two runs differ only in the scope they pass.
  - `TestApproveUnderARealScriptRunFailsCleanly`: `approve_under_a_real_script_run_fails_cleanly`
  - `TestPendingAndDenyUnderARealScriptRun`: `pending_and_deny_under_a_real_script_run`
  - `TestPendingRendersExpiryInTheLocalTimezone`: `pending_renders_expiry_in_the_local_timezone`. Two zones through `Io`, the same instant.

(Rows: 14 + 9 + 8 + 8 + 19 + 13 = 71.)

- [ ] Beyond Go's rows: a malformed payload carrying ANSI escapes never
      reaches the approve screen
      (`approve_never_renders_an_unvalidated_device_name`). This is Go
      E4's bullet; it has no row of its own in Go's inventory, and it is
      the point of E2's validation.

### 7. Completion

- [ ] Request ids as completion candidates for `recipient approve` and
      `deny` (M13's obligation, ledger §6 `addressing-entries.md`). Check
      Go's `complete.go` for what it offers, and port that.

### 8. Docs

Docs describe shipped behavior, so they go last, after every P7 surface is
green.

- [ ] `doc/cli/enrollment.md`, new, checked against Go's page of the same
      name. Add it to `doc/cli/README.md`'s index.
- [ ] `doc/cli/identities-and-recipients.md`: replace the "Not yet in
      gage-rs" note with the enrollment path beside the manual one, and
      document the new commands.
- [ ] `doc/cli/sharing.md`, `sync.md`, `git-and-auth.md`: enrollment's
      exceptions (enroll fails on an unreachable remote; approval fetches
      first and warns on push failure; `pending/` is a union on merge).
- [ ] `doc/cli/session-mode.md`: enroll against a never-unlocked vault, and
      approve's late unlock in a session.
- [ ] `doc/cli/addressing-entries.md`: request ids (exact, substring,
      candidates, over the UUID portion) and their completion.
- [ ] `doc/cli/exit-codes.md`: the enrollment errors' codes.
- [ ] Each page is checked against Go's `doc/cli/` page for the same
      surface.

### 9. Opus review

- [ ] Opus review of the ⚑ rows, by mutation.

| Mutation | Killed by |
|---|---|
| Write from the displayed `OpenedRequest` instead of the re-open | |
| Unlock before the confirmation | |
| Full-access pre-flight before the unlock (impossible) or after the lock | |
| `approve` calls `pull()` (self-locking) | |
| Divergence check removed | |
| The retry loop also retries `WrongVault`/`MalformedRequest` | |
| Render before the open validates the payload | |
| A "skip the bound" path for an ID-scoped run | |
| The name pre-check moved after the confirmation | |
| Approval's push clause not threaded through | |

### 10. Parity and ledger

- [ ] Fixture parity for `recipient pending`, `approve` and `deny` (name,
      group, usage `[ID...]`/`<ID>`, flags) and the `recipient` child
      list, in a `p7_recipient_enrollment_commands_have_their_go_fixture_flags_and_descriptions`
      test.
- [ ] Parity ledger:
  - §2 `f800af6`/`187c810`/`faf49b9`, `df3c0fe`
  - §3 E4 and the init-design row
  - §5's four enrollment commands
  - §6 `enrollment.md` and every page row naming P7

  All marked `[x]` after CI.
- [ ] Mark E4 and P7 `[x]` in the [P7 index](index.md) and the
      [port plan](../index.md) after CI.

## Definition of done

Every box above is checked. All 71 claimed rows are ported, the review
table is filled in, and the docs are written. E1b's, E2's and E3's suites
are still green. `make lint` and `make test` are clean locally and on
Linux, macOS and Windows CI.

The milestone is done when:
- a second device can be enrolled and approved end to end, and then reads
  entries written before it existed
- a wrong code, an expired request, a request for another vault, a
  malformed request, or a declined prompt costs the approver no passphrase
- nothing is ever written that wasn't verified under the lock
