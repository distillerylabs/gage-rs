# ADR-0034: `GAGE_PASSPHRASE` in script mode

- Status: Accepted
- Origin: Q-SCRIPT-PASSPHRASE ([entry](../archive/rust-port/open-questions.md#q-script-passphrase)); Q-SCRIPT-CHOOSE ([entry](../archive/rust-port/open-questions.md#q-script-choose)); A18 (original project's register)

## Decision

`main` reads the variable once into an `Io` field (`Io.passphrase_env`), as
`editor` is. Nothing in `crates/gage` learns the variable exists, and
in-process tests share one environment so they set the field instead.

Four `gage-cli` wrapper prompters answer one thing each and delegate every
other method: `EnvPrompter`, `RefusingPrompter`, `NoCreatePrompter` and
`NoChoosePrompter`. The source order, the unlock-purpose-only rule and the
single attempt are the script-mode policy.

- Refusals use `PromptError::Refused(String)`, coded locked-or-auth (5). The
  message carries no `gage:` prefix, because `prefix_once` adds it.
- `NoChoosePrompter` answers `choose` with `Cancelled`, so an ambiguous
  query fails with its candidate list as one-shot does (exit 4) and reads
  nothing. It is installed under the same condition as the create refusal
  (no terminal, or `--stdin`); `--script FILE` on a terminal still lets the
  human pick. Without it, the prompter would take the next command line as
  the pick number.
- `confirm` and `value` are not wrapped: they read the stream, since
  `insert` taking its value from the next line is sometimes wanted.

## Consequences

Every wrapper is tested to pass through each method it does not answer.
