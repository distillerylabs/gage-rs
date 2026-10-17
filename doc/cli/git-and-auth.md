# Git remotes and authentication

gage never runs a `git` binary. Every git operation goes through the git
library linked into gage (libgit2), and gage cuts that library off from
your git configuration entirely: system, global and XDG git config files
are never read. That has consequences worth knowing up front rather than
discovering by surprise:

- **`~/.ssh/config` is not read.** Host aliases, `IdentityFile`,
  `ProxyCommand`: none of it applies. A remote spelled
  `git@internal:secrets/work-vault.git` that relies on a `Host internal`
  block won't connect.
- **Git credential helpers are not used.** gage has its own token store
  (below).
- **`url.<base>.insteadOf` rewrites are ignored**, as is every other
  setting in your git config. A rewrite in your global config can't
  redirect a vault's push somewhere else.

## Giving a vault a remote

```
gage init <name> --remote <url>
gage git set-remote <name> <url>
gage clone <url> [--name NAME] [--dir DIR] [--device NAME]
```

- **`init --remote`** creates the vault and publishes its first commit
  straight away. gage never creates the repository for you: point it at an
  empty one you made on your git host. If the URL names a repository that
  doesn't exist, init says to create an empty one first; the local vault is
  kept, and `gage push` publishes it once the repository exists. If the
  remote can't be reached at all, init still succeeds, says so, and the
  next successful sync publishes the vault. For an HTTPS remote with no
  token stored for its host, init asks for one (a blank answer skips it,
  for hosts that need none).
- **`git set-remote`** sets or changes the vault repository's `origin`
  and gage's own record of it in your global config, in one step, so
  `gage vault info` never reports a stale remote. It contacts nothing. A
  vault can start local-only and gain a remote this way later.
- **`clone`** copies an existing vault and registers it on this machine.
  `--name` defaults to one inferred from the URL, `--dir` to
  `$GAGE_DATA/vaults/<name>`, and `--device` to this machine's normalized
  hostname. Cloning gets you the vault's files and history; it does not
  grant access to their contents. If this device can't decrypt anything
  yet, clone says so. On a terminal it offers to
  [enroll](enrollment.md) this device straight away (`[Y/n]`); otherwise
  it points at the commands that get you access. A clone that fails leaves
  nothing half-made behind.

Network work gives up after 30 seconds (one window for a whole sync's
fetch and push) rather than hanging a terminal on a dead host.

Enrolling a device needs **write** access to the remote: `gage identity
enroll` commits and pushes its request. A token that can read but not
write is only found out at the push, and the error names
`gage auth login`. A device without write access joins by the manual path
instead (`gage identity add` there, `gage recipient add` here). See
[Adding a device](enrollment.md#enrollment-needs-the-network-and-fails-if-it-cant-reach-it).

## HTTPS with a token: the supported path

```
gage auth login  [--host HOST]     # store a token for a git host
gage auth status [--host HOST]     # which hosts have a token stored
gage auth logout [--host HOST]     # forget it
```

This works the same against GitHub, GitLab, Gitea, Bitbucket or a
self-hosted server. It's not a GitHub feature; it's what follows from SSH
config not being honored. A token belongs to a *host*, not a vault: two
vaults on the same host share one. `--host` defaults to the host of the
current vault's `origin`.

- **Scope the token as narrowly as your host allows.** On GitHub, use a
  fine-grained personal access token limited to the vault's repository,
  not a classic `repo`-scoped token that reaches every private repository
  you own. `gage auth login` reminds you at the prompt.
- **You bring the token; gage never issues one.** There is no OAuth
  device flow, on purpose: on GitHub a device-flow grant is scope-based and
  reaches every private repository you own, which is broader than a
  properly scoped token.
- **The token is typed at a prompt, never passed as an argument**, where
  it would land in your shell history and the process list.
- **It is only ever sent to the host it belongs to.** gage hands it over
  when that host asks for credentials, and never to a different host,
  including one a redirect points at: redirects are followed only on the
  first request of an operation. A host that rejects it gets no second
  try; the operation fails.
- **A rejected or expired token fails legibly** (exit `5`), naming the
  host and `gage auth login` as the fix, rather than a bare HTTP 401 or
  403.
- **Tokens are stored at `$GAGE_STATE/tokens/<host>`**, mode `0600`, in
  local device state that is never synced (see
  [Configuration](configuration.md)): a token can always be had again by
  logging in. A token file anyone else can read is refused (exit `5`)
  rather than used, with the fix named.

A remote that is a local path (a bare repository on a mounted disk, say)
needs no token.

## SSH remotes

SSH is best-effort. A plain SSH remote (`git@github.com:you/vault.git`,
or an `ssh://` URL) works if `ssh-agent` is running and holds a key the
host accepts. gage asks the agent for credentials and nothing else: it
reads no key files and runs no `ssh` program.

What won't work is anything that needs `~/.ssh/config`. If the remote's
host isn't a resolvable host name, gage assumes it is an alias, refuses
(exit `2`) and says so, pointing you at the HTTPS URL. If ssh-agent can't
be reached, the operation fails (exit `5`) with the same advice.

## What gage doesn't model

For git operations gage has no command for (branches and tags, `gc`,
`fsck`, the reflog and the rest of git's long tail), remember that a vault
is an ordinary git repository at a known path (`gage vault info` shows
it). `cd` there and use the real `git` CLI. There is deliberately no
`gage git -- <args...>` passthrough.
