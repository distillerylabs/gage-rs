# ADR-0051: What `--recovery-key-out` refuses

- Status: Accepted
- Origin: Q-RECOVERY-DATA-DIR ([entry](../archive/rust-port/open-questions.md#q-recovery-data-dir)), Q-RECOVERY-EMPTY-OUT ([entry](../archive/rust-port/open-questions.md#q-recovery-empty-out))

## Context

`gage init --recovery-key-out <file>` writes the recovery key to a file.
gage deletes from, or rewrites, some places, and a key written into one of
them would be destroyed or committed.

## Decision

The destination is refused, with `Usage`, when it is:

- **the vault being created;**
- **under any ancestor directory holding `.gage/config.toml`**, registered
  or not: the dirty-tree reset (`gitrepo::reset_hard`) deletes untracked
  files, and `commit_all` would otherwise commit the key;
- **anything under the data directory**, which `crates/reset-local-state`
  wipes;
- **empty.** `--recovery-key-out ""` does not read as "not given": a user
  who typed the flag asked to keep the key off the screen. Without the
  check the empty path named the working directory, which then "already
  exists", a refusal that explained nothing.

How a path is compared:

- It is made absolute against an explicit `cwd` parameter: the process's
  working directory in production, a test-chosen directory in tests. A
  relative path inside a vault is the bug this prevents.
- Symlinks are resolved as far as the path exists, so macOS's `/var` and
  `/private/var` compare equal.
- Paths are compared by component, never by string prefix.

The guard runs before the existence and parent checks, because it is the
only check that holds when nothing on the path exists yet.

## Consequences

- `an_empty_recovery_key_out_is_refused_as_empty` and the guard tests in
  `init_recoverykey_tests.rs` pin the behavior.
