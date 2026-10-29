# E3 — Joining a vault

[← E2](e2-sealed-request.md) · [P7 plan](index.md) · [#66](https://github.com/distillerylabs/gage-rs/issues/66) · next: [E4 — Approving a device](e4-approving-side.md)

> **Recommended model: Sonnet\* ⚑.** It wires settled primitives to a
> command, plus a prompt on `clone`. Go rated it Sonnet. It moves up to
> borderline here because three of its rows fail for reasons that look
> like unrelated remote or concurrency bugs:
> - the lock held across the pull, through the lock-held sync functions
> - the dirty-tree reset before the pull
> - a divergence that arrives as a value, not an error
>
> Run an **Opus review of those rows and of "no wrapper answers yes"**
> (task 9) before calling it done.

## Goal

`gage identity enroll` creates or reuses this device's identity, seals a
request to a freshly generated code, publishes it, and prints the code.
`clone` offers it with no flag. After this milestone a device can *ask* to
join a vault. Nothing can approve it yet (E4), so the end state is an
inert request in `pending/` and a code on screen.

- **`Vault::enroll(device, ttl, prompter) -> Result<EnrollmentRequest, EnrollmentError>`**,
  ordered per `D-ENROLL-REMOTE`:
  1. Check the remote: none is `NoRemote` (`Usage`); unreachable is
     `RemoteUnreachable` (`Unreachable`). Either fails before any
     identity is written or any prompt is shown.
  2. Take the lock and run the **dirty-tree reset**: `with_vault_write`,
     not `with_write_lock`.
  3. Fetch and fast-forward through **`pull_locked`**. A divergence is
     `Diverged` (`Conflict`), with advice a device that can't decrypt can
     act on, and never "run `gage sync`".
  4. The **already-a-recipient check by key**: this device's recorded
     `pubkey` is in the recipient list. That is a success that does no
     work (`published: false`, nothing prompted, generated, sealed,
     committed or pushed). It runs before the **name check**, which is
     `DeviceNameTaken` (`Conflict`), naming `--device`.
  5. Create or reuse the identity through `identityfile::create`. The
     create path asks twice (`Purpose::Create`); the reuse path unlocks
     once (`Purpose::Unlock`).
  6. Seal (E2), write, commit, and push through **`push_locked`**.
  7. Release the lock.

  The clock check warns, and proceeds, when local time is behind HEAD's
  committer time.
- **`gage identity enroll [--device NAME] [--ttl DURATION] [-u NAME]`**:
  group Identity, both modes. The handler records `device`, `method` and
  `pubkey` in global config, as `identity add` does. Both secrets carry
  their labels: the passphrase "stays on this device", the code is "safe
  to send".
- **`Prompter::confirm_default_yes`**: rendered `[Y/n]`, where a bare
  Enter accepts. It has no default body. Every wrapper delegates, and none
  answers yes on its own.
- **`clone`'s offer:** interactive, and this device holds no identity for
  the vault, means `confirm_default_yes`, and yes runs the same enroll
  under the device name `clone` resolved. Otherwise it prints the reworded
  access message, which names `identity enroll` and keeps the manual path.

## Depends on

- **E2:** the code, the seal, the filename, `pending_enrollments`,
  `open_enrollment` (the rows open what enroll published).
- **P4 · M8a2:** `pull_locked`/`push_locked`, `gitrepo::compare`,
  `RemoteSyncer` and `Vault::with_remote_syncer`, `SyncVault` and the
  fakes (`remotesync_tests.rs`), and `clone`.
- **P2 · M2:** `identityfile::create`, `Purpose`, `UnlockRequest`.
- **P1:** `VaultEntry.pubkey`, `devicename`, `resolve_device_name`,
  `with_vault_write` and the dirty-tree reset.
- **P6 · M12d:** the script-mode wrappers (`scriptprompt.rs`) and `for_script`.

## Design references

Init TDD sections:

- "The flows", the joining device
- "Enrolling with an identity you already have"
- "Which secret is which"
- `D-ENROLL-REMOTE`, including "Which means enroll cannot reach the network
  through the existing public sync entry points"
- `D-ENROLL-COLLISIONS`
- `D-ENROLL-PROMPTER`, and "Clone's offer needs a default-yes confirm"
- "Why there is no `--enroll` flag"
- "Clock skew, in the direction the ceiling doesn't cover"

## Go sources

- Go plan `gage-cli-init-design/e3-joining-side.md`: all of it.
- Parity ledger: §2 `c86ad4c`; §3 E3; §4 `clone`'s enrollment offer;
  §5 `identity_enroll`.
- Go sources at the pinned SHA: `internal/gage/enroll.go`,
  `cmd/gage/enroll.go`, `cmd/gage/clone.go` (`offerEnrollment`,
  `accessLines`), `cmd/gage/prompter.go` (`ConfirmDefaultYes`), and
  `cmd/gage/script.go` and `assumeyes.go` (the wrappers' delegation).
- Help fixtures: `identity_enroll.txt`, `identity.txt`, `clone.txt`.
- Test inventory, 60 rows:
  - `internal/gage/enroll_test.go` (10)
  - `enrollcollide_test.go` (5)
  - `enrollremote_test.go` (10)
  - `enrollcrash_test.go` (4)
  - `enrollmerge_test.go` (4)
  - `cmd/gage/enroll_cli_test.go` (18)
  - `clone_enroll_test.go` (8)
  - `enroll_prompter_test.go` (1)
- Inbound, 2 rows:
  - P6: the `HeadCommitterTime` half of `gitrepo/history`
    `TestLogAndHeadCommitterTimeOnARepoWithNoCommitsYet`.
  - P2: `prompter` `TestConfirmDefaultYesDefaultsToYes`.
- Inbound obligations:
  - `use`'s no-identity hint (P3 · M6)
  - `enroll` in the identity children's completion (P3 · M13)
  - the identity parent-help child list (P5 · M9a)
  - clone's access message (P4 · M8a2)

## Decisions to make first

[Q-CONFIRM-DEFAULT-YES](../open-questions.md#q-confirm-default-yes)
settles the prompter. What is left before code:

- **Reaching the remote from inside the lock.** Read `pull_locked`,
  `push_locked` and `push_after_write` (`remotesync.rs`) first.
  - The pull side is `pull_locked`, then `gitrepo::compare` checked for
    `RemoteState::Diverged`.
  - An unreachable fetch must become `RemoteUnreachable`, not the
    warn-and-proceed `sync_on_unlock` gives an unlock.
  - On the push side, `push_locked` merges a rejected push. Decide
    whether enroll can use it as is (a merge of an inert `pending/` file
    is safe, the TDD's union argument), or needs its own push. Either
    way, a push rejected for write access must surface as an error that
    names `gage auth login` (`syncerr` `Auth`), not as `push_after_write`'s
    warning.
  - Never `pull()`, `push()` or `try_pull`: they take the lock, which is
    not re-entrant, so enroll would block on its own lock and fail as
    contended.
- **The reuse path's prompt.** `identityfile::create` is get-or-create
  and warns when it reuses. Confirm its reuse branch asks once with
  `Purpose::Unlock` and fails with `WrongPassphrase` (`LockedOrAuth`) on a
  wrong one. If it doesn't, the fix is there, not a second create path in
  `enroll`. "`identity enroll` is `identity add` plus publishing", and
  must not be forked.
- **The already-a-recipient check reads global config from the library.**
  `pubkey` is empty after `clone`, and that means the key check is
  skipped. The global-config *write* stays in `gage-cli`, as `identity
  add`'s does (`commands/identity.rs`); `EnrollmentRequest.pubkey` is
  returned so the handler has it.
- **`gitrepo::head_committer_time(dir) -> Result<Option<Timestamp>, Error>`.**
  `None` on an empty repo. That is the inbound row's half.
- **The clone offer's plumbing.** `clone::run` takes no prompter
  (`commands/mod.rs`). Pass `keys.prompter()` as `init` does. The offer
  is shown where a `Purpose::Create` request can be answered (a terminal
  prompter, or `--script` at a terminal), not by a TTY test of its own,
  so `--stdin` gets no offer.
- **`--ttl` parsing.** Reuse `shell.rs`'s `parse_duration` (Go duration
  syntax, as the fixture's `24h0m0s` default implies). The help shows
  Go's defaults. `TestHumanDurationReadsLikeAPersonSaidIt` is the
  printed-expiry rendering; decide where it lives.
- **The session route.** `identity enroll -u NAME` in a session goes
  through `with_resolved_vault` / `Session::vault_without_unlocking`
  (the sync verbs' route). The vault can't be `use`d, because this device
  holds no key it accepts.

### Resolutions (proposed, awaiting review)

Grounded in a read of `remotesync.rs`, `identityfile.rs`, `entry/crud.rs`
and `enrollment/{mod,seal}.rs`. No code is written until these are
accepted.

1. **Remote route.** `enroll` runs inside `with_vault_write` (lock, then
   dirty-tree reset). Step 1 is the cheap `has_remote` check
   (`NoRemote`, `Usage`) before the lock. "Unreachable" is not a separate
   pre-fetch: step 3's fetch is the first thing that touches the network,
   and it precedes the identity (step 5) and every prompt, so the
   `Unreachable` row holds without a second round trip. Only
   `pull_locked` changes: from private to `pub(crate)`, so `enroll` can
   call it. Its fetch error is mapped to `RemoteUnreachable`
   (`Unreachable`) or the auth error as-is, never the warn-and-proceed of
   `sync_on_unlock`. Divergence is read from the returned
   `SyncReport.diverged`, not from an `Err`, which is what
   `divergence_is_found_by_the_report_rather_than_an_error` pins. Never
   `pull()`, `push()` or `try_pull` (they re-take the lock).
2. **Push.** Use `push_locked` as is. A rejected push is fetched and
   merged, which is safe here: the only new path is an inert
   `pending/<id>-<epoch>.age` with a fresh UUID, so the two sides cannot
   touch the same path (the union argument). A `Kind::Auth` failure
   becomes an `EnrollmentError` whose message names `gage auth login`;
   `push_after_write`'s warning is not used. The identity and the local
   commit are kept on any push failure, and re-running enroll reuses the
   identity and seals a fresh request.
3. **Reuse path.** Confirmed in `identityfile::create`: an existing file
   goes through `open`, which asks with `Purpose::Unlock` (retrying up to
   `MAX_UNLOCK_ATTEMPTS`) and ends in `WrongPassphrase` (`LockedOrAuth`).
   No change to `create`, and no second create path in `enroll`.
   "Prompts once" means once when the passphrase is right.
4. **Already-a-recipient check reads global config in the library**, via
   `config::read(&paths::global_config_file(dirs))` keyed by the vault's
   local name, the way `Vault`'s existing registration read does
   (`vault.rs:579`). An unreadable or missing registration, or an empty
   `pubkey`, skips the key check; it is never an error. The global-config
   *write* stays in `gage-cli`.
5. **`gitrepo::head_committer_time`** goes in `gitrepo/history.rs`: HEAD's
   committer time as `jiff::Timestamp`, `None` on an unborn branch.
6. **`seal_enrollment`** loses its `expect(dead_code)` now that `enroll`
   calls it.
7. **`--ttl`** reuses `shell::parse_duration`, made `pub(crate)`. The
   printed-expiry rendering (`human_duration`) lives in the new
   `commands/enroll.rs` beside the handler, since only enroll prints it.
8. **Session route.** `identity enroll -u NAME` goes through
   `with_resolved_vault` / `Session::vault_without_unlocking`, the sync
   verbs' route.
9. **Still to read, not decided:** how `clone` should tell "a
   `Purpose::Create` can be answered here" without a TTY test. I will read
   Go's `cmd/gage/clone.go` (`offerEnrollment`) and `script.go` at the
   pinned SHA before task 8 and record the answer here, as a refinement
   of this list, before writing that task's code.

Accepting 1–8 needs a `Q-ENROLL-REMOTE-ROUTE` entry in the open-questions
register; I will add it with the first commit-ready change, not before
you've signed off.

## Tasks

Write each task's tests first, at task granularity. Library rows go in
`crates/gage/src/enrollment/enroll_tests.rs` (and siblings); CLI rows in
`crates/gage-cli/src/enroll_cli_tests.rs` and `clone_enroll_tests.rs`.

### 1. `confirm_default_yes` — 1 row + 1 inbound ⚑

- [x] Add it to `Prompter` with no default body. Implement it on the
      terminal prompter (`[Y/n]`, Enter accepts), `FakePrompter`,
      `delegate_the_rest!`, `NoChoosePrompter` and `AssumeYes`, each
      delegating.
- [x] Rows:
  - Inbound `TestConfirmDefaultYesDefaultsToYes`: `confirm_default_yes_defaults_to_yes`. The terminal prompter on a bare Enter, and the `[Y/n]` rendering.
  - `TestNoNonInteractivePrompterAnswersConfirmDefaultYes` ⚑: `no_non_interactive_prompter_answers_confirm_default_yes`. Covers `AssumeYes`, `EnvPrompter`, `RefusingPrompter`, `NoCreatePrompter` and `NoChoosePrompter`, each over a base that declines: none returns yes.
- [x] Extend M12d's `every_wrapper_passes_through_each_method_it_does_not_answer` with the new method.

### 2. `head_committer_time` — 1 inbound

- [x] `gitrepo::head_committer_time`.
- [x] Inbound: the `HeadCommitterTime` half, `head_committer_time_on_a_repo_with_no_commits_yet` in `gitrepo/history_tests.rs`. Drop that file's "P7's" note.

### 3. `Vault::enroll`, the identity paths — 10 rows

- [x] `Vault::enroll` steps 2 and 4–7, against a reachable bare remote.
- [x] Rows (`enroll_test.go`):
  - `TestEnrollCreatesAnIdentityOnADeviceThatHoldsNone`: `enroll_creates_an_identity_on_a_device_that_holds_none`. `Purpose::Create`, asked twice.
  - `TestEnrollReusesAnExistingIdentityAndPromptsOnce`: `enroll_reuses_an_existing_identity_and_prompts_once`. Asserts the purpose and the prompt count, and that no second file is written.
  - `TestEnrollWithAWrongPassphraseOnTheReusePathPublishesNothing`: `enroll_with_a_wrong_passphrase_on_the_reuse_path_publishes_nothing`
  - `TestEnrollSealsThePublicKeyThisDeviceActuallyHolds`: `enroll_seals_the_public_key_this_device_actually_holds`
  - `TestEnrollHonorsTheTTL`: `enroll_honors_the_ttl`. The sealed expiry and the filename epoch both.
  - `TestEnrollRefusesAnUnusableTTLBeforeAnythingHappens`: `enroll_refuses_an_unusable_ttl_before_anything_happens`
  - `TestEnrollRefusesADeviceNameOutsideTheAllowlist`: `enroll_refuses_a_device_name_outside_the_allowlist`
  - `TestEnrolledRequestCarriesThisVaultsID`: `enrolled_request_carries_this_vaults_id`
  - `TestEnrollWarnsWhenThisMachinesClockIsBehindHead`: `enroll_warns_when_this_machines_clock_is_behind_head`. A remote commit with a future committer time.
  - `TestEnrollIsSilentWhenTheClockLooksFine`: `enroll_is_silent_when_the_clock_looks_fine`

### 4. Collisions — 5 rows

- [x] The already-a-recipient check by key, then the name check.
- [x] Rows (`enrollcollide_test.go`):
  - `TestEnrollOnADeviceThatIsAlreadyARecipientIsASuccessThatDoesNoWork`: `enroll_on_a_device_that_is_already_a_recipient_is_a_success_that_does_no_work`. Built the way it happens: `identity add`, then `recipient add` of that key from another device, then enroll. Prompt count zero; not `DeviceNameTaken`.
  - `TestTheAlreadyARecipientCheckIsByKeyNotByName`: `the_already_a_recipient_check_is_by_key_not_by_name`. The key is listed under a different label.
  - `TestWithNoRecordedPubkeyTheKeyCheckIsSkipped`: `with_no_recorded_pubkey_the_key_check_is_skipped`
  - `TestEnrollRefusesANameThatLabelsADifferentRecipient`: `enroll_refuses_a_name_that_labels_a_different_recipient`. No identity file written; the message names `--device`; `Conflict`.
  - `TestEnrollUnderADifferentDeviceNameSucceedsWhereTheDefaultCollided`: `enroll_under_a_different_device_name_succeeds_where_the_default_collided`

### 5. The remote — 10 rows ⚑

- [x] Step 1, step 3, and the push's error surface.
- [x] Rows (`enrollremote_test.go`; injected `RemoteSyncer` for unreachable, never a real timeout):
  - `TestEnrollFetchesBeforeItCommits`: `enroll_fetches_before_it_commits`. The push is a fast-forward over a remote that moved ahead.
  - `TestEnrollWithAnUnreachableRemoteFailsBeforeAnythingIsCreated` ⚑: `enroll_with_an_unreachable_remote_fails_before_anything_is_created`. Prompt count zero, nothing under identities, nothing committed, `Unreachable`.
  - `TestEnrollWithNoRemoteFailsWithItsOwnError`: `enroll_with_no_remote_fails_with_its_own_error`. `NoRemote`, `Usage`.
  - `TestEnrollHoldsTheWriteLockAcrossThePull` ⚑: `enroll_holds_the_write_lock_across_the_pull`. A **child process** can't move HEAD between the catch-up and the commit, in the style of M4's lock tests.
  - `TestUncontendedEnrollNeverReportsContention` ⚑: `uncontended_enroll_never_reports_contention`. Completes well inside the write-lock timeout.
  - `TestEnrollPushRejectedForWriteAccessKeepsWhatItMade`: `enroll_push_rejected_for_write_access_keeps_what_it_made`. The identity and the local commit stay; the error names `gage auth login`.
  - `TestEnrollAfterAFailedPushReusesTheIdentityAndSucceeds`: `enroll_after_a_failed_push_reuses_the_identity_and_succeeds`. A fresh request and code.
  - `TestEnrollRefusesADivergedPull` ⚑: `enroll_refuses_a_diverged_pull`. Built as it happens: a failed push, the remote moves, enroll again. Nothing written; the message doesn't say "gage sync".
  - `TestDivergenceIsFoundByTheReportRatherThanAnError` ⚑: `divergence_is_found_by_the_report_rather_than_an_error`. Asserts the *refusal*, not "some error".
  - `TestEnrollTakesTheDirtyTreeResetBeforeItPulls` ⚑: `enroll_takes_the_dirty_tree_reset_before_it_pulls`. An unrelated dirty file gives the ordinary discard warning, not a dirty-tree `Conflict`.

### 6. Crash-safety and two devices — 8 rows

- [x] Rows (`enrollcrash_test.go`, `enrollmerge_test.go`):
  - `TestAStrayPendingFileIsInert`: `a_stray_pending_file_is_inert`
  - `TestTheNextWriteDiscardsAStrayPendingFile`: `the_next_write_discards_a_stray_pending_file`. The warning names it, and it is absent from the commit.
  - `TestPendingEnrollmentsListsAStrayLocallyAndNowhereElse`: `pending_enrollments_lists_a_stray_locally_and_nowhere_else`
  - `TestAStrayDoesNotWedgeTheJoiningDevice`: `a_stray_does_not_wedge_the_joining_device`
  - `TestTwoDevicesEnrollAgainstTheSameRemoteAndBothRequestsSurvive`: `two_devices_enroll_against_the_same_remote_and_both_requests_survive`
  - `TestConcurrentEnrollsMergeIntoAUnion`: `concurrent_enrolls_merge_into_a_union`
  - `TestMergedRequestsStillGrantNothing`: `merged_requests_still_grant_nothing`
  - `TestADivergenceInvolvingPendingIsReportedHonestly`: `a_divergence_involving_pending_is_reported_honestly`. Port Go's finding about which `pending/` divergences are reachable. If one can reach M8b's resolver as an "entry" conflict, this is where it stops being a surprise.
- [x] Beyond Go's rows: `vault remove` on a device with a real pending
      request asks before deleting the identity
      (`vault_remove_with_a_real_pending_request_asks_before_deleting`),
      in place of M12b's `identity add` simulation.

### 7. `identity enroll` — 18 rows

- [x] The command: registry entry (group Identity, `Availability::Both`),
      `commands/mod.rs`, `--device`, `--ttl` and `-u`. The global-config
      write, the two secrets' labels, and the session route.
- [x] Reword `identity add`'s and `identity list`'s descriptions if they
      don't already match their fixtures (D-ENROLL-VERBS).
- [x] Rows (`enroll_cli_test.go`):
  - `TestIdentityEnrollPublishesARequestAndPrintsTheCode`: `identity_enroll_publishes_a_request_and_prints_the_code`
  - `TestIdentityEnrollRecordsThisDevicesPubkey`: `identity_enroll_records_this_devices_pubkey`
  - `TestIdentityEnrollHonorsTTLAndDefaultsTo24h`: `identity_enroll_honors_ttl_and_defaults_to_24h`
  - `TestIdentityEnrollRefusesAnUnusableTTL`: `identity_enroll_refuses_an_unusable_ttl`
  - `TestIdentityEnrollRefusesAnInvalidDeviceName`: `identity_enroll_refuses_an_invalid_device_name`
  - `TestIdentityEnrollRefusesANameThatLabelsSomeoneElse`: `identity_enroll_refuses_a_name_that_labels_someone_else`
  - `TestIdentityEnrollTheSecondTimeIsANoOpOnceApproved`: `identity_enroll_the_second_time_is_a_no_op_once_approved`. The key is added by `recipient add`, since approve is E4's.
  - `TestIdentityEnrollRemoteErrorsCarryTheirExitCodes`: `identity_enroll_remote_errors_carry_their_exit_codes`
  - `TestIdentityEnrollSelectsAVaultWithUse`: `identity_enroll_selects_a_vault_with_use`
  - `TestIdentityEnrollWorksInASessionAgainstAVaultThatWasNeverUnlocked`: `identity_enroll_works_in_a_session_against_a_vault_that_was_never_unlocked`
  - `TestUseStillFailsOnAVaultThisDeviceCannotDecrypt`: `use_still_fails_on_a_vault_this_device_cannot_decrypt`. The hint names `identity enroll` (M6's obligation).
  - `TestTheTwoSecretsNeverCross`: `the_two_secrets_never_cross`
  - `TestBothSecretsAreLabelledAtTheirPointOfUse`: `both_secrets_are_labelled_at_their_point_of_use`
  - `TestHumanDurationReadsLikeAPersonSaidIt`: `human_duration_reads_like_a_person_said_it`
  - `TestScriptedEnrollRefusesTheCreatePath`: `scripted_enroll_refuses_the_create_path`. Under `--stdin`, `LockedOrAuth`, with or without `GAGE_PASSPHRASE`.
  - `TestScriptedEnrollSucceedsOnTheReusePath`: `scripted_enroll_succeeds_on_the_reuse_path`
  - `TestARefusedScriptedEnrollLeavesTheFastForwardInPlace`: `a_refused_scripted_enroll_leaves_the_fast_forward_in_place`. The lock is released.
  - `TestScriptFileAtATerminalTakesTheCreatePathNormally`: `script_file_at_a_terminal_takes_the_create_path_normally`

### 8. `clone`'s offer — 8 rows

- [x] Thread the prompter into `clone::run`. The offer through
      `confirm_default_yes`, enrolling under the name `clone` resolved.
      Reword `access_lines`.
- [x] Rows (`clone_enroll_test.go`):
  - `TestInteractiveCloneOffersToEnrollAndYesPublishes`: `interactive_clone_offers_to_enroll_and_yes_publishes`. Same end state as `clone` then `identity enroll`.
  - `TestCloneOfferDefaultsToYes`: `clone_offer_defaults_to_yes`. A bare Enter enrolls; the prompt reads `[Y/n]`.
  - `TestConfirmsDefaultIsUnchangedEverywhereElse`: `confirms_default_is_unchanged_everywhere_else`. `recipient remove` of this device's own key still declines on Enter.
  - `TestCloneOfferDeclinedLeavesTheVaultCloned`: `clone_offer_declined_leaves_the_vault_cloned`. A later `identity enroll` still works; the message names `identity enroll`.
  - `TestNonInteractiveCloneNeverOffersAndExitsZero`: `non_interactive_clone_never_offers_and_exits_zero`. The reworded message.
  - `TestTheOfferTracksWhereACreateCanBeAnswered`: `the_offer_tracks_where_a_create_can_be_answered`. No offer under `--stdin`; an offer under `--script` at a terminal.
  - `TestCloneDoesNotOfferWhenThisDeviceAlreadyHoldsAnIdentity`: `clone_does_not_offer_when_this_device_already_holds_an_identity`. Prompt count zero.
  - `TestCloneDeviceFlagCarriesThroughToTheOffer`: `clone_device_flag_carries_through_to_the_offer`

(Rows: 1 + 10 + 5 + 10 + 8 + 18 + 8 = 60, plus 2 inbound. The Opus
review added three beyond-Go rows to task 3:
`enroll_is_silent_when_the_clock_is_behind_by_less_than_the_threshold`,
`enroll_seals_the_passphrase_method_whatever_config_records` and
`enroll_commits_under_a_message_that_names_nothing`.)

### 9. Opus review

- [x] Mutation pass over the ⚑ rows, run by the implementing model
      (Sonnet). The table below is that pass, and the Opus review extends it.
- [x] Opus review of the ⚑ rows and of "no wrapper answers yes", against
      Go's `enroll.go`, `enrollremote_test.go`, `prompter.go` and
      `clone.go` at the pinned SHA. Its rows are marked *Opus review* in
      the table. All six findings are fixed:
  - [x] **The lock row doesn't kill the pull-before-lock mutant.** This
        was run, not reasoned. With `self.pull()` moved ahead of
        `with_vault_write`, `enroll_holds_the_write_lock_across_the_pull`
        passes, because `pull()` holds its own lock during the fetch the
        child probes. Only `enroll_takes_the_dirty_tree_reset_before_it_pulls`
        failed, and only because the mutant also moved the pull ahead of
        the reset. A "lock, reset, release, `pull()`, lock again" variant
        would survive every row. Go kills it with `onEnrollPulled`, a hook
        fired right after the pull inside `enrollUnderLock`. No existing
        seam can do that job: the `Prompter` and the push both run inside
        the second lock, so a probe there sees it held. Fix: a test-only
        post-pull hook on `Vault` (gated like `with_remote_syncer`), called
        right after `pull_locked` in `enroll_locked`. Move the child-process
        probe there, or add a second probe there (the fetch probe also
        kills "no lock at all").
        **Fixed:** `EnrollHook` (`vault.rs`), shaped like `MoveHook`, and a
        second child-process probe in the lock row. A hook placed after the
        pull isn't enough on its own. Run against a mutant that moved the
        pull and left the hook where it was, the row still passed, and Go's
        `onEnrollPulled` has the same gap. So the pull, the divergence check
        and the hook are one function, `catch_up_for_enroll`. Moving the
        catch-up outside the lock takes the probe with it ("another process
        could take the write lock"). Going around it with `pull()` never
        fires the hook ("the post-pull hook never fired"). Both mutants were
        run, and the lock row kills each.
  - [x] **`confirm_default_yes` accepts more than Go does.** Go's
        `ConfirmDefaultYes` accepts only `""`, `y` and `yes`, and every
        other answer declines. The terminal prompter here declines only `n`
        and `no`, and `confirm_default_yes_defaults_to_yes` pins
        `"garbage" → true`. So a typo such as `nope` or `q` at clone's
        offer goes on to create an identity, commit and push. Go wins on
        behavior and no `Q-` entry records this. Fix: match Go, and flip
        the row's garbage case to `false`. EOF is already correct: it is
        `PromptError::Cancelled`, not yes. **Fixed**, and the row now also
        pins `nope` → no and blank → yes.
  - [x] **The clock warning has no threshold.** Go warns only when HEAD is
        at least `enrollmentClockSkewThreshold` (1h) ahead of local time.
        `warn_if_clock_is_behind_head` warns on any lead, so ordinary
        seconds-to-minutes drift against another device's commit warns.
        Neither Go's rows nor ours test under an hour. Fix: the 1h constant,
        plus a beyond-Go row where a commit 10 minutes ahead stays silent.
        **Fixed:** `CLOCK_SKEW_THRESHOLD`, and
        `enroll_is_silent_when_the_clock_is_behind_by_less_than_the_threshold`.
  - [x] **The sealed method should be `METHOD_PASSPHRASE`, as in Go.**
        `enroll_locked` seals the recorded method, falling back to the
        vault's `[method].default`. The payload's method describes the
        identity file `identityfile::create` just wrote, which is always
        passphrase-wrapped. Today `ALLOWED_METHODS` is passphrase only, so
        nothing observable breaks. But a default from a newer or
        hand-edited config fails `seal_enrollment` with `UnknownMethod`
        *after* the identity is written, which breaks "refused before
        anything happens". The handler then records that method in global
        config too. **Fixed:** `METHOD_PASSPHRASE`, and the beyond-Go
        `enroll_seals_the_passphrase_method_whatever_config_records`
        (global config records `yubikey`).
  - [x] **Commit message.** Go commits `gage: enrollment request`; this
        commits `enrollment request <id>`. Nothing leaks, since the id is
        already in the filename, but the commit layout is Go's. Fix: Go's
        message. **Fixed:** `ENROLLMENT_COMMIT_MESSAGE`, and the beyond-Go
        `enroll_commits_under_a_message_that_names_nothing`.
  - [x] **`divergence_is_found_by_the_report_rather_than_an_error` re-uses
        `diverged_pair`,** so it adds only the variant match to
        `enroll_refuses_a_diverged_pull`. Go builds this row's divergence
        without enroll (an unrelated local commit), and first asserts the
        trap's premise: the pull returns `Ok` with `diverged` set. Fix:
        port both, through `pull()` as Go does, before the enroll call.
        **Fixed.**

| Mutation | Killed by |
|---|---|
| `enroll` calls `pull()` (self-locking) instead of `pull_locked` | 27 enroll rows, by contention with its own lock; `enroll_holds_the_write_lock_across_the_pull` among them |
| `with_write_lock` instead of `with_vault_write` | `enroll_takes_the_dirty_tree_reset_before_it_pulls`, `the_next_write_discards_a_stray_pending_file`, `a_stray_does_not_wedge_the_joining_device` |
| The divergence check removed (commit onto a diverged tip) | `enroll_refuses_a_diverged_pull`, `divergence_is_found_by_the_report_rather_than_an_error` |
| The lock taken after the pull | Not separately killed. `enroll_holds_the_write_lock_across_the_pull` probes the lock from a child process during the fetch, which kills "no lock at all" and the `pull()` mutant, but a pre-lock `pull()` that takes its own lock would also pass it. Nothing observable sits between the pull and the commit to probe without a new seam (the identity prompt could host one). Open for the Opus review. |
| The remote check after the identity is created | `enroll_with_no_remote_fails_with_its_own_error` (check removed outright; "moved after" is not separately mutated, and the unreachable row covers the fetch ordering) |
| The name check before the key check | `enroll_on_a_device_that_is_already_a_recipient_is_a_success_that_does_no_work` |
| `confirm_default_yes` wired to `confirm` | `confirm_default_yes_defaults_to_yes`, `clone_offer_defaults_to_yes` |
| A wrapper answering `confirm_default_yes` with `Ok(true)` | `no_non_interactive_prompter_answers_confirm_default_yes` (`AssumeYes`), and the delegate macro by that row plus `every_wrapper_passes_through_each_method_it_does_not_answer` |
| The offer gated on a TTY test instead of create-ability | `non_interactive_clone_never_offers_and_exits_zero`, `the_offer_tracks_where_a_create_can_be_answered` (gate forced open). See the clone note below. |
| *Opus review:* `pull()` moved ahead of `with_vault_write` (run) | **Survived the lock row** as first written. Killed only incidentally by `enroll_takes_the_dirty_tree_reset_before_it_pulls`, since the pull also moved ahead of the reset. Now killed by `enroll_holds_the_write_lock_across_the_pull` in both forms: the catch-up moved outside the lock, and `pull()` around it (above). |
| *Opus review:* `confirm_default_yes` treats an unrecognised answer as yes | Was the shipped behavior, unlike Go's. Fixed; now killed by `confirm_default_yes_defaults_to_yes` (`garbage`, `nope`). |
| *Opus review:* the clock warning fires on any lead, not 1h | Was the shipped behavior, unlike Go's. Fixed; now killed by `enroll_is_silent_when_the_clock_is_behind_by_less_than_the_threshold`. |
| *Opus review:* a wrapper gains a default-yes body | Can't be written: the trait method has no default body, so every one of the eight impls (terminal, `FakePrompter`, `Passthrough`, `EnvPrompter`, `RefusingPrompter`, `NoCreatePrompter`, `NoChoosePrompter`, `AssumeYes`) is compiled against it. An explicit `Ok(true)` in any wrapper is killed by `no_non_interactive_prompter_answers_confirm_default_yes`, which covers all six wrappers. |
| *Opus review:* the unreachable fetch maps to `Sync`, not `RemoteUnreachable` | `enroll_with_an_unreachable_remote_fails_before_anything_is_created` (variant and `Unreachable` code) |
| *Opus review:* a push failure returned as a warning (`push_after_write`) | `enroll_push_rejected_for_write_access_keeps_what_it_made` (it `unwrap_err`s the enroll) |

Clone note: `clone` is one-shot only in gage-rs, so `--stdin` and
`--script` never reach the offer, and the create-ability rule reduces to
`stdin_is_terminal`. `the_offer_tracks_where_a_create_can_be_answered`
asserts a terminal offers, no terminal doesn't, and a `--stdin` `clone` is
refused before it could. Go's `--script FILE at a terminal` offer row has no
Rust counterpart for that reason.

### 10. Parity, completion and ledger

- [x] Fixture parity for `identity enroll` (name, group, usage, flags and
      defaults) and the `identity` child list, in a
      `p7_identity_enroll_has_its_go_fixture_flags_and_descriptions` test.
      Update `p5_identity_commands_have_their_go_fixture_flags_and_descriptions`'s
      child-list note.
- [x] Completion: `subcommand_candidates_lists_identity_children`
      expects `["add", "enroll", "list"]` (`complete_tests.rs`).
- [x] Parity ledger: §2 `c86ad4c`, §3 E3, §4 `clone`'s offer, §5
      `identity_enroll`, the P6 `HeadCommitterTime` row and the P2
      prompter row, marked `[x]` after CI. (§3's row covers E1b–E4
      together, and §5's row shares `identity_enroll` with E4's three
      commands, so both stay open until E4 ships; the two inbound rows
      are accounted in §5's P7 note, which has no checkbox.)
- [x] Mark E3 `[x]` in the [P7 index](index.md) after CI. (CI green on
      Linux, macOS and Windows at `8cb2ed5`.)

## Definition of done

Every box above is checked. All 60 claimed rows and the 2 inbound rows are
ported, and the review table is filled in. `make lint` and `make test` are
clean locally and on Linux, macOS and Windows CI.

The milestone is done when:
- `gage clone URL` on a new device, answered with Enter and a new
  passphrase, leaves an inert request on the remote and a code on screen
- an interrupted, rejected or diverged enroll never wedges the device:
  re-running `identity enroll` is always the way forward
- no script, and no `--yes`, ever answers the offer
