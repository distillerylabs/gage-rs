# M13 — Tab completion & command abbreviation (session mode)

[← M7](m7-metadata-index.md) · [P3 plan](index.md) · [#25](https://github.com/distillerylabs/gage-rs/issues/25) · next: P4 (planned when it starts)

> **Recommended model: Sonnet\*.** Wiring against settled abstractions
> (the registry, the clap tree, M7's index, M6's `rustyline` reader). The
> borderline part is the `rustyline::Completer` integration, which needs
> the `Session` while the editor owns the helper. Switch to Opus at a
> milestone boundary if that turns awkward.

## Goal

Two conveniences in the interactive REPL only (not one-shot mode, not
shell completion scripts):

1. **Tab completion** at the `gage>` prompt: command and subcommand
   names and aliases, flags, vault names (for `use`, `lock` and `--use`),
   and entry titles for query arguments.
2. **Unambiguous top-level abbreviation** with no Tab: `ident` + Enter
   runs `identity` if it is the only session-visible command starting
   with `ident`.

Both share one prefix-match primitive over the registry. Completion never
unlocks anything, and never offers another vault's titles.

## Depends on

- **M7**: the per-vault index (titles), built lazily.
- **M6**: the REPL, session dispatch, the `RustylineReader`, history.
- **P1 · M0b**: the registry and the clap tree built from it.

## Design references

- Design doc "Session model", "Session-only commands"; Go's
  Q-HELP-SURFACES and Q-CMD-AVAILABILITY (the candidate sets follow the
  same availability rules as session help).
- Design doc "Addressing entries & the metadata index": M5's substring
  resolver is deliberately **not** reused here.
- [Q-LINE-EDITOR](../open-questions.md#q-line-editor).

## Go sources

- Go plan `gage-cli-design/m13-tab-completion.md` (Go #25 / PR #63).
- Parity ledger rows: §2 `0aa325c`; §3 M13 (with M5–M7);
  §6 the completion transcript in `session-mode.md`.
- Go sources at the pinned SHA: `cmd/gage/complete.go`,
  `cmd/gage/lineread.go` (`AutoComplete` wiring), `cmd/gage/repl.go`
  (`runSessionCommand`'s first-token resolution),
  `internal/gage/session.go` (`EntryTitleCandidates`).
- Test inventory: `complete` (41), `complete_pty` (1), and the 4
  `TestEntryTitleCandidates*` rows of `internal/gage/session_test.go`:
  46 rows.

## Decisions to make first

Go's decisions carry over unchanged; recommended answers for the
Rust-specific parts follow. Confirm them when this plan is reviewed.

- **Inherited from Go**: build both behaviors; prefix match only (`enti`
  never matches `identity`); a resolver separate from M5's; abbreviation
  reaches top-level commands only, while subcommands and flags are
  Tab-completable; entry titles come from the current vault only; a
  locked current vault offers no titles and never prompts; an ambiguous
  abbreviation lists the candidates, runs nothing and keeps the session;
  `init` and `clone` are in no candidate set; no `gage completion
  bash|zsh|fish`.
- **Library half**: `Session::entry_title_candidates() -> Vec<String>`
  returns the current vault's titles if it is unlocked (building the
  index at most once), and an empty list if there is no current vault or
  it is locked, without touching the prompter.
- **Candidate functions** live in `gage-cli` as plain functions callable
  without a terminal: `prefix_matches`, `top_level_session_names`,
  `resolve_session_command_name`, `completion_words`,
  `flag_candidates(&clap::Command)` (skipping hidden args),
  `subcommand_candidates`, `vault_name_candidates` (global config's
  registry), `leaf_positional_candidates`, and `completion_candidates`
  composing them. Flag candidates are read from the real clap `Command`
  built from the registry, never a second list.
- **`rustyline` wiring**: a `Helper` type implementing `Completer` (the
  rest of `Helper`'s traits as no-ops), holding shared access to the
  session (`Rc<RefCell<…>>`, since `Completer::complete` takes `&self`
  and runs only while no command is executing). `complete` returns the
  start of the partial word and its candidates; `rustyline` does the
  common-prefix fill and the listing. Completion never touches history;
  only submitted lines are added (M6).
- **Abbreviation** runs in session dispatch on the first token, before
  the clap parse; exact names and aliases pass through unchanged, and an
  unknown word passes through to clap's own unknown-command usage error.

## Tasks

Tests first, at task granularity.

### 1. Library — 4 `session_test` rows

- [x] `TestEntryTitleCandidatesListsCurrentVaultTitlesOnly`,
      `TestEntryTitleCandidatesEmptyWhenLockedNeverPrompts`,
      `TestEntryTitleCandidatesEmptyWithNoCurrentVault`,
      `TestEntryTitleCandidatesBuildsIndexOnce`.

### 2. The prefix primitive and abbreviation — 14 `complete` rows

- [x] `TestPrefixMatchesReturnsEveryPrefixHit`,
      `TestPrefixMatchesRejectsSubstring`, `TestPrefixMatchesDedupesAndSorts`.
- [x] `TestTopLevelSessionNamesIncludesGroupsAndAliases`,
      `TestTopLevelSessionNamesExcludesInitAndClone`.
- [x] `TestResolveSessionCommandNameExactNameUnchanged`,
      `TestResolveSessionCommandNameExactAliasUnchanged`,
      `TestResolveSessionCommandNameUnambiguousAbbreviation`,
      `TestResolveSessionCommandNameAmbiguousAbbreviationReportsCandidates`,
      `TestResolveSessionCommandNameSubstringNotAbbreviation`,
      `TestResolveSessionCommandNameNeverReachesInitOrClone`,
      `TestResolveSessionCommandNameUnknownWordUnchanged`,
      `TestResolveSessionCommandNameOnAnEmptyWord`.
- [x] `TestAbbreviationDoesNotReachSubcommands`.

### 3. Candidate sources — 12 `complete` rows

- [x] Word splitting: `TestCompletionWordsOnNothingTyped`,
      `TestCompletionWordsMidWord`, `TestCompletionWordsAfterASpace`,
      `TestCompletionWordsMultipleWordsPartialLast`.
- [x] Flags: `TestFlagCandidatesMatchesTheRealCobraCommand` (against the
      real clap `Command`), `TestFlagCandidatesSkipsHiddenFlags`.
- [x] Subcommands: `TestSubcommandCandidatesListsIdentityChildren`,
      `TestSubcommandCandidatesSkipsHiddenChildren`. **`identity` has no
      children until P5**: written against the registry's `vault` parent
      now, with the `identity` form added when P5 registers it; the
      substitution is recorded here when made.
- [x] Vaults and entries: `TestVaultNameCandidatesListsRegisteredVaults`,
      `TestEntryTitleCandidatesNoSessionIsNil`.
- [x] Positionals: `TestLeafPositionalCandidatesOffersNothingForACommandWithNoQuery`,
      `TestLeafPositionalCandidatesStopAfterTheFirstArgument`.

### 4. Composed completion and dispatch — 14 `complete` rows

- [x] `TestCompletingTopLevelListsSessionVisibleNames`,
      `TestCompletingPartialSessionCommandsMatchesExactly`,
      `TestCompletingABareDashAtTheTopLevelOffersRootFlags`,
      `TestCompletingFlagPositionListsCommandFlags`,
      `TestCompletingSubcommandListsIdentityChildren` (same `vault`
      substitution as task 3).
- [x] `TestCompletingUseArgumentListsVaultNames`,
      `TestCompletingLockArgumentListsVaultNames`,
      `TestCompletingUseFlagValueListsVaultNames`,
      `TestCompletingUseArgumentIsIndependentOfUnlockState`.
- [x] `TestCompletingAnEntryQueryListsCurrentVaultTitles`,
      `TestCompletingAnEntryQueryWhenLockedNeverUnlocks` (a prompter that
      panics if asked), `TestCompletingQueryAfterAUseFlagStillOffersEntryTitles`.
- [x] `TestRunSessionCommandDispatchesAnUnambiguousAbbreviation`,
      `TestRunSessionCommandRejectsAnAmbiguousAbbreviation`.

### 5. `rustyline` wiring and the real terminal — 1 `complete` row, `complete_pty` (1)

- [x] `TestSessionCompleterDoWiresCandidatesIntoReadlinesContract`: the
      `Completer::complete` adapter returns the right start position and
      candidates (one thin test; the logic is tested in tasks 2–4).
- [x] `complete_pty` `TestTabCompletionAtThePromptExpandsAPartialCommand`
      (Unix only, `portable-pty`): a literal Tab expands `stat` to
      `status`.
- [x] History is untouched by completion: a Tab-only interaction writes
      nothing to the history file (re-asserting M6's rule on this path).

### 6. User docs and ledger

- [x] Extend `doc/cli/session-mode.md` with a transcript showing Tab
      completion and abbreviation.
- [x] Parity ledger: §2 `0aa325c`; §3 "M5, M6, M7, M13" row; §6
      `addressing-entries.md`, `session-mode.md`; §5 P3 summary.
- [x] P3 index status column and the port plan's phase table.

## Outcome and deviations

Written as planned; the choices the plan left open, and the places the
code differs from Go:

- **`identity` substitution** (tasks 2–4), **undone in P5 · M9a**: the
  `identity` forms of `TestSubcommandCandidatesListsIdentityChildren`,
  `TestCompletingSubcommandListsIdentityChildren` and the `ident`
  abbreviation are now in `complete_tests.rs`, with `enroll` joining in
  P7. The `vault` versions stay as extra coverage. As written in P3,
  `identity` wasn't registered until P5, so the tests used the `vault` parent (`list`/`info`/`remove`/
  `set-default`) and the abbreviation `vau`. Go's `l` is ambiguous across
  `lock`, `log`, `ls`; here it is `lock`, `ls` (`log` is P4's). Both
  swap back when those commands are registered; `complete_tests.rs` says
  so in its module doc. `sync` in the no-query test is `reindex`/`status`
  for the same reason.
- **`mv`/`cp`** aren't registered, so they aren't in
  `ENTRY_QUERY_COMMANDS` and `--to-vault` isn't a vault-name flag; each
  joins when its command does.
- **Session sharing**: `Repl.session` is now `&RefCell<Session>`
  (`run_session` holds an `Rc<RefCell<…>>`), because the editor's
  completer reads the session while a line is typed. The loop borrows it
  mutably only while a command runs, and the completer uses
  `try_borrow_mut`. This is the one place the plan's "switch to Opus if
  awkward" note could have applied; it wasn't.
- **Library**: `Session::entry_title_candidates` returns a plain
  `Vec<String>` (empty on a failed index build) rather than Go's
  `([]string, error)`: completion has nowhere to report an error. It
  applies the idle timeout like any call, so an aged-out vault offers
  nothing.
- **Additions Go doesn't have**: a title containing whitespace is
  completed *quoted* (`"AWS root account"`), so it parses back as one
  argument; Go inserts it bare. `CompletionType::List` makes an
  ambiguous Tab fill the common prefix and list, like Go's readline. A
  partial word that itself opens a quote isn't completed (as Go).
- **Claimed rows**: 46 ported (4 library, 41 `complete_tests`, 1 pty);
  Go's `TestSubcommandCandidatesListsIdentityChildren` and
  `TestCompletingSubcommandListsIdentityChildren` are the `vault`
  versions. The pty test types `stat` where Go types `u` (Go's `use`
  needs an argument and would prompt). Rows that aren't Go's:
  `complete_tests` adds `vault_name_candidates_with_no_config_is_empty`,
  `completing_the_use_flags_other_spellings` (`-u`, `--use=`),
  `completing_after_an_unknown_command_offers_nothing`,
  `subcommand_candidates_skip_children_not_available_in_a_session` and
  `run_session_command_dispatches_an_abbreviation_through_clap`;
  `session_tests` adds `entry_title_candidates_empty_once_the_idle_timeout_relocks`.
- **Test review**: the completion tests' session is built like
  `run_session`'s (global config's current vault, as Go's
  `newSessionApp`), so the locked-vault completion test runs the locked
  path rather than the no-current-vault one; the leaf-positional tests
  run against an unlocked vault with titles, so a misrouted command has
  something to wrongly offer. Each was checked by mutating the code it
  guards.

## Definition of done

Every box above checked; all 46 claimed rows ported, with any
substitution named here; `make lint` and `make test` clean locally and
the unmodified suite on Linux, macOS and Windows CI. Completion and
abbreviation share one prefix primitive, never trigger an unlock, and
never offer another vault's titles. P3 is done.
