# ADR-0054: The library's errors are frontend-neutral

- Status: Accepted
- Origin: divergence plan D3b ([#133](https://github.com/distillerylabs/gage-rs/issues/133))

## Context

`crates/gage` is meant to serve a second, in-process frontend (a GUI or
TUI), but its errors spoke as the command line:

- 48 messages opened with `gage:`, and the CLI stripped it again in seven
  places so it wasn't doubled
- 34 opened with their module's name (`config:`, `atomicfile:`)
- 18 told the reader to run a `gage …` command, and more named flags
- every error enum mapped itself to an exit number, which
  [ADR-0037](0037-exit-code-taxonomy.md) already said belongs in `gage-cli`
- warnings were strings that also opened with `gage:`

A second frontend would have had to parse all of that back out.

## Decision

A library error or warning says what went wrong, and nothing about who
is showing it.

- **No name in the text.** No `gage:` and no module prefix. A message
  reads as a sentence about its subject: `parsing <path>: …`, not
  `config: parsing <path>: …`.
- **A kind, not a number.** Errors implement `gage::error::HasKind`, which
  gives an `ErrorKind` (`Conflict`, `Usage`, `NotFound`, `Ambiguous`,
  `LockedOrAuth`, `Internal`, `Unreachable`). `gage-cli`'s `exitcode` maps a
  kind to its number; the numbers stay as ADR-0037 fixes them.
- **A remedy, not a command.** What would fix an error is a typed
  `gage::error::Remedy` (`Sync`, `AuthLogin { host }`, `RecipientRepair`,
  …), returned by `HasKind::remedy()`. A wrapper error passes its inner
  error's remedy through, unless it has its own. `gage-cli` renders a
  remedy as the clause it appends: `; run `gage sync``. A remedy always
  comes last.
- **Warnings follow the same rules.** `Prompter::warn` takes a
  `Warning { text, remedy }`. The terminal prompter writes
  `gage: {text}{remedy}`.
- **`gage:` exactly once, in the CLI.** The one place an error reaches
  stderr adds the prefix. A multi-line message the CLI writes opens its
  later lines with `gage:` itself.
- **Commit messages are format, not UI.** `gage: initialize vault`,
  `gage: recipient add …` and the other commit subjects are fixed by
  [v2](../format/v2.md) and stay.

Advice that names no command stays prose in the library, such as "check
that the repository exists at that path". So does a flag that is the
subject of a refusal's reason: "removing a recipient requires
--reencrypt: without it …".

`crates/gage/tests/no_cli_text.rs` enforces the text rules. It fails on a
`"gage: ` literal that isn't a commit subject, and on a string literal
naming a `` `gage `` command.

## Consequences

- stderr changed only where a module prefix was dropped, and where a
  remedy that used to sit mid-sentence now comes last. The D3b plan lists
  each of those; the exit codes and help output are unchanged.
- A second frontend matches `ErrorKind` and `Remedy`, and can offer a
  remedy as an action instead of a shell command.
- A new error that wraps another must delegate `remedy()` as it delegates
  `kind()`. Forgetting drops the hint silently, so tests pin remedies as
  values, not as text.
