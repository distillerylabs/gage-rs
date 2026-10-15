# Session mode

`gage` has two invocation modes. Neither involves a background daemon or a
shared agent: each is one process holding its own unlocked keys in its
own memory for as long as it runs.

> **Status.** The interactive session and its metadata index ship. Tab
> completion and command abbreviation arrive later in phase 3 of the
> [port plan](../implementation/00_gage-cli/plans/rust-port/index.md).
> Scripted sessions (`--script`, `--stdin`) arrive with phase 6, and sync
> (which `use` will also trigger) with phase 4.

## One-shot mode

```
$ gage show protonmail
```

Unlocks the vault, decrypts what it needs to resolve the query, does the
one thing asked, drops the key and exits. Every invocation pays the full
unlock cost (the passphrase prompt) because nothing is kept between
invocations. Best for scripting, one-off lookups and piping.

## Session mode

Running `gage` with no command, with stdin attached to a terminal, starts
an interactive prompt:

```
$ gage
personal🔒 gage> use personal
Enter passphrase for vault "personal" (device laptop-1): 
personal🔓 gage> show protonmail
correcthorsebatterystaple
personal🔓 gage> show --use work "AWS root account"
Enter passphrase for vault "work" (device laptop-1): 
hunter2
personal🔓 gage> status
* personal     unlocked  (last used 6s ago)
  work         unlocked  (last used 2s ago)
personal🔓 gage> lock personal
personal locked. run `use personal` to unlock again.
personal🔒 gage> exit
$
```

A session starts pointed at your current vault (`gage vault set-default`),
locked: the first command that needs its key asks for the passphrase. From
then on every command reuses the key, with no further prompts. The key
exists only in this process's memory (page-locked where the OS allows) and
is wiped when the session ends, by `exit`, `quit`, Ctrl-D or the process
being killed. There is nothing for another terminal to attach to.

If stdin is not a terminal (piped or redirected), a bare `gage` prints
help instead of starting a session.

### Typing commands

A line is split on whitespace. Single or double quotes group words, so a
title with spaces is `show "AWS root account"`. There is no backslash
escaping: a backslash is an ordinary character, so a Windows path types as
itself (`C:\keys\prod`). An unbalanced quote is reported and the line is
not run.

Line editing and history recall (the arrow keys) work as in a shell.
Ctrl-C abandons the line being typed and keeps the session; Ctrl-D on an
empty line leaves it.

A mistake doesn't end the session. An unknown command, a wrong number of
arguments, a failed command: each is reported on stderr, and the prompt
comes back with every key still held.

### Session-only commands

These exist only inside a session. Run one-shot, they say so and exit `2`
(`usage`).

```
use <vault>          make <vault> current, unlocking it if this session doesn't hold its key
lock [vault]         drop the key for <vault>, or for every vault, without leaving
status, whoami       list the vaults this session has touched and their lock state
help [command]       the commands available here, or one command's help
exit, quit           leave the session (Ctrl-D too)
```

`use` on a vault the session already holds re-selects it without asking
again. A bare `lock` names each vault it locked, or says none were
unlocked.

### Everything else works in a session too

Every entry and vault command runs in a session exactly as it does
one-shot, against the session's current vault and with its key. `--use
NAME` still works on any command, ad hoc: it unlocks that vault if the
session doesn't hold it yet, and leaves the current vault where it was.

Three things differ:

- **An ambiguous query asks** instead of failing. The candidates are
  listed and you pick one by number:

  ```
  personal🔓 gage> show aws
  Multiple entries match "aws":
    1. AWS IAM backup admin (a03e5f88)
    2. AWS root account (4b9d7710)
  Which one? [1-2]: 2
  hunter2
  ```

  A number outside the list is asked again, up to three times, then the
  command fails `4` (`ambiguous`) without picking. Ctrl-D at the question
  declines, and the candidates are listed as one-shot mode lists them.
  `rm` and `rename` ask too.
- **`insert -m` and `insert --value-stdin` are refused**: both read stdin
  to its end, and in a session stdin is the session's own input. Use
  `insert -e`, or run `gage insert` from your shell.
- **`init` is one-shot only.** It creates a vault rather than operating on
  one, which leaves "should the new vault become current?" unanswered, so
  in a session it says so instead of running.

### The metadata index

Titles live inside the encrypted entries, so one-shot `ls`, `search` or
any query decrypts the whole vault every time. A session decrypts each
vault's metadata once instead: the first `ls`, query or `search` against
a vault builds an index of every entry's title, description, dates and
last writer, and later commands read it. `insert`, `generate`, `edit`,
`rename` and `rm` update it as they go, so it stays current with no
rebuild. A vault you only write to never gets one.

The index holds no secrets: never a value and never a field. It lives in
memory only (page-locked where the OS allows, with a one-time warning if
it can't be), is never written to disk, and is wiped the moment its vault
is locked, by `lock`, the idle timeout or the session ending. Each vault
has its own.

`search` matches titles and descriptions from the index, but body text
(values and fields) is decrypted fresh for each search and dropped
straight after, so a matched secret is in memory no longer than a `show`
puts it there.

If a vault's entries change behind the session's back (a `git pull` run
by hand, another device's push), the index doesn't notice. `reindex`
rebuilds it from a fresh decrypt:

```
personal🔓 gage> reindex
gage: index rebuilt
```

One-shot, `gage reindex` has nothing to rebuild and says so.

### Idle timeout

A session left open in a terminal multiplexer over a weekend is an
unmanaged agent. After `[shell].idle_timeout` without a command touching
it (10 minutes by default), a vault's key is dropped exactly as `lock`
drops it. The process stays; the prompt shows `🔒`, and the next command
that needs the key asks for the passphrase again.

The timeout is checked between commands, never during one, so a command
that runs long is never cut off halfway.

### The prompt

`[shell].prompt` is a template with three tokens:

- `{vault}`: the current vault's name, or nothing if none is selected.
- `{lock}`: `🔓` or `🔒` for the current vault, or nothing if none is
  selected.
- `{dirty}`: `*` when the current vault's git working tree has uncommitted
  changes. Only computed if the template uses it.

The default is `{vault}{lock} gage> `. Leading spaces are trimmed after
the tokens are filled in, so with no vault selected it reads `gage> `.

### History

Every command line you type is appended to `[shell].history_file`
(`$GAGE_STATE/history` by default), which is created readable by you
alone (mode `0600` on Unix, tightened if it already existed). Only typed
command lines are recorded: a passphrase, a value typed at a prompt, or a
number typed at the picker never is, and neither is anything a command
prints. A history file that can't be opened is reported once and the
session runs without it.

### Trade-off against a shared agent

Unlike `ssh-agent`, unlocking a vault in one terminal does not unlock it
anywhere else: each session is its own island, and a second terminal
unlocks again. That trades some convenience for a smaller blast radius
(one process, not every shell opened since boot) and no socket whose
permissions need reasoning about.

## `[shell]` settings

In `$GAGE_CONFIG/config.toml` (see
[Configuration](configuration.md#the-global-config-file)):

```toml
[shell]
prompt = "[{vault}{lock}{dirty}] gage> "
idle_timeout = "10m"
history_file = "$GAGE_STATE/history"
```

- **`prompt`**: the template above.
- **`idle_timeout`**: a duration such as `90s`, `10m`, `1h` or `1h30m`
  (units `ns`, `us`, `ms`, `s`, `m`, `h`; decimals allowed, as in `1.5h`).
  `"0"` turns the re-lock off. A value that isn't a duration, or a
  negative one, stops the session from starting with exit `2` (`usage`),
  rather than quietly meaning "never re-lock".
- **`history_file`**: a path. `$GAGE_CONFIG`, `$GAGE_DATA` and
  `$GAGE_STATE` expand to gage's roots, and a leading `~` to your home
  directory.

Every setting is optional; leaving one out takes its default.
