# M12d — `--script` / `--stdin`, `GAGE_PASSPHRASE`, docs

[← M12c](m12c-clip-qr.md) · [P6 plan](index.md) · [#56](https://github.com/distillerylabs/gage-rs/issues/56) · next: P7 Enrollment

> **Recommended model: Sonnet ⚑.** Go settled every decision, and the
> wrappers are shaped like `AssumeYes`. Run an **Opus review of the
> passphrase-source rows** (task 3) before calling it done. The failure
> they guard against is quiet: under `--stdin`, a passphrase prompt reads
> the next command line, and for a new identity that line becomes the
> passphrase protecting a device key nobody can ever type again.

## Goal

The design's "Non-interactive session mode": one `Session` driven by a
list of commands instead of a human at a prompt. It holds keys the same
way and drops them at the same point.

- **`gage --script FILE`** reads command lines from a file.
  **`gage --stdin`** reads them from stdin. They are mutually exclusive,
  refused alongside a subcommand, refused inside a session, and a missing
  script file is a Usage error.
- **Stop at the first failing line.** The error names the source, the
  line number and the line, and keeps that line's exit code. `exit` ends
  the run early with success. Blank lines and `#` comments are skipped.
- **Each vault is unlocked at most once** across the run.
- **Nothing is written to the history file.** No history file is opened.
- **Where an unlock's passphrase comes from**, in Go's fixed order:
  1. `GAGE_PASSPHRASE`, if set
  2. otherwise a prompt, but only on `--script` with a terminal on stdin
     (`--stdin` has spent stdin on the command stream)
  3. otherwise an immediate refusal, exit 5 (`LockedOrAuth`), naming the
     variable. Never a read that can't be answered, and never an EOF
     reported as a wrong passphrase.
- **`GAGE_PASSPHRASE` answers once.** A wrong value fails after one
  attempt, not three.
- **`GAGE_PASSPHRASE` never answers a new identity's passphrase** (`init`,
  `identity add`). That request goes to a human when there is one, and
  is refused with a message naming the reason when there isn't, without
  consuming the command stream.
- **`--yes` is accepted in a script session**, the CI case it exists for.
  It is still refused in an interactive session (amends
  [Q-RECIPIENT-PROMPT](../open-questions.md#q-recipient-prompt)).
- **Docs:** the user pages describe everything P6 shipped.

## Depends on

- **P3 · M6:** `run_session`, `exec_line`, `split_line`, the `History`
  file, `Session` and its key bookkeeping.
- **P5 · M10:** `--yes`, `AssumeYes`, `assumeyes::refused_in_session`.
- **P2 · M2:** `UnlockRequest.purpose` (`Purpose::Create`) and
  `.attempt`.
- **M12c:** `-c` in a session schedules, and session exit clears. A
  script session goes through the same exit path.

## Design references

- Design doc, "Non-interactive session mode".
- Design doc, "Command history must never contain plaintext".

## Go sources

- Go plan `gage-cli-design/m12-polish.md`: the `--script`/`--stdin` and
  unlock decision (source order, decorator, unlock purpose only, no retry,
  no per-vault variable, no `/dev/tty` fallback).
- Parity ledger: §3 M12 (`--script`, `--stdin`); §2 `db2731f` (#27, the
  `--stdin` half); §6 `session-mode.md`.
- Go sources at the pinned SHA:
  - `cmd/gage/script.go`: all of it
  - `cmd/gage/assumeyes.go`: `withAssumeYes` and the conflict-shape
    preservation
  - `cmd/gage/app.go`: the root flags and the `--yes` exception
    (`scriptMode`)
- Test inventory, 26 rows: `cmd/gage/script_test.go`.
- Inbound, 1 row from P3: the `--stdin` half of `repl`
  `TestSessionCommandArgumentCountsAreChecked`.

## Decisions to make first

[Q-SCRIPT-PASSPHRASE](../open-questions.md#q-script-passphrase) settles
the shape. What was left to pin down in code, **resolved while
implementing** (the choices are below each item):

- **`Io.passphrase_env: Option<Zeroizing<String>>`.** `main` reads
  `GAGE_PASSPHRASE` once at startup (as `Some` even when empty: set-but-
  empty is still "set", as Go's `LookupEnv`). Tests set the field. Nothing
  in `crates/gage` learns the variable exists.
- **Three wrappers in `gage-cli`**, each holding `&dyn Prompter`:
  - `EnvPrompter`: answers a non-create unlock on attempt 1 from the
    variable; refuses attempt 2+ with `LockedOrAuth`; passes `Create` to
    its `create` prompter.
  - `RefusingPrompter`: refuses every non-create unlock with
    `LockedOrAuth`, naming the variable; passes `Create` on.
  - `NoCreatePrompter`: refuses `Create` with a message naming why;
    passes everything else on.

  Every other trait method delegates. Check how `requestPassphrase`'s
  normalisation to `LockedOrAuth` maps onto gage-rs's unlock errors, so
  the exit code is 5 on every refusal path.

  **Resolved.** `UnlockError::Prompter(_)` and `WrongPassphrase(_)` are
  both `LockedOrAuth` whatever they wrap, which is Go's normalisation, so
  an unlock refusal exits 5 on every path. The refusals' own wording
  needed a new library variant, `PromptError::Refused(String)` (also
  `LockedOrAuth`); see Q-SCRIPT-PASSPHRASE's addendum. A fourth wrapper,
  `NoChoosePrompter`, closes the same hole for the candidate picker
  ([Q-SCRIPT-CHOOSE](../open-questions.md#q-script-choose)). A
  `Passthrough` wrapper stands where a create request goes when a human
  can be asked. `for_script` picks the wrappers; `lib.rs` composes
  `--yes` innermost, then `NoChoosePrompter` (no human only), then the
  script layer.
- **One session runner, two drivers.** Factor out of `run_session` what
  both modes share (dirs, `[shell]`, `Session` construction, the clipboard
  keeper, the exit path that drops keys and clears the clipboard). The
  REPL and the script runner each drive it. The script runner opens no
  `History` and uses no `LineReader`: it reads lines itself. Keep the
  REPL's tests green unchanged; that is the check the factoring didn't
  change interactive behavior.

  **Resolved.** `repl::open` builds the dirs, `[shell]` settings and
  `Session`; `repl::finish` is the exit path (keys dropped, clipboard
  cleared, a failed clear reported). `run_session` and `run_script` each
  call them. The REPL's tests passed unchanged. The script runner reads
  lines through `read_one_line`, a byte at a time and never ahead, so
  whatever follows a line belongs to whoever reads next.
- **Root flags.** `--script <FILE>` and `--stdin`, global and hidden, like
  `--yes`. `cli::parse` returns a new outcome for script mode; the checks
  (both given, a subcommand given, inside a session) run before any
  session opens. A session line's `--script` is refused the way its
  `--yes` is.
- **The `--yes` exception.** In script mode, `--yes` wraps the script's
  prompter for the run. `refused_in_session` still applies to an
  interactive session and to a `--yes` typed on a script line (the run's
  own `--yes` already covers it; check Go's behavior for a per-line
  `--yes` in a script session and port it).

  **Resolved.** Go accepts a per-line `--yes` in a script session (its
  `PersistentPreRunE` wraps `app.Prompter` when `scriptMode`), but the
  session's prompter was fixed at the start, so the line's `--yes` changes
  nothing there. Ported as: accepted, adds nothing beside the run's own
  (`a_yes_on_a_script_line_is_accepted`). Recorded in
  [Q-RECIPIENT-PROMPT](../open-questions.md#q-recipient-prompt).
- **Ambiguous matches.** Go calls `reportAmbiguous` before returning a
  failed line's error. In gage-rs, check what a session line does with an
  ambiguous query when the prompter can't choose, and make the script
  error carry the candidate list the same way.

  **Resolved, and it was a real hole.** The session asks the prompter's
  `choose`, which under `--stdin` reads the command stream (up to five
  lines swallowed before it gives up). With nobody to ask, `choose` is now
  declined (`Cancelled`), which the session already turns into an
  `Ambiguous` error whose message lists the candidates. No separate
  reporting step is needed: a session line's error already carries the
  list. See [Q-SCRIPT-CHOOSE](../open-questions.md#q-script-choose);
  `confirm` and `value` still read the stream, as in Go.

## Tasks

Write each task's tests first, at task granularity.

### 1. The wrappers — 5 rows, 1 excused

- [x] `EnvPrompter`, `RefusingPrompter`, `NoCreatePrompter` (and `Passthrough`, `NoChoosePrompter`), in `crates/gage-cli/src/scriptprompt.rs`.
- [x] Rows (unit tests in `scriptprompt.rs`):
  - `TestEnvPrompterAnswersOnceThenRefuses`: `env_prompter_answers_once_then_refuses`
  - `TestNoCreatePrompterPassesOrdinaryUnlocksThrough`: `no_create_prompter_passes_ordinary_unlocks_through`
  - `TestAssumeYesAnswersTheTrustQuestionAndNothingElse`: over the new
    wrappers too, never `confirm` or `resolve_conflict`:
    `assume_yes_answers_the_trust_question_and_nothing_else_over_the_wrappers`
    (both nesting orders).
  - `TestAssumeYesOverANonConflictPrompterStaysNonConflict`: `AssumeYes`
    over a prompter whose `resolve_conflict` refuses still refuses:
    `assume_yes_over_a_prompter_that_refuses_conflicts_still_refuses`.
  - Excused: `TestTerminalPrompterOfWalksDecoratorChains` (see the
    [P6 plan](index.md#inventory-accounting)). In its place: every
    wrapper passes each method it doesn't answer through unchanged:
    `every_wrapper_passes_through_each_method_it_does_not_answer`.

### 2. `--script` / `--stdin` — 12 rows + 1 inbound

- [x] The flags (`cli.rs`), their checks, the shared session runner
      (`repl::open`/`finish`), the script loop and its error wrapping
      (`repl::run_script`). Rows, in `script_tests.rs`:
  - `TestScriptRunsCommandsNonInteractively`: `script_runs_commands_non_interactively`
  - `TestStdinRunsCommandsNonInteractively`: `stdin_runs_commands_non_interactively`
  - `TestScriptUnlocksEachVaultAtMostOnce`: `script_unlocks_each_vault_at_most_once`
  - `TestScriptAbortsOnTheFirstFailingCommand`: later lines don't run;
    the exit code is the failing command's; the message names
    `source:line: text`: `script_aborts_on_the_first_failing_command`.
  - `TestScriptWritesNothingToTheHistoryFile`: `script_writes_nothing_to_the_history_file`
  - `TestScriptSkipsBlankLinesAndComments`: `script_skips_blank_lines_and_comments`
  - `TestScriptExitEndsTheRunEarly`: `script_exit_ends_the_run_early`
  - `TestScriptAndStdinAreMutuallyExclusive`: `script_and_stdin_are_mutually_exclusive`
  - `TestScriptFlagsRejectASubcommand`: `script_flags_reject_a_subcommand`
  - `TestScriptFlagsAreRefusedInsideASession`: `script_flags_are_refused_inside_a_session`
  - `TestMissingScriptFileIsAUsageError`: `a_missing_script_file_is_a_usage_error`
  - `TestScriptAcceptsAssumeYes`: `script_accepts_assume_yes`
- [x] Inbound: the `--stdin` half of `repl`
      `TestSessionCommandArgumentCountsAreChecked` (a wrong argument
      count prints that command's usage and fails the run):
      `stdin_session_command_argument_counts_are_checked`, and the
      usage-block half `stdin_wrong_arg_count_shows_command_usage`.
- [x] Beyond Go's rows: a `-c` in a script session is cleared when the
      run ends (M12c's exit path): `a_script_ends_by_clearing_a_pending_copy`
      (finished and failed runs). Also `script_quit_is_exit`,
      `script_runs_the_session_verbs_and_stops_on_a_usage_error`,
      `a_failed_stdin_line_names_the_stream_and_keeps_an_ambiguous_list`,
      `a_script_does_not_leave_a_key_for_the_next_run`,
      `script_assume_yes_answers_the_recipient_change_and_without_it_the_write_is_refused`,
      `a_yes_on_a_script_line_is_accepted`,
      `a_one_shot_command_ignores_gage_passphrase`,
      `an_empty_gage_passphrase_is_set_and_is_refused_as_empty`,
      `an_ambiguous_query_under_stdin_does_not_eat_the_command_stream`,
      `script_on_a_terminal_still_lets_a_human_pick` and the hidden-flag
      parity check `the_script_flags_are_hidden_root_flags`.

### 3. Where the passphrase comes from — 9 rows ⚑

- [x] Wire the source order: `GAGE_PASSPHRASE`, else a terminal on
      `--script`, else refuse. `Create` goes to a human or to
      `NoCreatePrompter` (`scriptprompt::for_script`, composed in
      `lib.rs`; `main` reads the variable into `Io.passphrase_env`).
- [x] Rows (`script_tests.rs`):
  - `TestGagePassphraseUnlocksWithoutPromptingOrReadingTheCommandStream` ⚑: `gage_passphrase_unlocks_without_prompting_or_reading_the_command_stream`
  - `TestWrongGagePassphraseFailsAfterOneAttempt` ⚑: `wrong_gage_passphrase_fails_after_one_attempt`
  - `TestGagePassphraseIsIgnoredForANewIdentity` ⚑: `gage_passphrase_is_ignored_for_a_new_identity` (end to end) and `env_prompter_never_answers_a_new_identity` (unit`
  - `TestScriptWithNoPassphraseSourceFailsFast` ⚑: exit 5, names the
    variable, no read: `script_with_no_passphrase_source_fails_fast`.
  - `TestScriptOnATerminalStillPrompts`: `script_on_a_terminal_still_prompts`
  - `TestStdinNeverPromptsEvenOnATerminal` ⚑: `stdin_never_prompts_even_on_a_terminal`
  - `TestStdinRefusesToCreateAnIdentityWithNoHuman` ⚑: `stdin_refuses_to_create_an_identity_with_no_human`
  - `TestStdinIdentityAddDoesNotEatTheCommandStream` ⚑: the lines after
    `identity add` run as commands: `stdin_identity_add_does_not_eat_the_command_stream`
    (asserts the next line is still unread in the stream).
  - `TestScriptOnATerminalStillCreatesIdentitiesInteractively`: `script_on_a_terminal_still_creates_identities_interactively`

(Rows: 5 + 12 + 9 = 26, one of them excused, plus 1 inbound.)

### 4. Opus review

- [x] **Review of task 3's marked rows, by mutation.** Not the Opus
      review the header asks for (this milestone was implemented in a
      Sonnet session), so each named mutation was instead applied to the
      code, the library and CLI suites run, and the tests that failed
      recorded. A mutation nothing killed would have been a missing test;
      none survived. **An Opus read of the ⚑ rows is still recommended
      before M12d is marked done.**

| Mutation | Killed by |
|---|---|
| `EnvPrompter` answering `Create` | `script_on_a_terminal_still_creates_identities_interactively`, `stdin_identity_add_does_not_eat_the_command_stream`, `stdin_refuses_to_create_an_identity_with_no_human`, `env_prompter_never_answers_a_new_identity`, `for_script_sends_a_new_identity_to_a_human_or_refuses_it` |
| `EnvPrompter` answering attempt 2 | `wrong_gage_passphrase_fails_after_one_attempt`, `env_prompter_answers_once_then_refuses`, `a_refusal_never_prints_the_passphrase` |
| `--stdin` falling through to the terminal prompter when stdin is a terminal | `stdin_never_prompts_even_on_a_terminal` |
| `NoCreatePrompter` not installed under `--stdin` | `stdin_identity_add_does_not_eat_the_command_stream`, `stdin_refuses_to_create_an_identity_with_no_human`, `for_script_sends_a_new_identity_to_a_human_or_refuses_it` |
| the refusal returning a code other than 5 | `prompt_errors_carry_their_exit_codes` only |
| `NoChoosePrompter` not installed (added) | `an_ambiguous_query_under_stdin_does_not_eat_the_command_stream`, `a_failed_stdin_line_names_the_stream_and_keeps_an_ambiguous_list` |
| no `RefusingPrompter` when there is no variable and no terminal (added) | `script_with_no_passphrase_source_fails_fast`, `stdin_never_prompts_even_on_a_terminal`, `for_script_picks_the_source_in_go_s_order`, `for_script_sends_a_new_identity_to_a_human_or_refuses_it` |

The "wrong code" row is killed only at the unit, and that is the honest
shape of it: an unlock failure exits 5 whatever the prompter's own code
(`UnlockError::Prompter` is `LockedOrAuth`), so no CLI-level test can
distinguish them. The CLI tests assert exit 5 anyway, for the user-visible
contract.

### 5. Docs

Docs describe shipped behavior, so they go last, after every P6 surface
is green.

- [x] `doc/cli/entries.md`: `cat`'s display lines, `ls -H`, `-c`/`-q`,
      `log` and `history --decrypt`.
- [x] `doc/cli/session-mode.md`: `--script`/`--stdin`, `GAGE_PASSPHRASE`
      and its limits, `-c` in a session, `--yes` in a script.
- [x] `doc/cli/configuration.md` (the `GAGE_PASSPHRASE` row; `clipboard_timeout` is in session-mode's `[shell]` list, where the other `[shell]` settings are): `[shell] clipboard_timeout`.
- [x] `doc/cli/vaults.md`: `vault remove`'s identity-file cleanup.
- [x] `doc/cli/identities-and-recipients.md`: `--yes` in a script session.
- [x] `doc/cli/README.md` and `exit-codes.md` where P6 adds anything
      (exit 5 with no passphrase source).
- [x] Each page is checked against Go's `doc/cli/` page for the same
      surface, for anything P6 shipped that Go documents and the page
      doesn't.

### 6. Parity and ledger

- [x] Fixture parity: `--script`, `--stdin` and `--yes` are hidden root
      flags; `root.txt` lists none (`the_script_flags_are_hidden_root_flags`).
- [x] Parity ledger: §3 M12, §2 `db2731f` (the `--stdin` half) and §6
      updated as `[~]`; they become `[x]` with CI.
- [x] Q-RECIPIENT-PROMPT's "no script-mode exception yet" note updated to
      say it shipped here.
- [ ] Mark M12d and P6 `[x]` in the [P6 index](index.md) and the
      [port plan](../index.md) after CI.

## Definition of done

Every box above is checked. All 26 claimed rows are ported (one excused
in writing, with its replacement test), the inbound row is ported, the
review table is recorded and the docs are written. `make lint` and
`make test` are clean locally and on Linux, macOS and Windows CI.

The milestone is done when:
- `cat cmds.txt | GAGE_PASSPHRASE=… gage --stdin` runs every line against
  one unlock and stops at the first failure
- no path through script mode reads a command line as a passphrase
