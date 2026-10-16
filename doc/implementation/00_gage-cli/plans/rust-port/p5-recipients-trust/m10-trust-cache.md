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
    the cache), and in `Vault::sync` after a successful sync.
  - Confirm the session path: `Session::use_vault` re-runs
    `sync_on_unlock` for a held vault. Decide whether it warns there too,
    as Go's unlock hook does on every unlock.
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

## Tasks

Write each task's tests first, at task granularity.

### 1. Cache storage, the warning value and the diff — 6 rows

- [ ] Load and store under `trust_cache_dir`. `RecipientChangeWarning`
      and the diff.
- [ ] Rows:
  - `TestTrustCacheStoresBothHalves`
  - `TestTrustCacheLivesUnderStateAndIsNeverCommitted`: `git status` is
    clean after a regeneration.
  - `TestTheRecipientWarningIsATypedValueNotPrintedText`
  - `TestDeltaWithNoRecipientChangeSaysEditedAlone`
  - `TestLongestCommonSubsequenceHandlesPureInsertionAndEmptyInput`
  - `TestFirstUseWritesTheCacheThenAnUnreviewedChangeWarns`

### 2. The blocking check — 7 rows

- [ ] `confirm_recipient_change` on `Prompter`, `with_encrypting_write`,
      and the two-outcome rule.
- [ ] Rows:
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

- [ ] Hook `Vault::unlock` (success only) and `Vault::sync`.
- [ ] Rows:
  - `TestUnlockWarnsOpportunisticallyWithoutBlocking`
  - `TestSyncWarnsOnACleanFastForward`: no unlock happens.

### 4. Recipient verbs: refusal, trust and regeneration — 5 rows

- [ ] Rows:
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

- [ ] `Vault::repair_recipients(&self, prompter) -> Result<RecipientRepair, _>`:
      under `with_vault_write`, behind `confirm` (naming every key
      dropped and added), committed as `gage: recipient repair`, with no
      cache regeneration.
- [ ] Rows:
  - `TestRepairRewritesTheRecipientsFileFromConfig`
  - `TestRepairDeclinedWritesNothing`
  - `TestRepairWithNoPrompterRefuses`
  - `TestRepairOnAnAlreadyInSyncVaultDoesNothing`
  - `TestRepairDoesNotRegenerateTheTrustCache`
  - CLI: `TestRecipientVerifyRepairReportsWhatItAdded`,
    `TestRecipientVerifyRepairOnAVaultInSyncSaysThereIsNothingToDo`

(Tasks 1–5 total 25 library rows: 6 + 7 + 2 + 5 + 5.)

### 6. CLI: `--yes`, rendering, `vault remove` — 11 rows, 3 inbound, 2 P4 follow-ups

- [ ] Root `--yes`, the wrapper prompter, and both session refusals.
      `TerminalPrompter::confirm_recipient_change` renders the diff and
      `[y/N]` on stderr.
- [ ] `vault remove` deletes `trust_cache_dir`.
- [ ] `trustcache_cli` rows:
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
- [ ] Inbound:
  - `prompter` `TestConfirmRecipientChangeNamesBothDirectionsOfAMismatch`
    (from P2)
  - `prompter` `TestRecipientChangeCountSummarizesEveryShape` (from P2)
  - `vaultid_keying` `TestTwoVaultsUnderOneLocalNameShareNeitherDirectory`
    (from P1): neither the identities nor the trust cache are shared
    between two vaults registered under one local name.
- [ ] P4 follow-ups:
  - `yes_does_not_resolve_conflicts` (M8b): with `--yes` now a real root
    flag, it must still not resolve a conflict. Update the test's
    premise, not just its parse, and record that in M8b's doc.
  - The recipient-file conflict summary that ends "run `gage sync`": look
    at what Go's `sync` does with a recipient-file conflict once M10 is
    in (`syncresolve.go`), and port that. If Go still refuses, keep the
    refusal and make the summary name what to do next, citing Go.
- [ ] Extend the P5 fixture-parity test: `recipient verify --repair`, and
      `--yes` on the root.

### 7. Opus review and ledger

- [ ] **Opus review (⚑)** of the tests in task 4, the session refusals in
      task 6, and `TestReencryptRefusalHappensBeforeAnyEntryIsRewritten`.
      For each one, name the mutation it kills, as M8b did:
  - the regeneration moved before the confirmation
  - the in-sync check moved after the re-encryption
  - `--yes` answering `confirm`
  - the cache written on a failed unlock

  Record the table here.
- [ ] Parity ledger:
  - §3 M10
  - §4 `sync --yes`, `recipient_verify` `--repair`
  - §2 E0, the trust-cache half
- [ ] Mark M10 `[x]` in the [P5 index](index.md) after CI.

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
