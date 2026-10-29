# ADR-0031: Two help surfaces, one registry

- Status: Accepted
- Origin: Q-HELP-SURFACES, A10 (original project's register)

## Context

`gage --help` must not list `use`, `lock`, `status` or `exit` (they do not
exist outside a session), while in-session `help` must list exactly those.
Two hand-maintained lists would drift as commands land.

## Decision

- `gage help` is supported and equivalent to `gage --help`, including
  `gage help <subcommand>`.
- In-session `help <command>` mirrors `gage help <subcommand>`.
- Both render from one command registry: name, aliases, short description,
  group (mirroring the command-reference sections), and availability.
  A test compares the rendered sets.
- `-u|--use` appears in per-command help, not in top-level session help,
  which leads with bare `use <vault>`.
- The registry lives in `gage-cli`, never the library: command names are
  CLI vocabulary, and a GUI or TUI calls library methods directly.

Aliases: `search`/`grep`, `status`/`whoami`, `exit`/`quit`.
