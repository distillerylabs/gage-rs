# ADR-0029: The metadata index holds plaintext in one locked arena

- Status: Accepted
- Origin: Q-INDEX-ARENA ([entry](../archive/rust-port/open-questions.md#q-index-arena))

## Decision

Each vault's in-session metadata index packs titles and descriptions into
one `LockedBuf` (page-aligned, zeroized on drop); entries hold spans into
it. Growth allocates a larger buffer, locks it, copies the live spans and
drops the old one. Locking goes through the vault's `Arc<dyn Locker>`, the
same as its `Identity`, so the existing counting and failing lockers prove
the lock, warn-once and release-on-discard rules.

## Consequences

- Timestamps and `updated_by` (a device name, already public in the vault
  config) stay in ordinary memory.
- The index holds no `value` or `fields` and is never written to disk.
- The index lives on `Session`, never on `Vault`.
