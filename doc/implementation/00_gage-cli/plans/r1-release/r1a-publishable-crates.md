# R1a: Publishable crates + versioning

[R1 plan](index.md) · [#150](https://github.com/distillerylabs/gage-rs/issues/150) · next: [R1b](r1b-release-pipeline.md)

> **Recommended model: Sonnet.** This milestone is:
>
> - a package rename the compiler and `cargo metadata` check
> - manifest fields
> - a version fallback small enough to cover completely with unit tests
>
> The one thing to watch is `no_frontend_deps.rs`. It looks the library
> up by package name, so after the rename it must find `gage-core`. It
> panics on an unknown name rather than passing vacuously; keep it that
> way.

## Goal

`cargo publish --dry-run -p gage-core` succeeds, and `gage-cli` packages
cleanly. A `gage` binary built any way prints the release version:

- through the Makefile
- by cargo-dist
- by `cargo install gage-cli`

The release and versioning policy is an ADR. Nothing user-visible
changes except the version string itself. The help snapshots and
`format_v2_*` tests stay green and unedited.

## Depends on

The parent branch `149-first-release` (these docs).

## Design references

- [R1 index](index.md).
- `crates/gage-cli/build.rs`, `src/build_env.rs`, `src/version.rs`, and
  `tests/smoke.rs` (`real_binary_version_flag`).
- The Makefile's `GAGE_VERSION`/`GAGE_COMMIT` exports and its
  `build-stamp-check` target (`STAMP_VERSION := v9.9.9-testtag`).
- `crates/gage/tests/no_frontend_deps.rs`, which checks the library's
  dependency tree by package name.
- `rust-toolchain.toml`: the pinned compiler is 1.99.0. The floor 1.89 is
  claimed for `std::fs::File::lock` ([ADR-0003](../../../../decisions/0003-file-lock.md)).
- [doc/format/README.md](../../../../format/README.md): `format_version`
  is the format's own version.

## Where things are

**Package names.** `crates/gage` is package `gage`. These name it:

- `gage = { path = "../gage" }` in `crates/gage-cli/Cargo.toml` (twice:
  the dependency and the `test-support` dev-dependency) and
  `crates/reset-local-state/Cargo.toml`
- `cargo clippy -p gage` in `scripts/lint-selftest.sh:43`
- `cargo test -p gage` in CLAUDE.md and CONTRIBUTING.md
- `offenders(&metadata, "gage")` four times in `no_frontend_deps.rs`,
  plus its fixture's `"name": "gage"`, which stays: that fixture is
  synthetic

**Publish state.** `publish = false`, `version = "0.0.0"`, `license`,
`repository` and `rust-version` are set in `[workspace.package]`. No
crate sets `readme`, `keywords`, `categories`, `homepage`,
`documentation` or `include`. The only README and LICENSE are at the
repo root, outside every package directory.

**Version stamping.**

- `version::resolve_build_info(version, commit)` treats a stamped value
  as authoritative, including an explicit `dev`. Unset or empty falls back
  to `dev`/`unknown`.
- The Makefile always stamps one of these:
  - the exact tag (`v0.1.0`)
  - `dev`
- So today a tagged Makefile build prints `gage v0.1.0 (commit abc1234)`,
  and any other build prints `gage dev (commit unknown)`.

**Packaging hazards checked.**

- Every `include_str!`/`include!` in `src/` and `build.rs` reads a file
  inside its own crate. The two in `src/` are inside test modules.
- `cargo package` copies a `readme`/`license-file` that lives outside the
  package directory, but nothing else.

## Decisions to make first

Proposals, to be settled in review and then moved to "Settled
decisions":

1. **The version string.** The proposal is one shape, `gage 0.1.0
   (commit abc1234)`, with no leading `v`, matching the crate version:
   - The version is always `CARGO_PKG_VERSION`.
   - The Makefile stops stamping the version and stamps only a dev
     marker. An untagged checkout prints `0.1.0-dev`.
   - A tagged checkout whose tag isn't `v$CARGO_PKG_VERSION` fails the
     build loudly, so a mis-tag can't ship a wrong string.
   - `GAGE_VERSION` keeps working as an override (tests use it).

   Rejected alternative: keep the tag (`v0.1.0`). It can't work for
   `cargo install`, which has no tag.
2. **Where a non-Makefile build gets the commit.** `build.rs` tries, in
   order:
   1. `GAGE_COMMIT`
   2. `git.sha1` from `$CARGO_MANIFEST_DIR/.cargo_vcs_info.json`, which
      `cargo package` writes and every crates.io build has
   3. `git rev-parse --short HEAD`, which is allowed in gage-cli's
      `build.rs` (the purity rule binds only `crates/gage`); a missing
      `git` is not an error
   4. `unknown`

   Reading `.cargo_vcs_info.json` goes through a pure function, so it is
   unit-tested in-process.
3. **README and LICENSE in the packages.** The proposal:
   - each published crate gets its own short `README.md` (library
     audience for `gage-core`, install and usage for `gage-cli`)
   - `license-file` is not set; `license = "Apache-2.0"` stays
   - `include` lists the root `LICENSE` explicitly via `../../LICENSE`

   If cargo refuses that path, the fallback is a per-crate copy, guarded
   by a test that it is byte-identical to the root `LICENSE`.
4. **`include` lists.**
   - `gage-core`: `src/**`, `Cargo.toml`, `README.md`, `LICENSE`,
     `clippy.toml`.
   - `gage-cli`: the same, plus `build.rs`.
   - Both drop `tests/`: integration tests need the repo's
     `tests/fixtures/`. The `*_tests.rs` unit modules stay, because
     excluding a declared `mod` file breaks a downstream `cargo test`.
5. **The `test-support` feature on crates.io.** It ships, because gage-cli's
   dev-dependency needs it and cargo resolves features at publish. It
   is documented in ADR-0058 as outside semver, and `gittest` and
   `testsupport` get `#[doc(hidden)]`. `tests/shipped_features.rs`
   already proves the binary doesn't enable it.
6. **Keywords and categories.**
   - `gage-core`: keywords `age`, `git`, `secrets`, `password-manager`,
     `encryption`; categories `cryptography`, `authentication`.
   - `gage-cli`: keywords `age`, `git`, `secrets`, `password-manager`,
     `cli`; categories `command-line-utilities`, `cryptography`.
7. **MSRV job.**
   - A `msrv` job in `lint.yml` on ubuntu only, running
     `cargo +1.89 check --locked -p gage-core -p gage-cli` (not
     `--all-targets`: the promise covers building, not dev-deps).
   - If 1.89 fails because a dependency needs newer, `rust-version` is
     raised to the lowest version that passes. ADR-0058 records the
     number and that ADR-0003's floor still holds.
8. **What ADR-0058 covers.**
   - semver scope: the CLI contract (help layout and flags, exit codes
     0–7, the duration grammar, `--version` shape) and `gage-core`'s
     public API, with 0.x minor bumps allowed to break either
   - `gage-cli`'s library target (`gage_cli::run`) is not a public API
   - `format_version` is independent of crate versions
   - the latest release is the only supported version
   - the crate names and why `gage-core`
   - lockstep versioning: both crates always share the workspace
     version

## Settled decisions

None yet.

## Tasks

### 1. Rename the package

- [ ] Tests first: point `no_frontend_deps.rs`'s four
      `offenders(&metadata, "gage")` calls at `"gage-core"`. They go red
      (panic: package not found).
- [ ] `crates/gage/Cargo.toml`: `name = "gage-core"`, `[lib] name =
      "gage"`. Update the three path dependencies to `gage = { package =
      "gage-core", path = "../gage", version = "0.1.0" }`, keeping
      `features = ["test-support"]` on the dev-dependency. Green.
- [ ] Update `scripts/lint-selftest.sh`, CLAUDE.md and CONTRIBUTING.md
      (`-p gage` → `-p gage-core`). `make lint-selftest` still fires.

### 2. Version and metadata

- [ ] Workspace version `0.1.0`. Move `publish = false` from
      `[workspace.package]` to `reset-local-state` only.
- [ ] Per decisions 3, 4 and 6: per-crate `README.md`, `readme`,
      `keywords`, `categories`, `homepage`, `documentation`, `include`.
- [ ] `cargo package --list -p gage-core` and `-p gage-cli` show exactly
      the intended files; paste both lists in the PR description.
- [ ] `#[doc(hidden)]` on `gittest` and `testsupport` (decision 5).

### 3. Version stamping

- [ ] Tests first in `version.rs`:
      - unset version → `CARGO_PKG_VERSION`
      - the dev marker → `<pkg>-dev`
      - a `.cargo_vcs_info.json` body → its 7-char sha
      - malformed or absent JSON → falls through
      - an override still wins

      Red.
- [ ] Implement decisions 1 and 2 in `build.rs`/`build_env.rs`/`version.rs`
      and the Makefile. Green.
- [ ] Update `make build-stamp-check` (it stamps `v9.9.9-testtag`) to the
      new shape, and add a Makefile check that a tag that isn't
      `v$CARGO_PKG_VERSION` fails.
- [ ] `real_binary_version_flag` stays green unedited.
- [ ] `cargo install --path crates/gage-cli --root <tmp>`, then
      `<tmp>/bin/gage --version`, prints `gage 0.1.0 (commit <sha>)`.

### 4. CI

- [ ] `lint.yml`, ubuntu leg only: `cargo publish --dry-run --locked -p
      gage-core` and `cargo package --list --locked -p gage-cli`.
- [ ] `lint.yml`: the `msrv` job (decision 7), with its toolchain
      installed explicitly and actions SHA-pinned like the rest.

### 5. ADR and green

- [ ] ADR-0058: release and versioning policy (decision 8). Link it from
      `doc/decisions/README.md`.
- [ ] `make lint` and `make test` are clean.
- [ ] `git diff --stat main -- tests/fixtures` is empty.

## Definition of done

- Every box above is ticked, and decisions 1–8 are recorded as settled.
- `cargo publish --dry-run -p gage-core` passes locally and in CI.
- The three build paths (Makefile, plain `cargo build`, `cargo install`)
  print the same `gage 0.1.0 (commit …)` on a tagged checkout.
- `make lint`, `make test` and `make qa` are clean. The help snapshots
  and `format_v2_*` tests are green and unedited.
- CI is green on Linux, macOS and Windows, and the MSRV job is green.
