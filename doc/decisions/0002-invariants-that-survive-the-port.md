# ADR-0002: Invariants kept from the port's compatibility decision

- Status: Accepted
- Origin: Q-COMPAT ([entry](../archive/rust-port/open-questions.md#q-compat)); see also [ADR-0001](0001-divergence-from-go-gage.md)

## Context

The port's compatibility decision was to match the earlier implementation's
behavior but not its bytes. That half is retired by ADR-0001. The rest of it
was never about interop: it recorded properties that come from design or
security.

## Decision

These stay, as gage-rs's own rules:

- the library/CLI split and the lint that enforces it
- process-level `vaultlock` tests
- validation of vault ids and device names as untrusted input
- local state keyed by vault id ([ADR-0006](0006-vault-id-keying.md))
- the exit-code taxonomy ([ADR-0037](0037-exit-code-taxonomy.md))
- `.age-recipients` readable by stock `age`; standard age files; the scrypt
  guard tests ([ADR-0024](0024-scrypt-hook.md))
- 3-OS CI, the coverage gate, and test-first milestones

gage-rs does not reproduce another implementation's bytes (TOML or YAML
style, help layout). Its own bytes are pinned by the format fixtures and the
help snapshots.

## Consequences

Changing any item above is a decision and needs an ADR.
