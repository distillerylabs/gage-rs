# ADR-0045: When enroll touches the network

- Status: Accepted
- Origin: D-ENROLL-REMOTE ([design doc](../design/gage-cli-init-design.md)); Q-ENROLL-REMOTE-ROUTE ([entry](../archive/rust-port/open-questions.md#q-enroll-remote-route))

## Decision

Enroll is the one write that is worthless unless it reaches the remote, so
it fetches first and fails early if the remote is unreachable, and pushes
last and fails legibly if it is unwritable.

- It pulls explicitly. A joining device never unlocks the vault, so the
  automatic pull that rides on unlock never runs. Committing onto a stale
  tip would end in a divergence message pointing at `gage sync`, which
  cannot work for a device that cannot decrypt anything.
- An unreachable remote **fails** the command (exit 7). This departs from
  "warn and proceed" for reads, because an enrollment request that was never
  published is a private key, a local commit and a code nobody can act on.
- It reaches the remote through the lock-held sync functions
  (`pull_locked`, `push_locked`), never `pull()` or `push()`, which take the
  lock; the lock is not re-entrant. A failed fetch happens before the
  identity is created and before any prompt. On any push failure the
  identity and the local commit are kept.
- "No remote configured" and "remote unreachable" are separate typed errors
  with different fixes.

Order, holding the write lock from step 2 through 8 on every exit path:

1. remote configured
2. take the vault write lock
3. discard an unexpectedly dirty working tree ([ADR-0007](0007-dirty-tree-reset.md)), before the pull
4. fetch and fast-forward; a bad token is usually discovered here
5. already-a-recipient check, then device-name check ([ADR-0043](0043-enroll-collisions.md))
6. create or reuse the identity (the only step that prompts)
7. seal, write to `pending/`, commit
8. push, then release the lock
