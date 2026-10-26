# Session mode

`gage` has two invocation modes. Neither involves a background daemon or a
shared agent: each is one process holding its own unlocked keys in its
own memory for as long as it runs.

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

### Tab completion and abbreviation

Tab completes what can be typed at the cursor: command and subcommand
names (and aliases), flags (after a `-`), vault names (`use`, `lock`, and
the value of `-u/--use`), and entry titles for a command's query. A single
match is filled in; several fill their common prefix and are listed.
Matching is by prefix, so `enti` never completes `identity`. `init` and
`clone` are never offered: they only run one-shot.

Entry titles come from the *current* vault, only while it is unlocked.
Completion never asks for a passphrase and never offers another vault's
titles, even after `--use`. A title containing spaces is filled in quoted.
Enrollment request ids (`recipient approve <ID>`, `recipient deny <ID>`)
are not completed: take them from `recipient pending`.

```
personal🔓 gage> show <Tab>
AWS root account  GitHub  ProtonMail
personal🔓 gage> show Pr<Tab>        (becomes: show ProtonMail)
```

Without Tab, the first word of a line may be an unambiguous prefix of a
top-level command: `stat` runs `status`. If several commands start with
it, nothing runs and the candidates are named; the session carries on.
Only that first word is expanded, never a subcommand or a flag, so
`vault inf` is an error rather than `vault info`.

```
gage> l
gage: "l" matches more than one command: lock, ls
gage> stat
no vaults have been unlocked in this session yet.
```

Completion is for the interactive prompt only; there are no shell
completion scripts. A passphrase or value typed at a prompt is never
completed, and completing records nothing in the history.

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

Four things differ:

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
- **`-c/--clip` schedules instead of waiting.** One-shot, `show -c` holds
  the process until the clipboard clears. In a session the clear goes on a
  timer and the prompt comes straight back; ending the session (`exit`,
  Ctrl-D or end of input) clears anything still pending, so a copy made a
  second before `exit` doesn't outlive the process that made it. See
  [Entries](entries.md#copying-and-qr-codes).
- **`init` is one-shot only.** It creates a vault rather than operating on
  one, which leaves "should the new vault become current?" unanswered, so
  in a session it says so instead of running.

A few commands don't need the vault unlocked: `identity enroll`,
`recipient pending`, `recipient approve` (until its last step),
`recipient deny`, `recipient list` and `recipient verify`. They work with
`--use NAME` against a vault this session has never `use`d and never
unlocks. That is the point for a device that is
[enrolling](enrollment.md): it can't unlock the vault yet. `use` on such a
vault still fails, and the message names `gage identity enroll` as the way
in.

`recipient approve` asks for a key only after you've confirmed the
approval. If the session already holds the vault's key, no passphrase is
asked at all. If it doesn't, the vault is unlocked once, after the "yes",
and stays unlocked like any other. A wrong code, a "no" or any refusal
before that point leaves the vault as locked as it was.

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
session runs without it. A [scripted session](#non-interactive-sessions-scripting)
never opens it: its lines were never typed.

### Trade-off against a shared agent

Unlike `ssh-agent`, unlocking a vault in one terminal does not unlock it
anywhere else: each session is its own island, and a second terminal
unlocks again. That trades some convenience for a smaller blast radius
(one process, not every shell opened since boot) and no socket whose
permissions need reasoning about.

## Non-interactive sessions (scripting)

For automation that wants "unlock once, do several things, then gone"
without a human at a keyboard:

```
gage --script deploy-secrets.gage      # session commands from a file
cat commands.txt | gage --stdin        # the same, from stdin
```

It is the same session as the interactive one: one process holding each
vault's key from its first use to the end of the run, then dropping it.
Each vault is unlocked at most once. What differs is deliberate:

- **It stops at the first failing line.** A script has nobody to notice a
  line that failed halfway down, and carrying on would run the rest
  against a state its author never anticipated. The message names the
  source, the line number and the line, and the exit code is the failing
  command's own:

  ```
  $ gage --script deploy.gage
  gage: deploy.gage:3: show NoSuchEntry: no entry matches "NoSuchEntry"
  $ echo $?
  3
  ```

  Under `--stdin` the source is called `<stdin>`. An ambiguous query
  fails `4` with its candidates listed in the message, as one-shot mode
  does: nobody is there to pick one.
- **Blank lines and lines starting with `#` are skipped**, so a script
  can be commented like any other file. `exit` (or `quit`) ends the run
  early, successfully.
- **It writes nothing to the history file**, which records what a human
  typed at a prompt.
- **Closing it works like closing a session**: the keys are dropped, and
  a pending `-c` copy is cleared, whether the run finished, hit `exit`
  or stopped on a failing line.

`--script` and `--stdin` are mutually exclusive, can't be combined with a
command (`gage --stdin show foo` asks for a session and a one-shot in one
breath, so it's refused: put `show foo` on a line instead), and aren't
available inside a session. A script file that can't be opened exits `2`
(`usage`).

`insert --value-stdin` and `-m` read stdin to its end, and under `--stdin`
stdin is the command stream. Don't combine them.

### Where the passphrase comes from

In this fixed order:

1. **`GAGE_PASSPHRASE`**, if it is set in the environment (an empty value
   is still set).
2. Otherwise a prompt, but only under `--script` with a terminal on
   stdin, which leaves stdin free for a human to answer on. `--stdin` has
   already spent stdin on the command stream and never prompts, even on a
   terminal.
3. Otherwise a refusal, exit `5` (`locked-or-auth`), naming the variable:

   ```
   gage: <stdin>:1: use personal: no way to unlock vault "personal" without a terminal: set GAGE_PASSPHRASE
   ```

   Never a read that can't be answered, and never a command line taken
   for a passphrase.

This applies to `--script`/`--stdin` runs and only to them: an ordinary
one-shot `gage show ...` ignores `GAGE_PASSPHRASE` and prompts.

`GAGE_PASSPHRASE` answers once. A wrong value fails after that one
attempt, not three, since asking again could only fail the same way.

It only opens an *existing* identity. It is never used for a new one
(`init`, `identity add`), because a new identity's passphrase can't be
checked against anything and a typo would silently create a key nobody can
unlock. That request goes to a human when there is one (`--script FILE` on
a terminal) and is refused when there isn't:

```
gage: <stdin>:2: identity add --device laptop-2: creating an identity for vault "personal" needs a passphrase typed by a human, and this run has no terminal to ask on; run it from a shell instead (GAGE_PASSPHRASE is deliberately not used for a new identity)
```

The refusal consumes nothing: the lines after it are still commands.
The same goes for an ambiguous query when nobody can be asked to pick.
A script driving two vaults with different passphrases can't use one
variable for both: run one `gage --stdin` per vault.

Treat the variable like any secret in the environment: the processes you
start from that shell can read it.

### `--yes` in a script

`--yes` is for runs with nobody to show a recipient diff to, and a script
is exactly that: `gage --script deploy.gage --yes` answers the
[recipient-change confirmation](identities-and-recipients.md#the-recipient-change-confirmation)
yes for the whole run, and nothing else. Without it, a script that would
write to a vault whose recipient list changed under it asks as usual where
a human can answer (`--script FILE` on a terminal), and where nobody can
it fails `1` (`conflict`) and writes nothing. A `--yes` on a script line is
accepted and adds nothing beside the run's own. It is still refused in an
interactive session, where a human is sitting at the prompt.

### What enrollment does in a script

`gage identity enroll` has two paths, and they behave differently here,
both correctly:

- **Creating a key refuses.** A new identity's passphrase can't come from
  `GAGE_PASSPHRASE`, for the reason above. Under `--script FILE` at a
  terminal the prompt goes to the human and enroll proceeds normally.
  Under `--stdin`, or with no terminal, it fails with `locked-or-auth`
  (`5`) before any key is generated.
- **Reusing an existing key succeeds.** That's an ordinary unlock, so
  `GAGE_PASSPHRASE` answers it, and a fully scripted enroll works end to
  end. A re-run after a failed push takes this path, and it's the reason
  enroll is scriptable at all.

A refusal costs nothing that matters: no identity is written, and nothing
is sealed, committed or pushed. The fast-forward enroll does first stays,
which only means the vault is more up to date than it was.

`gage recipient approve` is **not** scriptable, by design. Its confirmation
decides whether a device gets into the vault, and a run nobody is watching
must not make that decision by default. With nobody to ask, it fails with
`locked-or-auth` (`5`) and the request stays pending. `--yes` doesn't
answer it: that flag answers the recipient-change confirmation and nothing
else. `recipient pending` and `recipient deny` need no unlock and no
confirmation, so both work in a script.

## `[shell]` settings

In `$GAGE_CONFIG/config.toml` (see
[Configuration](configuration.md#the-global-config-file)):

```toml
[shell]
prompt = "[{vault}{lock}{dirty}] gage> "
idle_timeout = "10m"
history_file = "$GAGE_STATE/history"
clipboard_timeout = "45s"
```

- **`prompt`**: the template above.
- **`idle_timeout`**: a duration such as `90s`, `10m`, `1h` or `1h30m`
  (units `ns`, `us`, `ms`, `s`, `m`, `h`; decimals allowed, as in `1.5h`).
  `"0"` turns the re-lock off. A value that isn't a duration, or a
  negative one, stops the session from starting with exit `2` (`usage`),
  rather than quietly meaning "never re-lock".
- **`clipboard_timeout`**: how long `show -c` and `generate -c` leave a
  secret on the clipboard before clearing it, as a duration like
  `idle_timeout`'s. 45 seconds by default. Anything under one second is
  an error (there is no `0` to turn the clear off), and so is a value
  that isn't a duration. See
  [Entries](entries.md#copying-and-qr-codes).
- **`history_file`**: a path. `$GAGE_CONFIG`, `$GAGE_DATA` and
  `$GAGE_STATE` expand to gage's roots, and a leading `~` to your home
  directory.

Every setting is optional; leaving one out takes its default.
