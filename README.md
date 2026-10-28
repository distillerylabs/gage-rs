# gage-rs

[![Test](https://github.com/distillerylabs/gage-rs/actions/workflows/test.yml/badge.svg?branch=main)](https://github.com/distillerylabs/gage-rs/actions/workflows/test.yml)
[![Lint](https://github.com/distillerylabs/gage-rs/actions/workflows/lint.yml/badge.svg?branch=main)](https://github.com/distillerylabs/gage-rs/actions/workflows/lint.yml)
[![Audit](https://github.com/distillerylabs/gage-rs/actions/workflows/audit.yml/badge.svg?branch=main)](https://github.com/distillerylabs/gage-rs/actions/workflows/audit.yml)
[![QA](https://github.com/distillerylabs/gage-rs/actions/workflows/qa.yml/badge.svg?branch=main)](https://github.com/distillerylabs/gage-rs/actions/workflows/qa.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/distillerylabs/gage-rs/badges/coverage.json)](https://github.com/distillerylabs/gage-rs/actions/workflows/test.yml)
[![Dependencies](https://deps.rs/repo/github/distillerylabs/gage-rs/status.svg)](https://deps.rs/repo/github/distillerylabs/gage-rs)

`gage` (`git` + `age`) is a git-backed, age-encrypted secret and notes manager for the command line, written in Rust.

Secrets and notes are stored as individual [age](https://age-encryption.org/)-encrypted files in a git repository. Git handles history, sync and multi-device distribution; `gage` handles encryption, recipient management, and a fast CLI (and REPL) for storing and retrieving entries. There is no server, no daemon and no proprietary storage format: a vault is a git repo you can clone, back up and inspect like any other.

## Why

- **No server to trust or run.** Vaults sync over plain git remotes (self-hosted, a private GitHub repo, whatever you already use).
- **You already know how to inspect it.** A vault is a normal git repo on disk; `git log`, `git clone` and `git remote` all work as expected.
- **Opaque on disk.** One `.age` file per entry, named by random UUID. Someone with read access to the repo but not a decryption key sees ciphertext and meaningless filenames, not even titles or categories.
- **No long-lived agent process.** Unlocking happens in-process; the decrypted key lives only as long as that `gage` process runs.
- **Pluggable decryption, per device.** Each device chooses its own decryption method independently of what a vault's other devices use.

## How it works

A **vault** is a git repository plus:

- `.age-recipients`: the public keys of every device allowed to decrypt the vault. Recipients are vault-wide; there is no finer-grained per-entry sharing. A different set of readers means a different vault.
- `.gage/config.toml`: vault-level config (vault id, type, default decryption method, device metadata).
- `entries/*.age`: one encrypted file per entry. Everything a human would recognize (title, description, the secret value, structured fields) lives *inside* the encrypted payload as YAML.

Every write is a git commit; `gage sync` pulls, merges what can be cleanly merged, and pushes. Because entries are opaque encrypted blobs, git cannot three-way-merge a genuine conflict (both sides changed the same entry), so that case is always surfaced to you rather than silently resolved.

## Status

**Not released, and not independently audited.** The on-disk format is specified and frozen as version 2 ([doc/format/](doc/format/README.md)); the CLI is pre-1.0. gage leans on the `age` library for all cryptography, but the surrounding vault, sync and key-handling logic is new. Evaluate it accordingly before trusting it with secrets you cannot afford to lose or expose, and keep independent backups. See the [security model](doc/design/security-model.md).

## Documentation

- Using gage: [getting started](doc/cli/getting-started.md), then the rest of [doc/cli/](doc/cli/README.md). Build it with `make build`.
- Design rationale: [doc/design/](doc/design/gage-cli-design.md), including the [security model](doc/design/security-model.md).
- The on-disk format: [doc/format/](doc/format/README.md).
- Decisions that constrain the code: the [decision log](doc/decisions/README.md).
- Contributing: [CONTRIBUTING.md](CONTRIBUTING.md). Security reports: [SECURITY.md](SECURITY.md).

Licensed under [Apache-2.0](LICENSE).
