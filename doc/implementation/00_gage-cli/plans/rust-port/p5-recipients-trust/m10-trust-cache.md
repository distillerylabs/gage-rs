# M10 — Trust cache

[← M9b](m9b-recipient-write.md) · [P5 plan](index.md) · [#44](https://github.com/distillerylabs/gage-rs/issues/44) · next: [M11](m11-cross-vault.md)

> **Recommended model: Sonnet\* ⚑.** Borderline. The two-outcome
> regeneration rule is subtle but fully specified, and the hook placement
> is the part to get right: the encrypting methods, unlock, and `sync`
> separately. Run an **Opus review of the laundering and refusal-ordering
> tests** (task 7) before calling it done. Both of Go's post-implementation
> corrections were in exactly that area. Switch to Opus at the milestone
> boundary if the hook placement turns awkward.

## Goal

An unreviewed change to a vault's recipient list is detected and shown as
a plaintext diff before the next encrypt. This is the backstop behind the
design's weakest guarantee: anyone with git write access can append a key
to `.age-recipients`.

- The cache is `$GAGE_STATE/<vault-id>/known-config.toml` (a verbatim
  copy of `.gage/config.toml`) plus the SHA-256 of `.age-recipients`.
  Diffing only the first would miss an edit to the second. The cache is
  written silently on the first *successful* use, and never committed.
- **Blocking check** before every encrypt (`insert`, `edit`, `generate`,
  `rename`, and M11's `mv`/`cp` on the destination). It asks through
  `Prompter::confirm_recipient_change`. Declining aborts the write with no
  commit and leaves the cache as it was.
- **Opportunistic check** on unlock and on `sync`, including a clean
  fast-forward that never unlocks. It warns and never blocks.
- **Two outcomes.** Confirming a *routine* change (the two files agree)
  regenerates both halves. Confirming a *mismatched* one never clears the
  cache, and `verify` keeps failing.
- `recipient add`/`remove` are **refused over a failing `verify`**,
  naming the differences and leaving both files byte-identical. They also
  **ask about a change this device hasn't reviewed** before writing
  anything. A successful one regenerates the cache, so the operator isn't
  warned about their own change.
- `recipient verify --repair` rewrites `.age-recipients` from
  `config.toml` behind a confirmation that names every key dropped and
  added, and commits `gage: recipient repair`. It does *not* regenerate
  the cache.
- A root `--yes` answers `confirm_recipient_change` (only that) for
  scripted runs. It is refused in a session.
- `vault remove` deletes the cache, so re-adding the vault is a first use.

## Depends on

- **M9b:**
  - `with_vault_write`
  - `add_recipient` / `remove_recipient` and their single commit
  - `write_recipient_files`
- **M9a:**
  - `verify_recipients`
  - the two-device fixture (a pulled recipient change)
- **P4 · M8a2:** `sync_on_unlock` (`remotesync.rs`), `Vault::sync`, and
  `Session::use_vault`, which also calls `sync_on_unlock`.
- **P4 · M8b:**
  - `sync_resolving_with` (`syncresolve.rs`, the "M10 adds its trust-cache
    confirmation" comment)
  - `report_conflicts` (`commands/sync.rs`)
  - `yes_does_not_resolve_conflicts`
- **P1:** `paths::trust_cache_dir` (exists, no caller yet), `atomicfile`.

## Design references

- Design doc, "Trust boundaries": the two questions that get conflated.
- Design doc, "Local trust cache": what's cached, when it's checked, the
  diff prompt.
- Design doc, "Cache regeneration isn't one-size-fits-all".

## Go sources

- Go plan `gage-cli-design/m10-trust-cache.md`: all of it, including:
  - the refusal over a failing verify, and `--repair` as its exit
  - `--yes` not being a library concept, and being refused in a session
  - `vault remove` deleting the cache
  - first-use bootstrap only on success
  - local add/remove regenerating the cache
  - the corrected decision that add/remove also run the blocking check
- Go plan `gage-cli-init-design/e0-vault-id-keying.md`: the cache is keyed
  by id.
- Parity ledger rows:
  - §3 M10
  - §4 `recipient_verify` `--repair`, and `sync --yes`
  - §2 `0d710d4` E0, the trust-cache half
- Go sources at the pinned SHA:
  - `internal/gage/trustcache.go` (all of it, including `unifiedDiff` and
    `longestCommonSubsequence`)
  - `internal/gage/vault.go`: `withEncryptingWrite`
  - `internal/gage/unlock.go` and `remotesync.go`: the opportunistic hooks
  - `cmd/gage`: the terminal prompter's `ConfirmRecipientChange`, the
    `--yes` wrapper, `runRecipientRepair`, and `vault remove`
- Test inventory, 38 rows:
  - `internal/gage/trustcache_test.go` (25)
  - `cmd/gage/trustcache_cli_test.go` (11)
  - `cmd/gage/recipient_cli_test.go` (2 of 15)
- Inbound rows (3), plus the P4 follow-ups: see task 6.

## Decisions to make first

- **The prompter method**
  ([Q-RECIPIENT-PROMPT](../open-questions.md#q-recipient-prompt)):
  `Prompter::confirm_recipient_change(&RecipientChangeWarning) -> Result<bool, PromptError>`.
  - Its default returns `PromptError::NotInteractive`, as
    `resolve_conflict` does. `TestConfirmRecipientTrustWithNoPrompterRefuses`
    is ported as that default.
  - `FakePrompter` gets `with_recipient_answers`.
  - `TerminalPrompter` renders the design doc's diff and `[y/N]`, and
    still refuses when it isn't interactive.
  - The opportunistic checks go through `warn` with the warning's
    one-line summary.
- **`RecipientChangeWarning`:**
  - fields: `vault`, `diff`, `verification`, `added`, `removed`,
    `last_confirmed`
  - methods: `mismatched()` and `summary()`
  - It is a typed value, never printed by the library.
- **`--yes` in Rust terms.** The library decides regeneration from
  `verify_recipients`, so *who* answered can't change it. `--yes` is a
  `gage-cli` wrapper prompter that answers only
  `confirm_recipient_change`, never `confirm`, so M9b's self-removal
  question stays real. It is the first root persistent flag (`cli.rs`).

  It is refused:
  - on a session line, because the session's writes ask through the
    prompter the session holds, so a per-line `--yes` would do nothing
  - on the way into a session (`gage --yes` on a terminal), because it
    would wrap that prompter for the session's whole life

  Go's argument goes through `ident.frontend()`; the Rust version goes
  through the session's own prompter. Record it in the Q- entry.
- **Where the hooks sit.**
  - **Blocking:** an `with_encrypting_write` (Go's name) wrapping
    `with_vault_write` with the trust confirmation, used by `insert`,
    `update` (edit), `generate` and `rename`. M11 uses it on the
    destination. `rm` encrypts nothing and doesn't use it.
  - **Opportunistic:** in `Vault::unlock` after `sync_on_unlock` and only
    once the unlock has succeeded (a wrong passphrase must not bootstrap
    the cache), and in `sync_resolving` once its pull has succeeded (see
    "Resolved at the start of implementation": not `Vault::sync`).
  - The session path: `Session::use_vault` re-runs `sync_on_unlock` for
    a held vault but does not warn, as Go's `Session.Use` doesn't
    (resolved below).
- **The order inside add/remove** is:
  1. `with_vault_write`
  2. `require_recipients_in_sync` (nobody is asked to approve an
     operation that is about to be refused)
  3. `confirm_recipient_trust`
  4. the first write
  5. `note_recipients_reviewed` on success

  The refusal sits ahead of the re-encryption, never beside the
  recipient-file write.
- **Hash and diff** ([Q-TRUST-HASH](../open-questions.md#q-trust-hash)):
  add `sha2` as a direct dependency at the version `age` already uses, and
  check `cargo tree -d` adds no second copy. Port Go's LCS unified diff
  with its test. Check `make audit` and the dependency purity check stay
  green.
- **The cache files**
  ([Q-TRUST-CACHE-PATH](../open-questions.md#q-trust-cache-path)):
  - Use Go's file names inside `trust_cache_dir`.
  - A half-written pair reads as "no record", so it is bootstrapped again.
  - The mismatch-seen record follows Go's `noteRecipientMismatchSeen`.

### Resolved at the start of implementation

Reading Go's code at the pin settled the points above and turned up five
the plan left open or got wrong. Each is recorded in
[Q-RECIPIENT-PROMPT](../open-questions.md#q-recipient-prompt) or
[Q-TRUST-CACHE-PATH](../open-questions.md#q-trust-cache-path).

- **The sync hook is in `sync_resolving`, after the pull, not in
  `Vault::sync`.** The plan put it in `Vault::sync`. Go's code puts it in
  `SyncResolving`, between the pull and the push, and leaves `Vault.Sync`
  alone because it takes no prompter. The same holds here: `Vault::sync`
  takes none, and `gage sync` goes through `sync_resolving` in both modes.
  The warning runs once the pull has succeeded, whatever the push does
  next: the list worth warning about is the one that just landed.
- **Re-selecting a held vault in a session does not warn.** Go's
  `Session.Use` re-runs `syncOnUnlock` for a held vault but not
  `warnRecipientTrust`. The unlock already warned, and a change that a
  re-`use` pulls in is still caught by the blocking check before the next
  encrypt. gage-rs ports Go's behavior.
- **`TerminalPrompter::confirm_recipient_change` refuses when it isn't
  interactive**, before drawing anything or reading a line, exactly as
  its `resolve_conflict` does. Go's renders and reads a line regardless.
  On piped input that line belongs to someone else: `printf 'pass\nvalue'
  | gage insert x --value-stdin` would spend the value as the answer.
  Refusing keeps the outcome Go's bare-Enter default gives (no write,
  exit Conflict) without consuming input. `--yes` is how a script says
  yes. Recorded as a deliberate deviation in Q-RECIPIENT-PROMPT.
- **`--yes` with no script mode.** Go lets `--yes` through in a
  `--script`/`--stdin` session, since CI is the case the flag exists for.
  gage-rs has no script mode yet: every session is a terminal session,
  so both refusals apply unconditionally. Whoever ports script mode adds
  Go's exception with it.
- **Errors.** A new `trustcache` module owns `TrustError`:
  - `ChangeDeclined { vault }`: the answer was no.
  - `Unconfirmable { vault, source: PromptError }`: nobody could answer,
    including the trait default's `NotInteractive`. This is the port of
    Go's nil-prompter `ErrRecipientChangeDeclined`.
  - `OutOfSync { vault, verification }`: add/remove refused, naming
    every difference and pointing at `--repair`.
  - plus the I/O, TOML, config and verify failures.

  All three refusals exit Conflict, as Go's do. `WriteError` and
  `RecipientError` each gain a `Trust(TrustError)` variant. Repair
  returns `RecipientError` with a new `RepairDeclined { vault }` variant
  (Conflict).

## Tasks

Write each task's tests first, at task granularity.

### 1. Cache storage, the warning value and the diff — 6 rows

- [x] Load and store under `trust_cache_dir`. `RecipientChangeWarning`
      and the diff.
- [x] Rows:
  - `TestTrustCacheStoresBothHalves`
  - `TestTrustCacheLivesUnderStateAndIsNeverCommitted`: `git status` is
    clean after a regeneration.
  - `TestTheRecipientWarningIsATypedValueNotPrintedText`
  - `TestDeltaWithNoRecipientChangeSaysEditedAlone`
  - `TestLongestCommonSubsequenceHandlesPureInsertionAndEmptyInput`
  - `TestFirstUseWritesTheCacheThenAnUnreviewedChangeWarns`

### 2. The blocking check — 7 rows

- [x] `confirm_recipient_change` on `Prompter`, `with_encrypting_write`,
      and the two-outcome rule.
- [x] Rows:
  - `TestBlockingCheckHooksEveryEncryptingVaultMethod`: `insert`,
    `update`, `generate` and `rename`, with no `Session` involved.
  - `TestRecipientsFileEditAloneStillWarns`
  - `TestDecliningTheRecipientChangeAbortsTheWrite`: no commit, and the
    cache is unchanged, so the warning returns.
  - `TestConfirmingARoutineChangeRegeneratesBothHalves`
  - `TestConfirmingAMismatchedChangeDoesNotClearTheCache`
  - `TestConfirmRecipientTrustWithNoPrompterRefuses`: the default
    method.
  - `TestReencryptRefusalHappensBeforeAnyEntryIsRewritten`

### 3. The opportunistic checks — 2 rows

- [x] Hook `Vault::unlock` (success only) and `sync_resolving` (after
      its pull).
- [x] Rows:
  - `TestUnlockWarnsOpportunisticallyWithoutBlocking`
  - `TestSyncWarnsOnACleanFastForward`: no unlock happens.

### 4. Recipient verbs: refusal, trust and regeneration — 5 rows

- [x] Rows:
  - `TestRecipientAddAndRemoveRefuseOverAFailingVerify`: both verbs, with
    both files byte-identical afterwards, nothing committed or pushed,
    and the cache unchanged.
  - `TestRecipientAddDoesNotLaunderSomeoneElsesChange`: a consistent
    change pulled through the two-device fixture. Declined re-encrypts
    nothing. Confirmed regenerates once and covers both keys.
  - `TestConfirmingLetsTheAddThroughAndRegeneratesOnce`
  - `TestALocalRecipientAddRegeneratesTheCache`
  - `TestVerifyAndRepairReportOnlyInConfig`

### 5. `verify --repair` — 5 library rows + 2 CLI rows

- [x] `Vault::repair_recipients(&self, prompter) -> Result<RecipientRepair, _>`:
      under `with_vault_write`, behind `confirm` (naming every key
      dropped and added), committed as `gage: recipient repair`, with no
      cache regeneration.
- [x] Rows:
  - `TestRepairRewritesTheRecipientsFileFromConfig`
  - `TestRepairDeclinedWritesNothing`
  - `TestRepairWithNoPrompterRefuses`
  - `TestRepairOnAnAlreadyInSyncVaultDoesNothing`
  - `TestRepairDoesNotRegenerateTheTrustCache`
  - CLI: `TestRecipientVerifyRepairReportsWhatItAdded`,
    `TestRecipientVerifyRepairOnAVaultInSyncSaysThereIsNothingToDo`

(Tasks 1–5 total 25 library rows: 6 + 7 + 2 + 5 + 5.)

### 6. CLI: `--yes`, rendering, `vault remove` — 11 rows, 3 inbound, 2 P4 follow-ups

- [x] Root `--yes`, the wrapper prompter, and both session refusals.
      `TerminalPrompter::confirm_recipient_change` renders the diff and
      `[y/N]` on stderr.
- [x] `vault remove` deletes `trust_cache_dir`.
- [x] `trustcache_cli` rows:
  - `TestInsertBlocksOnAnUnreviewedRecipientChange`
  - `TestReadsWarnOpportunisticallyInOneShotMode`: `ls` and `show` warn
    on stderr, stdout carries only the entry, and nothing blocks, asks
    or regenerates.
  - `TestRecipientAddIsRefusedOverADivergedVaultFromTheCLI`
  - `TestRecipientVerifyRepairFixesTheDivergence`
  - `TestSessionModeAsksTheSameRecipientQuestion`: the real terminal
    prompter in a session.
  - `TestTerminalPrompterRendersTheRecipientChangeAsTheDesignDocPrompt`
  - `TestVaultRemoveDeletesTheTrustCache`
  - `TestYesBypassesTheRecipientPromptForScripting`
  - `TestYesDoesNotAnswerTheSelfRemovalConfirmation`
  - `TestYesDoesNotRegenerateTheCacheOnAMismatch`
  - `TestYesIsRefusedInASession`: on a line and on the way in.
- [x] Inbound:
  - `prompter` `TestConfirmRecipientChangeNamesBothDirectionsOfAMismatch`
    (from P2)
  - `prompter` `TestRecipientChangeCountSummarizesEveryShape` (from P2)
  - `vaultid_keying` `TestTwoVaultsUnderOneLocalNameShareNeitherDirectory`
    (from P1): neither the identities nor the trust cache are shared
    between two vaults registered under one local name.
- [x] P4 follow-ups:
  - `yes_does_not_resolve_conflicts` (M8b): with `--yes` now a real root
    flag, it must still not resolve a conflict. Update the test's
    premise, not just its parse, and record that in M8b's doc.
  - The recipient-file conflict summary that ends "run `gage sync`": look
    at what Go's `sync` does with a recipient-file conflict once M10 is
    in (`syncresolve.go`), and port that. If Go still refuses, keep the
    refusal and make the summary name what to do next, citing Go.
- [x] Extend the P5 fixture-parity test: `recipient verify --repair`, and
      `--yes` on the root.

### 7. Opus review and ledger

- [x] **Opus review (⚑)** of the tests in task 4, the session refusals in
      task 6, and `TestReencryptRefusalHappensBeforeAnyEntryIsRewritten`.
      For each one, name the mutation it kills, as M8b did:
  - the regeneration moved before the confirmation
  - the in-sync check moved after the re-encryption
  - `--yes` answering `confirm`
  - the cache written on a failed unlock

  Record the table here.

  Each mutation was applied to the source, the suites run, and the file
  restored. Every one fails at least the test named:

  | Mutation | Killed by |
  |---|---|
  | The regeneration moved before the confirmation | `declining_the_recipient_change_aborts_the_write`, `recipient_add_does_not_launder_someone_elses_change` |
  | The in-sync check moved after the re-encryption pass | `reencrypt_refusal_happens_before_any_entry_is_rewritten`, `recipient_add_and_remove_refuse_over_a_failing_verify` |
  | `--yes` answering `confirm` | `yes_does_not_answer_the_self_removal_confirmation` |
  | `--yes` answering `resolve_conflict` | `yes_does_not_resolve_conflicts` (P4, premise updated) |
  | The cache written on a failed unlock (check before the passphrase) | `a_failed_unlock_does_not_bootstrap_the_cache` |
  | A session line's `--yes` honored | `yes_is_refused_in_a_session` (on a line) |
  | `gage --yes` let into a session | `yes_is_refused_in_a_session` (on the way in) |
  | A confirmed mismatch regenerating the cache | `confirming_a_mismatched_change_does_not_clear_the_cache` |
  | `add`/`remove` skipping the trust question | `recipient_add_does_not_launder_someone_elses_change`, `confirming_lets_the_add_through_and_regenerates_once`, `recipient_remove_asks_about_an_unreviewed_change_before_anything_else` |
  | `--repair` regenerating the cache | `repair_does_not_regenerate_the_trust_cache` |

  Tests beyond Go's rows, each for a property the rows leave implicit:
  `removing_an_entry_asks_nothing`, `a_failed_unlock_does_not_bootstrap_the_cache`,
  `a_half_written_cache_reads_as_no_record`, `an_unparseable_trust_record_is_an_error`,
  `summary_says_what_to_do_next`,
  `recipient_remove_asks_about_an_unreviewed_change_before_anything_else`,
  `recipient_verify_repair_declined_writes_nothing`, and
  `non_interactive_prompter_refuses_the_recipient_question`.
- [x] Parity ledger:
  - §3 M10
  - §4 `sync --yes`, `recipient_verify` `--repair`
  - §2 E0, the trust-cache half
- [x] Mark M10 `[x]` in the [P5 index](index.md) after CI (green on
      Linux, macOS and Windows at `522c9dd`, [#49](https://github.com/distillerylabs/gage-rs/pull/49)).

## Definition of done

Every box above is checked. All 38 claimed rows and the 3 inbound ones
are ported or excused in writing. The two P4 follow-ups are closed and
the review table is recorded. `make lint` and `make test` are clean
locally and on Linux, macOS and Windows CI.

The milestone is done when:
- a recipient added by another device produces a legible diff and one
  prompt on this one
- a hand-edited `.age-recipients` produces a warning that doesn't go away
  by answering yes, or by running `recipient add`, which refuses
- `recipient add` asks about a change this device hasn't reviewed before
  it writes or re-encrypts anything
