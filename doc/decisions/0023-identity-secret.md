# ADR-0023: `Identity` holds its key in locked memory

- Status: Accepted
- Origin: Q-IDENTITY-SECRET ([entry](../archive/rust-port/open-questions.md#q-identity-secret))

## Decision

An `Identity` keeps the `AGE-SECRET-KEY-1…` line in a page-aligned,
page-locked `memlock::LockedBuf`. The X25519 key is parsed for each decrypt
and dropped straight after (microseconds against the Diffie-Hellman it
precedes). It is not kept as a parsed `age::x25519::Identity`, which
zeroizes itself but lives wherever its owner puts it, and a global-allocator
`Box` shares pages.

`Drop` order is zeroize, then unlock (only if the lock took), then
deallocate. `Identity` is not `Clone`. Use after release and shared release
are unrepresentable; they are proved by `compile_fail` doctests.

## Consequences

Recorded gap: `age`'s Bech32 decoding makes transient copies that are neither
locked nor wiped. `Identity` is always a parameter, never internal state of
a `Vault`.
