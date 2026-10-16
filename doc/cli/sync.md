# Sync and conflicts

Every write to a vault is a git commit, made immediately in your local
repository. That part never depends on the network and never fails because
of it. Getting those commits to and from a remote is a separate job, with
its own rules.

A vault with no remote (`gage init` without `--remote`) is a first-class
state, not an error: everything on this page is a no-op for it, and the
sync commands say it is local-only.

## What happens automatically

- **On every unlock** (a session's `use <vault>`, or the unlock behind a
  one-shot command), gage fetches and fast-forwards. Fast-forward only: it
  either catches up cleanly or leaves your local state exactly as it was.
  It never merges anything for you. If the network is down, gage warns once
  and carries on with your local copy: refusing to show a password because
  you are offline is worse than showing one that might be stale. If another
  `gage` process is in the middle of a write, the catch-up is skipped
  silently rather than making your read wait.
- **After every write** (`insert`, `edit`, `generate`, `rename`, `rm`),
  gage pushes straight after the commit. When nothing has diverged this is
  invisible. If the push fails because you are offline, nothing is lost:
  the commit already exists locally, like any git repository with unpushed
  commits, and the next push that gets through (after your next write, or
  a `gage push` or `gage sync`) publishes it. The warning says so: "the
  write is committed locally but not pushed".

Neither ever fails the command it follows. Only a sync command you ran
yourself reports a network problem as a failure.

## What never resolves itself

If another device pushed commits your copy doesn't have, the push after a
write fails and says so:

```
gage: origin has diverged — 2 local commit(s) pending; run `gage sync`
```

Entries are `.age` files, opaque binary blobs, so git can't merge two edits
to the same encrypted entry the way it merges text. A last-write-wins rule
could silently throw away a password that was just rotated, with no record
it happened. That is a much worse failure for a secrets manager than for a
notes app, so a real divergence always goes to a human.

## The sync commands

```
gage sync [--use NAME]
gage pull [--use NAME]
gage push [--use NAME]
```

- **`pull`** fetches and fast-forwards, and nothing else. A divergence is
  reported (exit `1`), never merged.
- **`push`** publishes your commits. If the remote has moved on, it
  fetches, merges a divergence whose two sides touched different files, and
  pushes again. A real conflict is reported with the conflicting paths
  named, and nothing changes.
- **`sync`** catches up, merges what can be merged, asks you about each
  entry both sides changed, then pushes. Day to day, the automatic
  behavior above means you rarely need it. It exists for the divergence
  case and for explicit control.

All three take the vault's write lock, as a write does, since a
fast-forward or a merge moves the working tree. A fast-forward or a merge
refuses to run over uncommitted changes in the vault's repository (exit
`1`), naming the files: a fast-forward would discard someone's
half-finished edit, and a merge would publish it in a commit nobody wrote
it for.

**Sync unlocks lazily.** A fast-forward, and a merge where the two sides
touched different entries, decrypt nothing, so `gage sync` asks for no
passphrase. It asks only when it reaches an entry conflict it has to
decrypt to show you. In a session, a vault that is already unlocked is not
asked for again.

### What counts as a conflict

| Situation | What happens |
|---|---|
| Both sides changed **different** entries | Merges cleanly, nothing to ask |
| Both sides changed the **same** entry | A real conflict: both versions are decrypted and you choose |
| One side **deleted** an entry the other **edited** | The surviving version is decrypted and you choose |
| Both sides changed **`.age-recipients`** or **`.gage/config.toml`** | Never merged; see below |
| Both sides **inserted** entries with the same title | Not a conflict: two entries with different ids. Tell them apart later through [query resolution](addressing-entries.md) |

Two devices that each add a different recipient conflict on
`.age-recipients` rather than merging into a list neither of them wrote.
gage decides that itself, file by file; it does not depend on the vault's
`.gitattributes`.

### Resolving an entry conflict

```
gage> sync
Entry conflicts: "ProtonMail"

  local    updated 2026-08-30 14:22  by laptop-1
  remote   updated 2026-08-29 09:03  by phone

  [l] keep local     [r] keep remote
  [b] keep both      [s] skip this entry     [q] abort sync
Which? [l/r/b/s/q]:
```

Each side shows when it was last written (UTC) and by which device. If one
side deleted the entry, its line says `deleted this entry`. Neither secret
value is printed: the choice is made on who wrote what and when, not by
putting two passwords on the screen.

- **`l` / `r`** keeps one side and discards the other. Keeping the remote
  side of a deletion deletes the entry.
- **`b`** (keep both) keeps your version where it is and writes the other
  version as a new entry under a fresh id, with its own title, `created`,
  `updated` and `updated_by`. Resolving a conflict never destroys a
  secret. You end up with two entries sharing a title, which you can
  reconcile later with `rm` or `rename`. A wrong choice made in a hurry can
  be undone; it is never a silent loss. When the other side had deleted
  the entry, there is nothing more to keep and your version stays as it
  is. When *you* had deleted it, the other device's version comes back
  under a fresh id.
- **`s`** (skip) leaves that entry conflicted and moves on to the next.
  Any skip means nothing at all is applied and nothing is pushed, not even
  the answers you gave to the other entries: the divergence is still there
  for the next `gage sync`. Applying only some answers would quietly turn
  each skip into "keep local".
- **`q`** (abort) stops asking and changes nothing.

There is no default. Anything other than the five keys is asked again, and
after five unrecognized answers gage gives up without choosing.

The result is one real git merge commit with both sides as parents,
whatever the number of answers. Its message is
`gage: merge origin, resolving 2 entries`: a count, never a title. Both
devices' commits stay in the history, and the vault is an ordinary git
repository you can inspect with `git log` and `git show`. Files the merge
creates are mode `0600`, like every entry gage writes.

The push that follows gets a fresh 30-second network window. The time you
spent deciding doesn't count against it.

### With nobody to ask

`gage sync` in a script, in CI, or in a `--stdin` session (anywhere stdin
is not a terminal) stops at the first entry conflict rather than choosing,
exits `1`, and changes nothing:

```
gage: resolving an entry conflict needs someone to ask
```

Run it again from a terminal.

### Conflicts gage won't ask about

- **The recipient files.** A divergence in `.age-recipients` or
  `.gage/config.toml` is refused with the paths named. Those files decide
  who can read the vault, and they are never offered in this menu. The
  recipient-change confirmation that resolves them arrives with the
  recipient commands in a later phase of the
  [port plan](../implementation/00_gage-cli/plans/rust-port/index.md).
- **Anything that isn't an entry.** A vault is an ordinary git repository,
  so a file you added yourself can conflict too. gage has no version of it
  to offer, refuses (exit `1`) naming the path, and asks for no
  passphrase. Resolve it with `git` in the vault's directory.

## What sync exits with

| Code | When |
|---|---|
| `0` | In sync, caught up, pushed, merged, or a conflict resolved and pushed. |
| `1` (`conflict`) | A divergence `pull` won't merge, a conflict `push` can't merge, a resolution that was skipped, aborted or had nobody to ask, a conflict on a non-entry or recipient file, or uncommitted changes in the vault. A human has something to do. |
| `5` (`locked-or-auth`) | The host refused the credentials, the stored token is unusable, ssh-agent can't be reached for an SSH remote, or the passphrase for a conflict's unlock was wrong. See [Git remotes and authentication](git-and-auth.md). |
| `7` (`unreachable`) | The remote could not be reached at all. Nothing is wrong on either side; try again later. |

## Concurrent `gage` processes

It's normal to have a session open in one terminal and run a one-shot
`gage show` in another, against the same vault. Every write and every sync
command holds the vault's lock for its whole read-modify-commit sequence,
and `gage sync` holds it for the whole resolution, including the time you
spend answering. A write in another terminal waits up to ten seconds, then
fails with exit `1`, rather than landing in the middle of the merge being
assembled. Reads never take the lock. See
[Commits and concurrency](entries.md#commits-and-concurrency).
