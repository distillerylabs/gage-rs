# ADR-0030: clap renders help; the registry is the source

- Status: Accepted
- Origin: Q-HELP-RENDER ([entry](../archive/rust-port/open-questions.md#q-help-render))

## Decision

clap renders one-shot help and usage errors. Grouping, aliases, descriptions
and one-shot/session availability come from the command registry in
`gage-cli` ([ADR-0031](0031-help-surfaces.md)); the root listing is grouped
in registry order (via `help_heading` or a help template).

The rendered output is gage-rs's own and is pinned by the snapshots in
`tests/fixtures/help/`. Tests also assert structure: grouping, aliases shown
with the canonical name, the one-shot set equal to the registry, and usage
errors showing the command's own usage and exiting with the usage code.

## Consequences

Adding a command means registering it in the registry, not just adding a
clap subcommand. A change to rendered help is a deliberate snapshot update.
