# M0a — Toolchain, CI, lint purity

[← P1 plan](index.md) · next: [M0b — Git-crate spike, primitives, registry, help](m0b-primitives.md)

> **Recommended model: Sonnet.** Mechanical setup with every choice already
> made below. No library logic, no security reasoning.

## Goal

The Cargo workspace, the Makefile, three-OS CI, and the enforcement that
keeps the library/CLI split honest from the first line of code. At the end,
`make build` produces a `gage` binary that does nothing useful yet, and
every gate that later milestones rely on (purity lint, dependency ban,
coverage, cargo-deny, offline tests) is live and proven to fire.

## Depends on

Nothing. Cut from `4-p1-foundation`.

## Design references

- [Port plan, "Layering rules" and "Test conventions"](../index.md)
- [CLAUDE.md](../../../../../../CLAUDE.md) "Commands" and "Architecture: the
  library/CLI split is load-bearing"
- Design doc ["Library architecture"](../../../tdds/gage-cli-design.md)

## Go sources

- Go plan `gage-cli-design/m0-scaffolding.md`: the build/test/lint, purity
  lint and CI-matrix bullets. The rest of Go M0 is M0b.
- Go plan `test-coverage/index.md` decisions D3 (gate at 80 total, a floor,
  not a target) and D4 (test scaffolding excluded from the gate).
- Parity ledger rows: #3 / PR #4 (CI), #25 / PR #26 (Codecov), #27–#29 /
  PRs #30–#32 (coverage gate), #34 / PR #35 (lint/security config audit,
  here the clippy lint set and `deny.toml`), #36 / PR #37 (pre-commit hook),
  `bbb24ba` (coverage and vulnerability reporting).
- Go `Makefile`, `.github/workflows/{test,lint,vuln}.yml`,
  `scripts/install-pre-commit-hook.sh` at the pinned SHA.
- Test inventory: no rows. Go tests these properties by CI behavior, not
  test functions; the checks below are the Rust equivalents.

## Decisions to make first

Settled here so they aren't re-derived at the keyboard:

- **Toolchain pin**: the current stable Rust at the start of M0a, which
  must be ≥ 1.89 (`std::fs::File::lock`, [Q-FILE-LOCK](../open-questions.md#q-file-lock)).
  `rust-toolchain.toml` pins the exact version plus `rustfmt`, `clippy` and
  `llvm-tools-preview`.
- **Crate names**: library package `gage` at `crates/gage`; CLI package
  `gage-cli` at `crates/gage-cli` with `[[bin]] name = "gage"`. Both
  `publish = false`. Edition 2024.
- **Dependency-ban check** is a Rust integration test,
  `crates/gage/tests/no_frontend_deps.rs`, not a CI-only script, so it runs
  in `make test` on every platform. It runs `cargo metadata
  --format-version 1 --locked`, walks `gage`'s normal (non-dev)
  dependency closure, and fails on any denylisted crate: `clap*`,
  `rpassword`, `crossterm`, `termion`, `rustyline`, `reedline`, `arboard`,
  `qrcode`, `dialoguer`, `inquire`. `serde_json` is a dev-dependency. The metadata command omits `--offline`: `cargo metadata` needs every lockfile package on disk, which a bare `cargo test` hasn't fetched. `make test` fetches first and sets `CARGO_NET_OFFLINE=true`, which the command inherits, so the suite still can't reach the network where it matters. Build dependencies count; dev-only edges don't.
- **Purity-lint self-test** is `make lint-selftest`, run by the CI lint
  job: it copies a file containing each disallowed macro/method into a
  scratch copy of `crates/gage` and asserts clippy fails on each one. A
  lint that silently stopped firing would otherwise pass forever.
- **Coverage**: `cargo-llvm-cov`, gate on total line coverage at **80**
  (`--fail-under-lines 80`), as Go's D3. Test-support code (`gittest`,
  arriving in M0b) is excluded via `--ignore-filename-regex`, as Go's D4.
  `make cover` writes `lcov.info` and `make cover-check` gates on the
  collected data (`COVER_MIN`, default 80; override on the command line to
  see it fail). `make cover-report` prints the summary, which CI also writes
  to the job summary. CI uploads lcov to Codecov when a `CODECOV_TOKEN`
  secret exists and skips the upload otherwise, so forks and a
  not-yet-configured repo stay green.
- **Version stamping hook**: the Makefile exports `GAGE_VERSION`
  (`git describe --tags --exact-match`, else `dev`) and `GAGE_COMMIT`
  (`git rev-parse --short HEAD`, else `unknown`); `crates/gage-cli/build.rs`
  only declares `rerun-if-env-changed` for both. Reading them is M0b's.
- **Tool pins**: `cargo-deny` 0.20.2 and `cargo-llvm-cov` 0.9.1 are
  installed into `./bin/<tool>-<version>/` by the Makefile, as Go does for
  its tools.
- **Audit policy** (`deny.toml`): vulnerabilities and yanked releases fail;
  `unmaintained = "workspace"` (only direct dependencies; cargo-deny 0.20
  has no warn level for it); licences are an explicit allowlist, starting at
  `Apache-2.0` and `MIT` and extended one reviewed licence at a time as
  dependencies need them; duplicate versions warn; wildcard versions fail
  except between workspace crates; crates.io is the only source.
- **Lint self-test**: `scripts/lint-selftest.sh` copies the workspace to a
  scratch directory with one shared target dir, appends each forbidden item
  to the scratch `lib.rs`, and requires clippy to reject it *with a
  disallowed-item error* (not merely any failure). Eleven probes: the
  untouched crate, the nine items, and a local `#[allow]` in test code.

## Tasks

Tests first, at task granularity.

### 1. Workspace and Makefile

- [x] Root `Cargo.toml` workspace (`resolver = "3"`), `rust-toolchain.toml`,
      `crates/gage` (empty `lib.rs` with crate-level docs) and
      `crates/gage-cli` (a `main.rs` that exits 0), `Cargo.lock` committed.
- [x] Workspace lints in `Cargo.toml`: `unsafe_code = "deny"` at the
      workspace level (Windows/Unix syscall modules opt in locally later),
      clippy `all` as warnings (denied by `-D warnings` in `make lint`).
- [x] `Makefile` targets `build`, `test`, `lint`, `fmt`, `audit`, `cover`,
      `cover-check`, `clean`, `clean-tools`, `install-hooks`, as listed in
      CLAUDE.md. `test` runs `cargo fetch --locked` then
      `CARGO_NET_OFFLINE=true cargo test --locked --workspace`.
- [x] Test: `make build` produces `target/debug/gage` (`gage.exe` on
      Windows) and it exits 0.
- [x] Test: `make test` passes with networking unavailable after the fetch
      (CI runs it after `cargo fetch` with `CARGO_NET_OFFLINE=true`; locally,
      checked once by hand with the network off and noted in the PR).

### 2. Library purity lint

- [x] `crates/gage/clippy.toml`: `disallowed-macros` = `print`, `println`,
      `eprint`, `eprintln`, `dbg`; `disallowed-methods` =
      `std::io::stdin`, `std::io::stdout`, `std::io::stderr`,
      `std::process::exit`. `crates/gage/src/lib.rs` denies
      `clippy::disallowed_macros` and `clippy::disallowed_methods`.
- [x] Test (`make lint-selftest`): each of the nine items, placed alone in a
      scratch copy of the crate, makes clippy fail; the unmodified crate
      passes.
- [x] Test: a `#[cfg(test)]` module using `println!` under a local
      `#[allow(clippy::disallowed_macros)]` passes, so tests can opt out
      locally and never crate-wide.

### 3. Dependency ban

- [x] `crates/gage/tests/no_frontend_deps.rs` as decided above.
- [x] Test: the check passes on the real workspace.
- [x] Test: the walker, fed a hand-built `cargo metadata` JSON fixture where
      `gage` depends transitively on `clap`, reports it with the path that
      pulls it in. Proves the check can fail.
- [x] Test: a denylisted crate reachable only through `gage`'s
      *dev*-dependencies is not reported.

### 4. Audit and lint config

- [x] `deny.toml`: advisories (deny vulnerabilities, warn unmaintained),
      licenses (allowlist compatible with Apache-2.0), bans (deny wildcard
      versions, warn duplicates), sources (crates.io only). `make audit`
      runs it; it needs the network, like Go's `govulncheck`.
- [ ] Record in the PR description which Go `gosec`/golangci checks have a
      Rust equivalent in this set and which don't apply (the ledger's #34
      row asks for that audit).

### 5. Coverage

- [x] `make cover` writes `lcov.info` and prints the summary; `make
      cover-check` fails under 80.
- [x] Test: `make cover-check` fails when the threshold is temporarily set
      above the measured figure (checked once by hand, noted in the PR).

### 6. CI

- [ ] `.github/workflows/test.yml`: matrix `ubuntu-latest`, `macos-latest`,
      `windows-latest`, `fail-fast: false`, natively (no cross-compilation).
      Steps: checkout, toolchain from `rust-toolchain.toml`, cache, `cargo
      fetch --locked`, `make build`, `make cover`, `make cover-check`,
      coverage summary to the job summary, Codecov upload when the secret
      exists. Windows installs GNU make.
- [ ] `.github/workflows/lint.yml`: same matrix (cfg'd platform code is only
      linted natively), `make lint` and `make lint-selftest`.
- [ ] `.github/workflows/audit.yml`: `make audit` on Linux, on pushes, PRs
      and a weekly schedule.
- [x] Actions pinned by commit SHA, `permissions: contents: read`,
      concurrency cancels superseded PR runs and never runs on `main`.

### 7. Developer tooling and docs

- [x] `scripts/install-pre-commit-hook.sh` (port of Go's): installs a hook
      running `make lint`; `--uninstall`, `--force`. `make install-hooks
      ARGS=...`.
- [x] CLAUDE.md "Commands": remove the "nothing below exists until M0a"
      note and correct anything the implementation changed.
- [x] `.gitignore` covers `/bin/` and coverage output.

## Definition of done

Every box above checked, `make lint`, `make lint-selftest` and `make test`
clean locally, and all three workflows green on Linux, macOS and Windows.
The purity lint, the dependency ban and the coverage gate have each been
shown to fail when they should.
