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

The page lock goes through the vault's `Locker`. If it fails (a restricted
`ulimit -l`, a locked-down container or Windows policy), the unlock warns
once and proceeds unprotected rather than refusing to work.

`Drop` order is zeroize, then unlock (only if the lock took), then
deallocate. Zeroing happens whatever the unlock does. `Debug` never prints
the key. `Identity` is not `Clone`. Use after release and shared release
are unrepresentable; they are proved by `compile_fail` doctests.

## Consequences

Recorded gap: `age`'s Bech32 decoding makes transient copies that are neither
locked nor wiped. `Identity` is always a parameter, never internal state of
a `Vault`.
