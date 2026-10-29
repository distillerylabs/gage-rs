# gage-rs user documentation

`gage` is a command-line secret and notes manager. Every vault is a git
repository; every secret is an age-encrypted entry inside it. This is the
user documentation for the gage-rs binary: how to build it, create and use
vaults, manage who can read them, and keep multiple devices in sync.

These pages describe the behavior gage-rs commits to. Where a page rests on
a recorded decision, it links the [ADR](../decisions/README.md). For the
design rationale, see [doc/design/](../design/gage-cli-design.md); for the
on-disk format, [doc/format/](../format/README.md). This directory is written
for people *using* `gage`, not developing it.

## Contents

1. [Getting started](getting-started.md): build it, create your first vault, store and read a secret.
2. [Vaults](vaults.md): creating, cloning, listing, and switching between vaults.
3. [Entries](entries.md): the entry format, and every command that creates, reads, or edits one.
4. [Addressing entries](addressing-entries.md): how a typed query resolves to an entry, and what happens when it's ambiguous.
5. [Session mode](session-mode.md): `gage` with no command, which unlocks each vault once for many commands, and `--script`/`--stdin`, which run a list of commands the same way with nobody at the prompt.
6. [Identities and recipients](identities-and-recipients.md): who can read a vault, adding and removing recipients, `recipient verify`, the recovery key (`recovery verify`, `enroll` and `rotate`), the local trust cache and the recipient-change confirmation.
7. [Adding a device](enrollment.md): `identity enroll`, enrollment codes, and the `recipient pending`/`approve`/`deny` flow.
8. [Sharing entries between vaults](sharing.md): `mv` and `cp` with `--to-vault`, and why gage has no per-directory sharing.
9. [Sync and conflicts](sync.md): the automatic pull and push, `sync`, `pull` and `push`, and resolving an entry both devices changed.
10. [Git remotes and authentication](git-and-auth.md): `init --remote`, `clone`, `git set-remote`, `auth` tokens, and SSH.
11. [Configuration and file locations](configuration.md): where gage keeps its config and data, and what a vault looks like on disk.
12. [Command reference](command-reference.md): every command and flag in one place.
13. [Exit codes](exit-codes.md): what each exit status means.

## Conventions used in these docs

- A `gage>` prompt means the command is shown as typed inside an interactive
  session. A bare `$ gage ...` means it's a one-shot invocation from your
  regular shell. Almost every command works both ways; see
  [Session mode](session-mode.md).
- `<query>` means an argument that resolves to an existing entry (by title or
  ID); see [Addressing entries](addressing-entries.md). `<title>` means an
  argument that *names a new entry* (`insert`, `generate`).
- Flags shown as `[--use NAME]` are optional everywhere except where a page
  says otherwise.
- Example output shows gage-rs's own wording. `gage help` and `--help` are
  rendered by clap from the command registry
  ([ADR-0030](../decisions/0030-help-render.md)).
