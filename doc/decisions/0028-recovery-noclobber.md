# ADR-0028: `--recovery-key-out` never overwrites

- Status: Accepted
- Origin: Q-RECOVERY-NOCLOBBER ([entry](../archive/rust-port/open-questions.md#q-recovery-noclobber))

## Decision

`atomicfile::write_new` writes a temp file in the destination's directory,
fsyncs it, sets mode 0600, then calls tempfile's `persist_noclobber`, so an
existing destination is refused atomically. (`atomicfile::write` always
replaces.) A check-then-write would leave a window in which another file can
appear at the destination.

The up-front checks stay: destination existence, parent-directory checks and
a writability probe. They run before anything is created, which is what lets
a refusal leave nothing behind. `write_new` is the backstop.
