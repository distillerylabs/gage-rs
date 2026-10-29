# ADR-0028: `--recovery-key-out` never overwrites

- Status: Accepted
- Origin: Q-RECOVERY-NOCLOBBER ([entry](../archive/rust-port/open-questions.md#q-recovery-noclobber))

## Decision

`atomicfile::write_new` writes a temp file in the destination's directory,
fsyncs it, sets mode 0600, then calls tempfile's `persist_noclobber`, so an
existing destination is refused atomically. tempfile creates the temp file
0600 on Unix, so the secret is never readable by others before the mode is
set. On Windows the mode step is a no-op and the file inherits the
directory's ACL. (`atomicfile::write` always
replaces.) A check-then-write would leave a window in which another file can
appear at the destination.

The up-front checks stay: destination existence, parent-directory checks and
a writability probe. They run before anything is created, which is what lets
a refusal leave nothing behind. `write_new` is the backstop.
