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
  each vault once for many commands.
- [Configuration and file locations](configuration.md): where gage keeps
  its config and data, and what a vault looks like on disk.
- [Exit codes](exit-codes.md): what each exit status means.
