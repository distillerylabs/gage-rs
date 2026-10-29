# ADR-0020: Tokens reach libgit2 through the credentials callback

- Status: Accepted
- Origin: Q-REMOTE-AUTH ([entry](../archive/rust-port/open-questions.md#q-remote-auth))

## Decision

- **Storage.** Tokens live at `$GAGE_STATE/tokens/<host>`, mode 0600,
  written through `atomicfile`. `load` refuses a file others can read (on
  Unix). The host is validated as a single path component before any path is
  built. Two vaults on one host share one token. No stored token means an
  anonymous attempt, not an error.
- **The callback, not `custom_headers`.** A header set with
  `custom_headers` goes on every request, including one a redirect sends to
  another host. The `credentials` callback receives the URL, so the token is
  offered only to the host it was stored for. `follow_redirects(Initial)` is
  set explicitly.
- **No retry loop.** libgit2 calls the callback again after a 401; a second
  call within one operation fails, which classifies as auth with the
  host-naming hint.
- **Windows.** WinHTTP follows redirects to another host and re-applies an
  accepted credential to every later request without asking the callback. So
  gage sets `RemoteRedirect::None` on Windows (`gitrepo::REDIRECTS`); Unix
  keeps `Initial`. A same-host redirect is still followed. `clone`'s branch
  listing connects through an anonymous remote on a scratch repository with
  the same redirect policy on every platform.

## Consequences

Tests observe the token on the wire with a loopback HTTP recorder built on
`std::net` (`gittest`): it answers 401 with `WWW-Authenticate: Basic`,
records the retry's `Authorization`, and can answer 403 or 404 for
classification tests. Redirect tests pin that a token is never offered to a
host it was not stored for, including after the origin accepted it.
