# Entries

An entry is one secret or note: a title, an optional description, a value,
and optional structured fields. Every entry is its own age-encrypted file
in the vault, encrypted to every recipient the vault lists, and every
write is one git commit.

> **Status.** gage-rs can `insert`, `cat`, `rm` and `ls` entries. Each
> command unlocks the vault, does one thing and forgets the key: nothing is
> cached between commands. Addressing is by exact title or exact id only;
> substring matching, `show`, `edit`, `rename`, `search` and sessions arrive
> with phase 3 of the [port plan](../implementation/00_gage-cli/plans/rust-port/index.md),
> sync with phase 4. `insert -e` and `ls --header` are not there yet.

Every entry command takes `-u`/`--use NAME` to pick a vault. Without it,
the current vault is used (see [Vaults](vaults.md)). A name that isn't
registered fails before anything is asked for.

## Unlocking

Entry commands ask for this device's passphrase, the one chosen at `gage
init`. A wrong one is re-asked, up to three attempts in all, then the
command exits `5` (`locked-or-auth`). Prompts go to stderr, so redirected
output never contains one. When stdin isn't a terminal, each answer is one
line of stdin: `insert --value-stdin` after a piped passphrase works,
because gage reads no further than the end of the line it asked for.

## `gage insert <title>`

```
gage insert <title> [--description TEXT]
                    [-m | --value-stdin]
                    [--field NAME=VALUE ...]
                    [-f | --force] [-u NAME]
```

Creates an entry and commits it. It prints `gage: inserted "<title>"
(<id>)`.

**The value** comes from one of three places, after the vault is unlocked
(so a wrong passphrase never makes you type a secret for nothing):

- by default, a prompt: masked, like the passphrase;
- `-m`/`--multiline`: stdin, read verbatim until end of input. On a terminal
  that is Ctrl-D at the start of a line;
- `--value-stdin`: stdin, verbatim, with exactly one trailing newline (`\n`
  or `\r\n`) removed, so `echo secret | gage ...` stores `secret` while a
  value that really ends in a blank line keeps it.

`-m` and `--value-stdin` together are a usage error (exit `2`), reported
before anything is read. A value read from stdin must be UTF-8.

**Fields** are structured key/value metadata stored alongside the value.
`--field NAME=VALUE` (repeatable) splits on the first `=`, so a value may
contain `=`. Every whitespace character is removed from `NAME`; `VALUE` is
kept exactly, and may be empty. A field with no `=`, a `NAME` that is empty
once stripped, or the same `NAME` twice is a usage error before any unlock:
there is no silent last-wins.

**Duplicate titles** are refused (exit `1`, `conflict`) unless you pass
`-f`/`--force`. Two entries with the same title can then only be told apart
by id.

`created` and `updated` are set to now, and `updated_by` to this device's
name.

## `gage cat <query>`

Prints the entry's full decrypted contents as YAML, for scripting and
piping: `title`, `description`, `created`, `updated`, `updated_by`,
`value`, `fields`. It is the same text that is stored inside the
ciphertext. (A later phase adds the short id and aligned labels for
display.)

## `gage rm <query>`

Deletes the entry and commits the deletion. It prints `gage: removed
<id>`.

## `gage ls`

Lists every entry, sorted by title (then id), one per line:

```
ProtonMail  3f9a1c2e  2026-01-14  2026-03-02  laptop-1
```

The columns are title, short id (the first 8 characters), created,
updated (both UTC, to the day) and `updated_by`. `-` stands in for any
value an entry doesn't have. An empty vault prints nothing.

## Addressing an entry

`cat` and `rm` take a *query*. It is tried as an exact title first, then as
an exact id, in any spelling a UUID is written (hyphenated, without
hyphens, braced, or as a `urn:uuid:`). Title comes first, so a title that
happens to look like an id is still the title. Matching is exact: no case
folding, no prefixes (substring matching arrives with phase 3).

If a query matches more than one entry (two entries inserted with the same
title), nothing is read or removed. gage lists the candidates on stderr,
with their full ids, and exits `4` (`ambiguous`); pass one of the ids.

## Commits and concurrency

Each write is one git commit: its message is the entry's id and nothing
else (a title in a commit message would leak what a secret is called to
anyone who can read the repository), and its author and committer are
`gage <gage@localhost>` whatever your own git configuration says. The
commit is local. Pushing is a later phase.

Every write holds the vault's lock for the whole read-check-write-commit
sequence, so two `gage` processes writing to one vault take turns instead
of corrupting each other's commits. A second writer waits up to ten
seconds, then fails with exit `1` (`conflict`) naming the process that
holds the lock. Reads (`cat`, `ls`) never take the lock and never wait for
a writer. The lock is released by the operating system if the holder
dies, so there is nothing stale to clean up.
