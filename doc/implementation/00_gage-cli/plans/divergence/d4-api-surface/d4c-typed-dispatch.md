# D4c — Typed CLI dispatch

[D4 plan](index.md) · [#143](https://github.com/distillerylabs/gage-rs/issues/143) · previous: [D4b](d4b-test-seams.md)

> **Recommended model: Sonnet.** This is a CLI-only refactor. Fifty-four
> help snapshots and the registry-consistency tests already pin its
> output. The thing to watch is that a registry entry and its handler
> can only disagree at compile time, never at run time.

## Goal

gage-cli dispatches on a typed command, not a string:

- Each registry entry carries a `Command` value.
- `run`, `flags` and `long_about` match it exhaustively, so a registered
  command with no handler fails to compile.
- The session verbs and `help` are typed too.
- The "not implemented yet" path, which nothing real can reach, is
  deleted.

Nothing user-visible changes. The help snapshots, the command reference
and the exit codes are unchanged.

## Depends on

D4b. D4c is independent of the library changes, but it stacks on D4b so
the branches stay linear.

## Design references

- [D4 index](index.md).
- [ADR-0030](../../../../../decisions/0030-help-render.md): clap renders
  per-command help, and the registry supplies grouping, aliases and
  availability.
- [ADR-0031](../../../../../decisions/0031-help-surfaces.md): one
  registry drives both help surfaces.
- [ADR-0032](../../../../../decisions/0032-command-availability.md):
  session-only and one-shot-only commands.
- CLAUDE.md, "Command registry".

## Where things are

**The registry** (`registry.rs`).

- `CommandInfo { name, aliases, short, group, usage, availability }`,
  all `&'static`. `name` is the space-joined path (`"vault list"`).
- `fn registry()` (:85-428) builds a `Vec` literal of 45 commands and 6
  parents.
- `find(name)` matches a name or an alias.
- There are three aliases: `whoami`, `quit`, `grep`.
- Five commands are `SessionOnly` (`use`, `lock`, `status`, `exit`,
  `help`). Five are `OneShotOnly` (`init`, `clone`, `recovery enroll`,
  `recovery rotate`, `recovery verify`). The other 35 are `Both`.

**The clap tree** is built from the registry by `cli::build`/`children`/
`leaf` (`cli.rs:63-196`). `cli::parse` + `resolve` return
`Outcome::Command(info.name, matches)`, where `name` is always canonical
because aliases are resolved by `reg.find` and by clap.

**The string-keyed sites:**

| Site | What it keys | Fallthrough |
|---|---|---|
| `commands/mod.rs:61` `flags(name)` | 12 arms → a module's `flags()` | `_ => Vec::new()` |
| `commands/mod.rs:84` `long_about(name)` | 19 arms | `_ => None` |
| `commands/mod.rs:136` `run(name, …) -> Option<Result>` | 40 arms | `_ => None`, which becomes "not implemented yet" |
| `repl.rs:366` `exec_line_in` | `Some("exit" \| "use" \| "lock" \| "status" \| "help")` | `run_command` |
| `lib.rs:157` | `Outcome::Command("help", …)` | the general arm |
| `cli.rs:180` | `info.name == "help"` gives a trailing-varargs arg | `usage_args` |
| `commands/{entry,log,identity,recipient,approve,recovery}.rs` `flags` | sub-matches on the full name | each has a `_ =>` default |
| `complete.rs:253` + `ENTRY_QUERY_COMMANDS` (:42) | the clap **leaf** name (`"show"`, not a path) | empty list |

**The "not implemented yet" path** has two copies:

- `lib.rs:188` (one-shot), with exit code `Internal`
- `repl.rs:427` (in-session)

No real command reaches either. `tests.rs:539`
`an_unimplemented_one_shot_command_is_internal` pushes a fake
`"fake-oneshot"` to reach the first. No test covers the second.

**Tests that inject `CommandInfo`s.** `tests.rs:56` `cmd()` builds
fakes for help grouping (`fake-entry-cmd`, `fake-vault-cmd`), alias
rendering, drift detection (`registry_completeness_detects_*`) and the
not-implemented test.

**What pins the output:**

- `help_snapshot_tests.rs`: 54 snapshots, plus a check that every
  registered command has one
- `registry_completeness` (the clap leaves equal the registry names)
- `command_reference_names_every_registered_command`
- the REPL help tests in `repl_tests.rs`
- the completion tests in `complete_tests.rs`

## Decisions to make first

1. **The enum's shape.** Proposed:

   ```rust
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum Command {
       Session(SessionVerb),   // use, lock, status, exit, help
       Ls, Show, Cat, Insert, Edit, Rm, Mv, Cp, Rename, Generate, Search,
       VaultList, VaultAdd, …, RecipientApprove, RecoveryEnroll, …
   }
   pub enum SessionVerb { Use, Lock, Status, Exit, Help }
   ```

   - `CommandInfo` gains `command: Command`, and keeps `name` as the
     display and parse path. clap and the help renderer still work by
     name.
   - `Outcome::Command` carries the `Command`, not the `&str`.
   - The enum is hand-written beside `registry()`, one variant per
     entry. The alternative is a `macro_rules!` that declares the
     entries and the enum together. That is fewer lines but harder to
     read, and the help snapshots already catch a missing entry.
   - A unit test asserts that every registry entry's `command` is
     distinct, and that every `Command` variant appears in exactly one
     entry. Use a hand-written `ALL` list of `Command`, checked against
     the registry.

2. **Where `help` lives.** `help` is session-only in the registry, but
   `gage help` (one-shot) is special-cased in `lib.rs:157`. Proposed:
   - `Command::Session(SessionVerb::Help)` is matched in both places.
   - `cli.rs:180`'s varargs special case keys on the same value.
   - Availability doesn't change.

3. **How tests inject fake commands.**
   - Most fakes only render help, so any variant works. Proposed: `cmd()`
     takes a `Command` and the help-only fakes pass an existing one,
     for example `Command::Ls`. Rendering never dispatches, so the
     variant is irrelevant.
   - The drift-detection tests compare names, so they're unaffected.
   - `an_unimplemented_one_shot_command_is_internal` is deleted,
     because the path it tests no longer exists. Exhaustiveness is now
     the compiler's job.
   - The alternative is a `#[cfg(test)] Command::Fake` variant. It puts
     a test-only arm into every production match, so no.

4. **Per-module `flags` sub-matches.** Proposed: each module's
   `flags(name)` becomes `flags(cmd: Command)`, matching its own
   variants exhaustively. Its `_ => use_flag` default becomes an
   explicit list, so adding a command to a module forces a decision
   about its flags.

5. **Completion.** `complete.rs:253` and `ENTRY_QUERY_COMMANDS` key on
   the clap leaf name. Proposed:
   - Resolve the leaf to its registry entry with `reg.find(full_path)`,
     then match on `Command`.
   - `ENTRY_QUERY_COMMANDS` becomes `Command::takes_entry_query(self) ->
     bool`, an exhaustive match. A new entry command then can't silently
     lack title completion.

6. **`commands::run`'s return.** It returns `Result<…>`, not
   `Option<Result<…>>`. Session verbs never reach it, because the REPL
   handles them first, and one-shot rejects session-only commands
   first. Proposed: `run` takes the non-session part of the command
   (for example `match cmd { Command::Session(_) => unreachable!(…) }`),
   or splits `Command` so `run` takes only a `RunCommand`. Prefer the
   split if the arms read cleanly. A type that rules the case out beats
   an `unreachable!`.

## Settled decisions

Decisions 1-6 were built as proposed, with these refinements
(recorded in [ADR-0057](../../../../../decisions/0057-typed-command-dispatch.md)):

- **Decisions 1 and 6.** The split is `Command::Session(SessionVerb)` /
  `Command::Run(RunCommand)`, so `commands::run` takes a `RunCommand` and
  needs no `unreachable!`. The one that remains is in the REPL, where
  `Outcome::Command` can still carry a session verb that `exec_line_in`
  rules out by dispatching verbs first. `Command::ALL` is hand-written and
  checked against the registry.
- **Decision 3.** The test helper `cmd()` fixes the variant to
  `RunCommand::Ls` rather than taking one; the fakes only render help.
- **Decision 4.** Finer than planned: modules expose one flags function per
  command that differs (`entry::show_flags`, ...), and `commands::flags`
  lists every `--use`-only command, so no module keeps a `_` default.
- **Decision 5.** `leaf_positional_candidates` keeps its signature (the
  `complete_tests` are unedited, `use` paths included). It resolves the
  clap leaf name through `registry().command_named`.

## Tasks

### 1. Pin what exists

- [x] Tests first: a registry test asserting that the set of
      `CommandInfo` names equals today's 45, so a rename in this
      milestone shows up as a test failure, not as a snapshot diff
      noticed late. Green on the current code.

### 2. `Command` in the registry

- [x] Implement decision 1. Add the `ALL`-versus-registry uniqueness
      test, which is red until every entry carries a distinct
      `command`.

### 3. Dispatch

- [x] Replace `commands::run`, `flags` and `long_about` with exhaustive
      matches on `Command` (decisions 4 and 6).
- [x] Replace the REPL's verb match and the two `"help"` special cases
      (decision 2).
- [x] Delete both "not implemented yet" paths and their test (decision
      3).

### 4. Completion

- [x] Implement decision 5. The `complete_tests` stay green unedited,
      except for `use` paths.

### 5. ADR and green

- [x] ADR-0057: the registry carries a typed `Command`; dispatch is
      exhaustive; tests inject fakes with real variants. Link it from
      `doc/decisions/README.md`. It amends nothing in ADR-0030–0032.
- [x] `make lint` and `make test` are clean.
- [x] `git diff --stat main -- tests/fixtures` is empty, so the help
      snapshots are unedited.
- [x] `grep -rn 'not implemented yet' crates/gage-cli/src` is empty.

## Definition of done

- Every box above is ticked, and decisions 1–6 are recorded as settled.
- No `match` on a command-name string remains in gage-cli. The only
  string lookups are `reg.find` (parsing what the user typed) and the
  clap tree build.
- Adding a registry entry without a handler, flags decision and
  completion decision fails to compile.
- `make lint`, `make test` and `make qa` are clean. The help snapshots,
  `command_reference_names_every_registered_command` and
  `registry_completeness` are green and unedited.
- CI is green on Linux, macOS and Windows.
