# ADR-0017: Fetch and push failures are classified by type

- Status: Accepted
- Origin: Q-SYNC-CLASSIFY ([entry](../archive/rust-port/open-questions.md#q-sync-classify)); Q-SYNC-PUSHED ([entry](../archive/rust-port/open-questions.md#q-sync-pushed))

## Decision

Failures fall into buckets with fixed exit codes, decided from
`git2::Error` class and code and **never from message text**.

| Bucket | Exit code | Detected by |
|---|---|---|
| Unreachable | 7 | Network, OS, TLS-connect and SSH-connect failures; gage's own deadline abort; a local path that does not exist |
| Auth | 5 | `ErrorCode::Auth`; the whole HTTP error class (401, 403, 404, and also a 5xx or a URL that is not a repository); a token gage's credentials callback refused |
| Diverged | 1 | Push: `ErrorCode::NotFastForward`, any status reported through `push_update_reference`, or any other push failure by elimination |
| Internal | 6 | Fetch only: a failure in no other bucket. A fetch is never rejected for divergence |

A ref the server refuses arrives through the `push_update_reference`
callback while `push()` returns `Ok`; any status reported there counts as
diverged. The classified error carries a message with credentials stripped;
it never carries a raw `git2::Error` whose text might contain a tokened URL.

## Consequences

- The full observed table is the doc comment on `syncerr`, and each row is
  triggered for real by tests.
- `RemoteSyncer::push` returns `Result<bool, _>` and `SyncReport::pushed` is
  what it returned: a `sync`, `push` or `pull` with nothing to send says
  the vault "is already in sync with origin" rather than claiming a push.
