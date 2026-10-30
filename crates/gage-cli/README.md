# gage-cli

`gage` (`git` + `age`): a git-backed, age-encrypted secret and notes manager.
Entries are encrypted to the vault's recipients, stored one opaque file each,
and synced through an ordinary git remote.

## Install

```
cargo install gage-cli
```

This installs the `gage` binary.

## Use

```
gage init personal
gage insert github
gage show github
gage help
```

Run `gage` with no arguments in a terminal for an interactive session.
Documentation for every command is in
[doc/cli](https://github.com/distillerylabs/gage-rs/tree/main/doc/cli); the
design is in
[doc/design](https://github.com/distillerylabs/gage-rs/tree/main/doc/design).

Licensed under Apache-2.0.
