# ADR-0014: `git2`, statically vendored

- Status: Accepted
- Origin: Q-GIT-CRATE ([entry](../archive/rust-port/open-questions.md#q-git-crate))

## Context

All git operations run in-process; gage never shells out to `git`. The
candidates were `gix` and `git2`.

## Decision

`git2` 0.21 (libgit2 1.9.7), with `default-features = false` and features
`https`, `vendored-libgit2`, `vendored-openssl`, plus `libz-sys` with
`static`. SSH was added later ([ADR-0015](0015-ssh.md)).

`gix` was rejected because it has no push: nothing in `gix` or
`gix-protocol` implements the receive-pack client, and gage pushes after
every write. Revisit only if the crate grows a push client.

## Consequences

- libgit2 honors the user's git config unless told not to
  ([ADR-0016](0016-git-isolation.md)).
- `deny.toml` allows `Unicode-3.0` (`url` → `idna` → ICU). libgit2's GPLv2
  with linking exception permits static linking into an Apache-2.0 binary.
- A C dependency means following libgit2 and OpenSSL advisories; `make
  audit` covers the `-sys` crates.
- Repo-local config is also pinned at init (`core.autocrlf=false`) as
  defense in depth for a vault opened by stock git.
- Merges never use `merge_trees` ([ADR-0019](0019-file-merge.md)).
