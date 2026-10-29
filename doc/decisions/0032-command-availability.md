# ADR-0032: Which commands work in a session

- Status: Accepted
- Origin: Q-CMD-AVAILABILITY, A11, A12 (original project's register)

## Decision

| Availability | Commands |
|---|---|
| session only | `use`, `lock`, `status`/`whoami`, `exit`/`quit`, `help` |
| both | everything else: entry commands, `reindex`, sync/pull/push, `log`, `history`, `vault`, `identity`, `recipient`, `git set-remote` |
| one-shot only | `init`, `clone` |

`init` and `clone` create a vault rather than operating on the current one,
which would leave open whether the new vault becomes the session's current
vault.

## Consequences

Widening availability later is additive; narrowing it would break users, so
the conservative side is the right one to start on.
