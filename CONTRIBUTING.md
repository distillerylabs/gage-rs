# Contributing to gage-rs

Thanks for your interest! gage-rs is a port of [Go gage](https://github.com/distillerylabs/gage), so the most useful contributions right now follow the [port plan](doc/implementation/00_gage-cli/plans/rust-port/index.md). For anything non-trivial, please open an issue first so we can agree on the approach before you spend time on it. Security problems should go through [SECURITY.md](SECURITY.md), not a public issue.

By participating you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
Contributions are licensed under the project's [Apache-2.0 license](LICENSE).

## Setup

> The Cargo workspace and Makefile land in the first code milestone (P1 · M0a). Until then this repo holds only docs.

Requires the Rust toolchain pinned in `rust-toolchain.toml` (rustup installs it automatically).

```
make build       # cargo build -> target/debug/gage
make test        # offline cargo test across the workspace (no network, no pre-existing keys)
make lint        # cargo fmt --check + cargo clippy -D warnings
make fmt         # cargo fmt --all
make audit       # cargo-deny: advisories, bans, licenses
```

Run a single test with `cargo test -p gage <test_name> -- --nocapture` (or `-p gage-cli`).

## Before opening a PR

`make lint` and `make test` must both be clean. CI runs them natively on Linux, macOS, and Windows, but it should confirm your change, not discover problems. Several invariants (page-locking, file locking, core-dump suppression) depend on real platform syscalls, so please don't assume a change that passes on one OS passes on all three.

## Conventions

The project's conventions are documented in [CLAUDE.md](CLAUDE.md) (written for AI coding assistants, but they apply to everyone). The ones that matter most:

- **Go gage is the reference.** gage-rs is fully on-disk compatible with Go gage. If the two binaries disagree, Go is right unless a decision in [open-questions.md](doc/implementation/00_gage-cli/plans/rust-port/open-questions.md) says otherwise.
- **Library/CLI split.** All real behavior lives in `crates/gage`; `crates/gage-cli` is a thin clap frontend. `crates/gage` must never use the print macros, touch stdin/stdout/stderr, or call `std::process::exit`. Clippy enforces this. Anything needing a human decision is a typed return value that the CLI renders.
- **Tests first, at task granularity.** Write the tests for a change, then the implementation. Prefer in-process tests with injected IO and a fake `Prompter`; use a real terminal (pty) only where genuinely needed.
- **No network, no real keys in tests.** Git behavior is tested against ephemeral local bare repos, not a mocked git crate. Use the existing seams (`Prompter`, `Locker`, `RemoteSyncer`) for failures that can't be produced honestly.
- **Typed, distinguishable errors** at library boundaries so callers decide retry and UX policy.
- **Every vault write holds the per-vault lock** (`vaultlock`) across the full read-modify-commit sequence.
- **No shelling out to `git`.** All git operations go through an in-process git crate.
- **New commands** are registered in the single command registry, not just added as a clap subcommand.

Design rationale for most non-obvious decisions is in [doc/implementation/00_gage-cli/tdds/gage-cli-design.md](doc/implementation/00_gage-cli/tdds/gage-cli-design.md); please read the relevant section before changing behavior it documents.

## Dependencies and tooling

Cargo dependencies and GitHub Actions are updated by Dependabot. Cargo-installed tools pinned in the `Makefile` are not covered by it, so bump those by hand when needed.
