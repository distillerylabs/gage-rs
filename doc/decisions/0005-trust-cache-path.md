# ADR-0005: Trust cache location

- Status: Accepted
- Origin: Q-TRUST-CACHE-PATH ([entry](../archive/rust-port/open-questions.md#q-trust-cache-path)); A20 ([ADR-0006](0006-vault-id-keying.md))

## Decision

The trust cache lives in `$GAGE_STATE/<vault-id>/`: the verbatim
`known-config.toml` and the hash of `.age-recipients`, written with
`atomicfile` and never committed. `vault remove` deletes the files the
cache owns by name, then the directory if it is empty.

## Consequences

- A half-written pair reads as "no record" and is bootstrapped again, never
  trusted.
- Keyed by id, so two vaults registered under one local name in sequence do
  not share a cache. A stale cache fails safe: it produces a spurious
  recipient-change warning, never a suppressed real one.
