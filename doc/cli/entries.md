# Entries

An entry is one secret or note: a title, an optional description, a value,
and optional structured fields. Every entry is its own age-encrypted file
in the vault, encrypted to every recipient the vault lists, and every
write is one git commit.

> **Status.** gage-rs can `insert`, `generate`, `show`, `cat`, `edit`,
> `rename`, `rm`, `mv`, `cp`, `ls`, `search`, `log` and `history --decrypt`
> entries. Run one-shot, each command unlocks the vault, does one thing and
> forgets the key; in a [session](session-mode.md) (or a
> [script](session-mode.md#non-interactive-sessions-scripting)) they all
> reuse the key the session holds, and `ls`, queries and `search` read its
> [metadata index](session-mode.md#the-metadata-index).

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
                    [-m | --value-stdin | -e]
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

`-m`, `--value-stdin` and `-e` are mutually exclusive: two of them together
are a usage error (exit `2`), reported before anything is read. A value
read from stdin must be UTF-8.

**Fields** are structured key/value metadata stored alongside the value.
`--field NAME=VALUE` (repeatable) splits on the first `=`, so a value may
contain `=`. Every whitespace character is removed from `NAME`; `VALUE` is
kept exactly, and may be empty. A field with no `=`, a `NAME` that is empty
once stripped, or the same `NAME` twice is a usage error before any unlock:
there is no silent last-wins.

`--field` and `-e` are mutually exclusive too: they are two ways to set
fields, and gage won't guess which wins.

**`-e`/`--edit`** opens a template in `$EDITOR` instead: the title and
`--description` filled in, `value` and `fields` empty. Whatever you save is
inserted, including a changed title, which is then the one the duplicate
check uses. It is the one insert mode that can set fields alongside a
multi-line value. Nothing is written, and the command exits `2`, if the
file comes back unchanged, or with `value` and `fields` both still empty
(a title or description alone is not something to save). See
[`gage edit`](#gage-edit-query) for how `$EDITOR` is run and where the
file lives.

**Duplicate titles** are refused (exit `1`, `conflict`) unless you pass
`-f`/`--force`. Two entries with the same title can then only be told apart
by id.

`created` and `updated` are set to now, and `updated_by` to this device's
name.

## `gage generate <title>`

```
gage generate <title> [-l N | --length N] [--no-symbols]
                      [--description TEXT] [-f | --force] [-u NAME]
```

Like `insert`, with the value drawn at random from the operating system's
secure random source: 24 characters by default, from letters, digits and
`!@#$%^&*-_=+` (a set unlikely to need escaping in a shell, a URL or a
CSV). `--no-symbols` draws from letters and digits only. `-l` sets the
length; below 8 is refused (exit `2`) before anything is unlocked, since a
shorter value is too weak to be worth generating.

It prints `gage: generated "<title>" (<id>)`, and never the value: a fresh
secret doesn't belong in your scrollback. Read it back with `gage show`.

## `gage show <query>`

Prints the entry's value and nothing else, for piping into another
command: exactly one trailing newline, none added to a value that already
ends in one, so a multi-line value prints as stored.

`--field NAME` prints that field instead of the value. A name the entry
doesn't have exits `3` (`not-found`), listing the field names it does have
(never their values), or saying it has none.

`-c` and `-q` hand the value over instead of printing it; see
[Copying and QR codes](#copying-and-qr-codes).

### Copying and QR codes

`show` and `generate` take two flags that deliver a secret without putting
it in your scrollback. They are exclusive: nothing prints the value
alongside either one, and `-c` with `-q` is refused (exit `2`). `generate`
checks that before it writes anything, so a refused run never leaves a new
secret in the vault.

- **`-c`/`--clip`** copies the value to the clipboard and says so on
  stderr; nothing reaches stdout. One-shot, it writes `gage: copied to
  clipboard; clears in 45s — Ctrl-C to clear now.`, then waits out the
  timeout, clears the clipboard and exits `0`; Ctrl-C during the wait
  clears it at once and also exits `0` (you are done pasting, not
  aborting). In a
  [session](session-mode.md#everything-else-works-in-a-session-too) it
  writes `gage: copied to clipboard; clears in 45s.`, the clear is
  scheduled and the prompt comes straight back, and ending the session
  clears anything pending.

  The timeout is `[shell].clipboard_timeout`, 45 seconds by default (see
  [Session mode](session-mode.md#shell-settings)). gage keeps only a
  SHA-256 of what it copied, never the value, and clears only if the
  clipboard still holds that: if you've copied something else since, it is
  left alone, and so is a clipboard gage can't read. A second copy
  replaces the first one's pending clear. A scheduled clear that fails is
  reported on stderr, since "your secret is still on the clipboard" is
  something you need to hear.

  One limit: if gage is killed outright (`SIGKILL`) the value stays on the
  clipboard, because nothing was left running to clear it. gage starts no
  background process that outlives it.
- **`-q`/`--qr`** renders a terminal QR code instead, in half-block
  characters with a four-module quiet zone, to scan with a phone rather
  than type or paste. With `show --field NAME -q` it encodes only that
  field, so a structured note isn't dumped whole into a code when you
  wanted one piece. An empty value, or one too long for a QR code, is
  refused (exit `2`).

With `-c`, `show --field NAME` copies that field, and `generate -c`
copies the new value and still prints `gage: generated "<title>" (<id>)`.

## `gage cat <query>`

Prints the entry's full decrypted contents as YAML, for scripting and
piping: `title`, `description`, `created`, `updated`, `updated_by`,
`value`, `fields`. It is the stored text with three display-only changes:

- an `id:` line, the entry's short id (the one `ls` prints), right after
  `title`. It isn't part of what's stored.
- the labels padded so every `:` lines up, with each `fields` mapping's
  keys aligned independently.
- a multi-line value printed flush left, with no added indentation, so it
  copies and pastes cleanly.

```
title      : GitHub
id         : 4245dddd
description: code host
created    : 2026-10-17T03:34:30Z
updated    : 2026-10-17T03:34:30Z
updated_by : laptop-1
value      : hunter2
```

That makes `cat` display-only for scripts that parse it: `grep '^value:'`
and `cut -d: -f2` need to allow for the padding, and an entry with a
multi-line value no longer parses back as strict YAML. Use `gage show`
(optionally `--field`) when a script needs the raw value.

## `gage edit <query>`

Opens the entry's full YAML in `$EDITOR`, and saves whatever comes back as
long as it parses (an unparseable file changes nothing and fails). It
always commits, even if you changed nothing, as `git commit --amend` with
no new message still amends. `created` is kept from the original and
`updated`/`updated_by` are stamped fresh, whatever the file says. It prints
`gage: updated "<title>" (<short id>)`.

`$EDITOR` is split on whitespace (so `code --wait` works) and its first
word looked up on `PATH` before anything is written; unset or not found
fails with exit `6` and no file ever exists. The decrypted entry is written
to a scratch file only you can read (mode 0600 on Unix), named
unpredictably, in a memory-backed directory on Linux (`$XDG_RUNTIME_DIR` or
`/dev/shm`, whichever is verified as tmpfs) and the system temp directory
elsewhere. However the edit ends, the file's bytes are overwritten with
zeros and it is deleted. That overwrite is best effort, not a secure
erase: on a copy-on-write filesystem the old blocks may survive, which is
why Linux prefers tmpfs. A non-zero exit from the editor fails the edit
with exit `6`.

## `gage rename <query> <new-title>`

Changes only the title, stamping `updated`/`updated_by`; no editor. A
title another entry already has is refused (exit `1`) unless
`-f`/`--force`; renaming an entry to its own title is always allowed. It
prints `gage: renamed to "<title>" (<short id>)`.

## `gage rm <query>`

Deletes the entry and commits the deletion. It prints `gage: removed
<id>`.

## `gage mv <query> --to-vault <name>` and `gage cp`

Moves (or copies, keeping the original) the entry into another vault,
re-encrypted to that vault's recipients. The destination is never
unlocked. It prints `gage: moved "<title>" to <name> (<new id>)`. See
[Sharing entries between vaults](sharing.md).

## `gage ls`

Lists every entry, sorted by title (then id), one per line:

```
ProtonMail  3f9a1c2e  2026-01-14  2026-03-02  laptop-1
```

The columns are title, short id (the first 8 characters), created,
updated (both UTC, to the day) and `updated_by`. `-` stands in for any
value an entry doesn't have. An empty vault prints nothing.

`-H`/`--header` prints a labelled table instead, for a human reading a
terminal rather than a script: a header row, a row of dashes under it
broken with `+` at each column boundary, and rows delimited by ` | `. The
last column is never padded.

```
$ gage ls --header
title  | id       | created at | updated at | updated by
-------+----------+------------+------------+-----------
GitHub | 4245dddd | 2026-10-17 | 2026-10-17 | laptop-1
```

Plain `ls` is unchanged and stays the greppable form, and both shapes
print the same in a session as one-shot. An empty vault still prints
nothing.

## `gage search <pattern>` (alias `grep`)

Finds entries whose title, description, value or any field value contains
`<pattern>`, ignoring case. Each match prints as its title and short id,
sorted like `ls`:

```
$ gage search aws
AWS root account  4b9d7710
Cloud creds  9e01c3aa
```

It never prints what matched: a match in a value or field is a match in a
secret, and that stays out of scrollback. No match prints nothing at all,
so `gage search x | wc -l` counts matches. Like `ls`, it decrypts the
whole vault one-shot; in a session, titles and descriptions come from the
index and only the body half is decrypted.

## `gage log [query]`

When an entry changed, newest first: one line per commit with the local
time and the short commit hash.

```
2026-10-16 20:35:44 -0700  032ad60  3a001757
2026-10-16 20:34:59 -0700  4ce297b  bfaedc52
```

With no query it is every entry the vault holds now, as one timeline, with
each line's short entry id at the end. With a query it is that entry's
commits, without the id. A commit that deleted the entry is marked
`(deleted)`.

Nothing decrypted reaches the output, and commit messages are never
printed (they are the entry's UUID and would only repeat the id). `log`
still asks for your passphrase, because resolving a title means decrypting
the entries. A removed entry's history is reached by its full id, after
ordinary resolution finds nothing; a substring never matches a removed
entry, and a vault-wide `log` doesn't list removed ones. An entry with no
commits says so on stderr and exits `0`.

## `gage history --decrypt <query>`

Decrypts every past revision of one entry and shows what changed, oldest
first, each revision as a diff against the one before it:

```
== 2026-10-16 20:34:30 -0700  6f2eec5
+ title: GitHub
+ description: code host
+ value: hunter2
+ updated_by: laptop-1
```

A header per revision (time, hash, and ` (deleted)` where it removed the
entry), then one line for each compared part of the entry, marked `+`,
`-` or a space. A deleted revision prints its header and no body.

This is a separate command, and `--decrypt` and the query are both
mandatory, because it prints old secret values: more dangerous than seeing
*when* something changed. It doesn't ask for confirmation and has no bulk
form. A revision your identity can't decrypt is an error, never skipped.
Like `log` it accepts a removed entry's full id.

## `gage reindex`

Rebuilds a [session's](session-mode.md#the-metadata-index) metadata
index. One-shot there is no index, so it checks the vault resolves and
says there is nothing to reindex.

## Addressing an entry

`show`, `cat`, `edit`, `rename` and `rm` take a *query*: an exact title,
then a case-insensitive title substring, then an exact id, then any part
of an id (the short id `ls` prints works). A query that matches more than
one entry does nothing and lists the candidates (exit `4`). The full rules
are in [Addressing entries](addressing-entries.md).

## Commits and concurrency

Each write is one git commit: its message is the entry's id and nothing
else (a title in a commit message would leak what a secret is called to
anyone who can read the repository), and its author and committer are
`gage <gage@localhost>` whatever your own git configuration says. If the
vault has a remote, the commit is pushed straight after; being offline
costs a warning, never the write (see [Sync and conflicts](sync.md)).

Every write holds the vault's lock for the whole read-check-write-commit
sequence, so two `gage` processes writing to one vault take turns instead
of corrupting each other's commits. A second writer waits up to ten
seconds, then fails with exit `1` (`conflict`) naming the process that
holds the lock. Reads (`show`, `cat`, `ls`) never take the lock and never wait for
a writer. The lock is released by the operating system if the holder
dies, so there is nothing stale to clean up.
