# gage-rs user documentation

What the gage-rs binary does today. Each page describes shipped behavior
and says where a later phase of the [port plan](../implementation/00_gage-cli/plans/rust-port/index.md)
extends it. For everything gage will do, see [Go gage's documentation](https://github.com/distillerylabs/gage/tree/main/doc/cli).

- [Vaults](vaults.md): creating, listing, inspecting, re-pointing and
  forgetting vaults.
- [Entries](entries.md): inserting, generating, reading, editing,
  renaming, listing and deleting secrets and notes.
- [Addressing entries](addressing-entries.md): how a query finds an entry,
  and what happens when it finds more than one.
- [Session mode](session-mode.md): `gage` with no command, which unlocks
  each vault once for many commands, and `--script`/`--stdin`, which run
  a list of commands the same way with nobody at the prompt.
- [Identities and recipients](identities-and-recipients.md): who can read a
  vault, adding and removing recipients, `recipient verify`, the local
  trust cache and the recipient-change confirmation.
- [Adding a device](enrollment.md): `identity enroll`, enrollment codes,
  and the `recipient pending`/`approve`/`deny` flow.
- [Sharing entries between vaults](sharing.md): `mv` and `cp` with
  `--to-vault`.
- [Sync and conflicts](sync.md): the automatic pull and push, `sync`,
  `pull` and `push`, and resolving an entry both devices changed.
- [Git remotes and authentication](git-and-auth.md): `init --remote`,
  `clone`, `git set-remote`, `auth` tokens, and SSH.
- [Configuration and file locations](configuration.md): where gage keeps
  its config and data, and what a vault looks like on disk.
- [Exit codes](exit-codes.md): what each exit status means.
