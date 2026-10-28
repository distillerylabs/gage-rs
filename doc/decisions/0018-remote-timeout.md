# ADR-0018: Network operations are bounded to 30 s

- Status: Accepted
- Origin: Q-REMOTE-TIMEOUT ([entry](../archive/rust-port/open-questions.md#q-remote-timeout))

## Decision

`REMOTE_OP_TIMEOUT` is 30 s, public, enforced in two layers:

- libgit2's server connect and read timeouts, set once in the isolation
  `Once` ([ADR-0016](0016-git-isolation.md)), bound a blackholed connect
  and a stalled read.
- A per-operation deadline is checked in the transfer-progress and sideband
  callbacks (fetch), in push negotiation (before the pack is sent), and in
  the credentials callback. Returning `false` aborts; the abort classifies
  as unreachable ([ADR-0017](0017-sync-classify.md)).

git2's pack-builder and push-transfer progress callbacks return nothing and
cannot abort, so they are not used for the deadline. Tests use a zero
deadline, never real time.

## Consequences

- Every automatic operation and every manual verb starts its own deadline.
  The push after a conflict resolution gets a fresh one: a human choosing
  between two versions of a password is not network time.
- There is no cancellation token. One can be added if a frontend needs it.
