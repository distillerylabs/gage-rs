# ADR-0035: Clipboard and its clear

- Status: Accepted
- Origin: Q-CLIPBOARD ([entry](../archive/rust-port/open-questions.md#q-clipboard)); A17 (original project's register)

## Decision

`arboard` in `gage-cli` only, with `default-features = false` and
`wayland-data-control`, behind a `ClipboardPort` trait. Plaintext is
written in-process on every platform and never passes through a child
process. One-shot Ctrl-C is caught with `ctrlc`, installed only on the
one-shot `-c` path just before the wait.

- The clear happens in the process that wrote the value. On Linux contents
  last only while a `Clipboard` lives, so a one-shot `-c` blocks holding it
  and a session holds it for its life.
- The production port opens the clipboard on first copy.
- Each copy carries the platform's history-exclusion hint
  (`exclude_from_history` on macOS and Linux, `exclude_from_monitoring` on
  Windows), so a clipboard-history manager does not keep the value after the
  clear.
- A generation number per copy: a scheduled clear does nothing under a later
  generation, so a timer waiting on the lock cannot wipe a second copy early.
- The timeout is read from global config when a copy happens, in both modes.
- `show -c -q` is refused before the unlock.
- The port, the timer, the wait and the failure report sink are seams in
  `gage-cli`. No test touches a real clipboard.

## Consequences

A SIGKILL leaves a copied secret on the clipboard; this is an accepted risk.
