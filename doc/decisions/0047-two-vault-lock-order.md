# ADR-0047: Two vaults are locked in vault-id order

- Status: Accepted
- Origin: Q-TWO-VAULT-LOCK ([entry](../archive/rust-port/open-questions.md#q-two-vault-lock))

## Context

`mv` and `cp` between vaults write to both, so both per-vault advisory locks
([`vaultlock`](../design/gage-cli-design.md)) must be held across the whole
operation. Two processes moving entries in opposite directions deadlock
unless every caller takes the locks in one fixed order.

## Decision

Both locks are held for the whole operation and acquired in order of vault
id. The lock file is keyed by vault id ([ADR-0006](0006-vault-id-keying.md)),
so the id is what names a lock; ordering by the local vault name, which no
longer names it, would allow the opposite-order deadlock the fixed order
exists to prevent.

A source and destination with the same id are refused before any lock is
taken. Each `vaultlock` lock is an `flock` on its own open file description,
so a process that locks one vault twice waits out the full timeout and then
fails. Two registrations of one repository share an id, so they must be
refused up front.

## Consequences

- Any future operation that takes more than one vault lock uses the same
  order.
- A write path never takes a second lock while holding one out of order.
