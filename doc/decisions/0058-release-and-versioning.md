# ADR-0058: Release and versioning policy

- Status: Accepted
- Origin: first release R1a ([#150](https://github.com/distillerylabs/gage-rs/issues/150))

## Context

gage-rs is published to crates.io and as release binaries. That needs
crate names, a statement of what a version number promises, one version
string every build path agrees on, and a compiler floor that is checked
rather than claimed.

## Decision

**Two published crates, lockstep versions.** `gage-core` (the library, in
`crates/gage`; `[lib] name = "gage"`, so code still says `gage::`) and
`gage-cli` (the `gage` binary). Both always carry the workspace version.
The name `gage` on crates.io belongs to an unrelated crate, hence
`gage-core`. `reset-local-state` is a developer tool and stays
`publish = false`.

**What semver covers.** The CLI contract: help layout and flags, exit codes
0-7, the duration grammar and the `--version` shape. And `gage-core`'s
public API. Within 0.x a minor bump may break either. `gage-cli`'s library
target (`gage_cli::run`) is not a public API; it exists for in-process
tests. The `test-support` feature of `gage-core` ships because
`gage-cli`'s dev-dependency needs it and cargo resolves features at
publish; it is outside semver, its modules are `#[doc(hidden)]`, and the
binary never enables it (`tests/shipped_features.rs`).

**`format_version` is independent of crate versions.** A crate release may
leave the format alone; a format change follows ADR-0001 and
[doc/format/README.md](../format/README.md) and bumps `format_version`
whatever the crate version does.

**Only the latest release is supported.**

**One version string.** `gage --version` prints `gage <version> (commit
<sha7>)`, with no leading `v`, on every build path. The version is
`CARGO_PKG_VERSION`. The Makefile stamps only `GAGE_VERSION=dev` on an
untagged checkout, which prints `<version>-dev`; any other non-empty
`GAGE_VERSION` is an override that wins verbatim (the stamp check uses
one). A checkout tagged anything but `v<version>` fails `make build` and
`make stamp-check` (`tag-check`). The commit is, in order: `GAGE_COMMIT`;
`git.sha1` from the `.cargo_vcs_info.json` that `cargo package` writes, so
a `cargo install` build has one; `git rev-parse` (a missing git is not an
error); `unknown`.

**Package contents.** Each crate has its own `README.md` and a copy of the
root `LICENSE` (cargo packages only files inside the crate directory;
`tests/license_copies.rs` keeps the copies byte-identical), and an
`include` list: `src/**`, the manifest, the README, `LICENSE`, plus
`clippy.toml` (`gage-core`) or `build.rs` (`gage-cli`). `tests/` is
dropped, because the integration tests need the repo's `tests/fixtures/`;
the `*_tests.rs` unit modules under `src/` stay, since excluding a declared
`mod` file breaks a downstream `cargo test`.

**Compiler floor 1.95, checked.** `rust-version = "1.95"`. The earlier
claim of 1.89 (ADR-0003, `std::fs::File::lock`) was the floor for our own
code; `rustyline` 19 needs `cfg_select!`, stable in 1.95, and 1.94 fails.
ADR-0003's floor still holds, but is no longer the binding one. The `msrv`
job in `lint.yml` runs `cargo +1.95 check --locked -p gage-core -p
gage-cli` (not `--all-targets`: the promise covers building, not
dev-dependencies). Raising the floor is a minor-version event.

## Consequences

- `cargo publish --dry-run -p gage-core` and `cargo package --list -p
  gage-cli` run in CI. `gage-cli` can be dry-run published only once
  `gage-core` is on the registry, so `gage-core` publishes first.
- Anything that names the library package says `gage-core` (`cargo test -p
  gage-core`); the lint that checks the library's dependency tree looks it
  up by that name.
