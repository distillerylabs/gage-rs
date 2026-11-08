# ADR-0070: The edit scratch file and signals

- Status: Accepted
- Origin: Remove the edit scratch file when gage is ended by a signal ([#216](https://github.com/distillerylabs/gage-rs/issues/216))
- Amends: [ADR-0036](0036-editor.md), whose "every exit path" this makes exact

## Context

ADR-0036 overwrites and removes the scratch file in `Drop`. That covers
returns, errors and panics, but a signal's default action ends the process
without running `Drop`, leaving a decrypted entry on disk. On Linux that
file sits in tmpfs until a reboot. A closed terminal is enough to cause it.

## Decision

On Unix, for as long as the scratch file exists, gage catches four signals
and restores the previous dispositions afterwards. A signal that was already
ignored when the edit began stays ignored, so `nohup` keeps working.

- **`SIGINT` and `SIGQUIT` are the editor's to decide.** The terminal sends
  them to the whole foreground process group, so the editor has them too,
  and many editors use them for their own purposes. gage absorbs them and
  keeps waiting. If they end the editor, gage takes the failed-editor path.
  `git` treats its editor the same way.
- **`SIGTERM` and `SIGHUP` end the edit.** gage forwards the signal to the
  editor and waits up to two seconds, then kills it. Only then is the file
  overwritten and removed, because a live editor could write the path back.
  gage then re-raises the signal, so its exit status still reports it.

The handler only stores to an atomic. The wait for the editor polls
`try_wait`, because the standard library's blocking wait retries when
interrupted and would never notice the signal.

## Not covered

- `SIGKILL` and power loss can't be handled. They are accepted risks.
- Other fatal signals (`SIGSEGV`, `SIGALRM`, `SIGUSR1`, ...) keep their
  default action. Nothing in a normal terminal or session teardown sends them.
- Windows console events are not handled. Ending a console process runs on a
  handler thread and does not unwind the main one, so it would need a
  separate design. Ctrl-C or closing the console during an edit can leave
  the file behind on Windows.
- An editor that hands off to a separate process (`code --wait`) can stop
  only its wrapper. The window that stays open holds its own copy.
