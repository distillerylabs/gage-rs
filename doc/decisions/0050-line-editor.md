# ADR-0050: The line editor and the history file

- Status: Accepted
- Origin: Q-LINE-EDITOR ([entry](../archive/rust-port/open-questions.md#q-line-editor))

## Context

The REPL needs history that records only chosen lines, a history file no
one else can read, a completer fed by live session data (vault names, entry
titles), and Windows support.

## Decision

`rustyline` 18.0.1, in `gage-cli` only, behind a `LineReader` trait. gage
owns the history file; the editor does not.

- History is never added automatically. The REPL adds each submitted,
  non-blank line itself and nothing else: no passphrase, no value.
- gage creates the history file, or tightens an existing one, to 0600 and
  appends to it. `rustyline` is built without default features, so its
  history is the in-memory recall list only, preloaded from the file.
  `FileHistory::save` rewrites the whole file, and a session must start
  even when the file cannot be read.
- `Completer::complete` returns candidates computed on demand.
- The REPL reads through `LineReader`: a plain implementation over
  `Io.stdin` for in-process tests and piped input, and a `rustyline` one
  when stdin is a terminal.
- `rustyline` puts the terminal in raw mode only for the duration of a
  `readline()` call, so the terminal prompter reads stdin directly between
  lines and passphrase prompts never go through the line editor.
- `reedline` was not taken: its painter and dependency tree are larger than
  line editing, history and completion need.

## Consequences

- On Windows `rustyline` depends on `clipboard-win` and `error-code`, both
  BSL-1.0, which are in `deny.toml`'s license allowlist. Neither reaches
  `crates/gage`.
