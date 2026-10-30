# ADR-0057: The registry carries a typed `Command`; dispatch is exhaustive

- Status: Accepted
- Origin: divergence plan D4c ([#143](https://github.com/distillerylabs/gage-rs/issues/143))

## Context

`gage-cli` dispatched on the registry's name strings: `commands::run`,
`flags` and `long_about` each matched on `&str` with a `_ =>` fallthrough,
and the REPL, `lib.rs`, `cli.rs`, six modules' `flags` and completion
repeated the pattern. A command registered without a handler compiled, and
ran as "not implemented yet" (two copies of that path, neither reachable by
any real command). A command added to a module silently took its `_`
default for flags and for completion.

## Decision

**A `Command` value per registry entry.** `CommandInfo` gains
`command: Command`, with `Command::Session(SessionVerb)` for the five verbs
only a session runs (`use`, `lock`, `status`, `exit`, `help`) and
`Command::Run(RunCommand)` for the other forty. `name` stays the parse and
display path; clap and the help renderer still work by name.
`cli::Outcome::Command` carries the `Command`, not a name.

**Exhaustive matches.** `commands::run` takes a `RunCommand` and returns
`Result`, not `Option<Result>`; `commands::flags` and `long_about` match
every variant with no `_` arm; `Command::takes_entry_query` replaces the
completion name list. The REPL matches `SessionVerb` exhaustively. A
registered command with no handler, no flags decision or no completion
decision does not compile. The "not implemented yet" paths are deleted.

**Flags are per command, not per module.** A module exposes one function
per command whose flags differ (`entry::show_flags`, `recipient::add_flags`,
...). Commands that take only `--use` are listed in `commands::flags`, not
defaulted by their module.

**Tests inject fakes with real variants.** Help-grouping and drift tests
register fake `CommandInfo`s carrying an existing variant; rendering never
dispatches. There is no test-only `Command` variant, which would put a
test arm into every production match. `Command::ALL` is checked against the
registry: each variant is registered exactly once.

## Consequences

- Name strings are looked up in two places only: `Registry::find` /
  `command_named` (what the user typed) and the clap tree build.
- Completion keeps the clap leaf name as its key and resolves it through
  the registry; nested leaves share no name with a command that takes a
  query.
- The REPL still has one `unreachable!`: a session verb reaching the clap
  tree, which `exec_line_in` rules out by dispatching verbs first.
- Amends nothing in ADR-0030 to ADR-0032.
