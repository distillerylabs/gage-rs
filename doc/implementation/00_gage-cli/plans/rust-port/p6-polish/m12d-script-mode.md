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
the shape. What's left to pin down in code:

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
- **One session runner, two drivers.** Factor out of `run_session` what
  both modes share (dirs, `[shell]`, `Session` construction, the clipboard
  keeper, the exit path that drops keys and clears the clipboard). The
  REPL and the script runner each drive it. The script runner opens no
  `History` and uses no `LineReader`: it reads lines itself. Keep the
  REPL's tests green unchanged; that is the check the factoring didn't
  change interactive behavior.
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
- **Ambiguous matches.** Go calls `reportAmbiguous` before returning a
  failed line's error. In gage-rs, check what a session line does with an
  ambiguous query when the prompter can't choose, and make the script
  error carry the candidate list the same way.

## Tasks

Write each task's tests first, at task granularity.

### 1. The wrappers — 5 rows, 1 excused

- [ ] `EnvPrompter`, `RefusingPrompter`, `NoCreatePrompter`.
- [ ] Rows:
  - `TestEnvPrompterAnswersOnceThenRefuses`
  - `TestNoCreatePrompterPassesOrdinaryUnlocksThrough`
  - `TestAssumeYesAnswersTheTrustQuestionAndNothingElse`: over the new
    wrappers too, never `confirm` or `resolve_conflict`.
  - `TestAssumeYesOverANonConflictPrompterStaysNonConflict`: `AssumeYes`
    over a prompter whose `resolve_conflict` refuses still refuses.
  - Excused: `TestTerminalPrompterOfWalksDecoratorChains` (see the
    [P6 plan](index.md#inventory-accounting)). In its place: every
    wrapper passes each method it doesn't answer through unchanged.

### 2. `--script` / `--stdin` — 12 rows + 1 inbound

- [ ] The flags, their checks, the shared session runner, the script
      loop and its error wrapping.
- [ ] Rows:
  - `TestScriptRunsCommandsNonInteractively`
  - `TestStdinRunsCommandsNonInteractively`
  - `TestScriptUnlocksEachVaultAtMostOnce`
  - `TestScriptAbortsOnTheFirstFailingCommand`: later lines don't run;
    the exit code is the failing command's; the message names
    `source:line: text`.
  - `TestScriptWritesNothingToTheHistoryFile`
  - `TestScriptSkipsBlankLinesAndComments`
  - `TestScriptExitEndsTheRunEarly`
  - `TestScriptAndStdinAreMutuallyExclusive`
  - `TestScriptFlagsRejectASubcommand`
  - `TestScriptFlagsAreRefusedInsideASession`
  - `TestMissingScriptFileIsAUsageError`
  - `TestScriptAcceptsAssumeYes`
- [ ] Inbound: the `--stdin` half of `repl`
      `TestSessionCommandArgumentCountsAreChecked` (a wrong argument
      count prints that command's usage and fails the run).
- [ ] Beyond Go's rows: a `-c` in a script session is cleared when the
      run ends (M12c's exit path).

### 3. Where the passphrase comes from — 9 rows ⚑

- [ ] Wire the source order: `GAGE_PASSPHRASE`, else a terminal on
      `--script`, else refuse. `Create` goes to a human or to
      `NoCreatePrompter`.
- [ ] Rows:
  - `TestGagePassphraseUnlocksWithoutPromptingOrReadingTheCommandStream` ⚑
  - `TestWrongGagePassphraseFailsAfterOneAttempt` ⚑
  - `TestGagePassphraseIsIgnoredForANewIdentity` ⚑
  - `TestScriptWithNoPassphraseSourceFailsFast` ⚑: exit 5, names the
    variable, no read.
  - `TestScriptOnATerminalStillPrompts`
  - `TestStdinNeverPromptsEvenOnATerminal` ⚑
  - `TestStdinRefusesToCreateAnIdentityWithNoHuman` ⚑
  - `TestStdinIdentityAddDoesNotEatTheCommandStream` ⚑: the lines after
    `identity add` run as commands.
  - `TestScriptOnATerminalStillCreatesIdentitiesInteractively`

(Rows: 5 + 12 + 9 = 26, one of them excused, plus 1 inbound.)

### 4. Opus review

- [ ] **Opus review (⚑)** of task 3's marked rows. For each, name the
      mutation it kills and record the table here:
  - `EnvPrompter` answering `Create`
  - `EnvPrompter` answering attempt 2
  - `--stdin` falling through to the terminal prompter when stdin is a
    terminal
  - `NoCreatePrompter` not installed under `--stdin`
  - the refusal returning a code other than 5

### 5. Docs

Docs describe shipped behavior, so they go last, after every P6 surface
is green.

- [ ] `doc/cli/entries.md`: `cat`'s display lines, `ls -H`, `-c`/`-q`,
      `log` and `history --decrypt`.
- [ ] `doc/cli/session-mode.md`: `--script`/`--stdin`, `GAGE_PASSPHRASE`
      and its limits, `-c` in a session, `--yes` in a script.
- [ ] `doc/cli/configuration.md`: `[shell] clipboard_timeout`.
- [ ] `doc/cli/vaults.md`: `vault remove`'s identity-file cleanup.
- [ ] `doc/cli/identities-and-recipients.md`: `--yes` in a script session.
- [ ] `doc/cli/README.md` and `exit-codes.md` where P6 adds anything
      (exit 5 with no passphrase source).
- [ ] Each page is checked against Go's `doc/cli/` page for the same
      surface, for anything P6 shipped that Go documents and the page
      doesn't.

### 6. Parity and ledger

- [ ] Fixture parity: `--script`, `--stdin` and `--yes` are hidden root
      flags; `root.txt` lists none.
- [ ] Parity ledger: §3 M12 closed; §2 `db2731f` (the `--stdin` half);
      §6 `entries.md`, `session-mode.md` and the other pages above.
- [ ] Q-RECIPIENT-PROMPT's "no script-mode exception yet" note updated to
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
