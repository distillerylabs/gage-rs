# Lint rationale

What each static check is for, and what enforces it here. The project began
with a security-linter configuration (`golangci-lint` with `gosec` and
`forbidigo`) in its original implementation. This maps each of those checks
to its Rust equivalent so that nothing it guarded went unguarded when the
language changed.

The gate is `make lint`: `cargo fmt --check`, then
`cargo clippy --workspace --all-targets --locked -- -D warnings`. `-D warnings`
turns every rustc and clippy warning into a failure, so "gates" below means a
failure of `make lint`, which CI and the pre-commit hook both run. Where the
control is a test, it gates through `make test`.

## Linters

| Original check | Rust equivalent | Gates? |
|---|---|---|
| `errcheck` (unchecked errors) | `Result` is `#[must_use]`: rustc's `unused_must_use` flags a dropped one. `let _ = f()` is the visible "I considered this" spelling. | Yes |
| `errcheck` type assertions | Language guarantee: Rust has no unchecked type assertion. `downcast` returns an `Option`/`Result`. | n/a |
| `govet` | rustc's lints plus clippy's `correctness` and `suspicious` groups, both in `clippy::all` (`Cargo.toml` `[workspace.lints.clippy]`). | Yes |
| `ineffassign` | rustc `unused_assignments` and `unused_variables`. | Yes |
| `staticcheck` | clippy's default groups (`clippy::all`). | Yes |
| `unused` | rustc `dead_code` and `unused_*` lints. | Yes |
| `gofmt` | `cargo fmt --check`, the first half of `make lint`. | Yes |
| `forbidigo`: stdin, stdout, exit | `disallowed-methods` in `crates/gage/clippy.toml`: `std::io::stdin`, `std::io::stdout`, `std::io::stderr`, `std::process::exit`. Denied crate-wide in `crates/gage/src/lib.rs`. | Yes |
| `forbidigo`: print functions | `disallowed-macros`: `print!`, `println!`, `eprint!`, `eprintln!`, `dbg!`. | Yes |
| `forbidigo`: writes to a stream | No lint. `write!` into a `String` or a `fmt::Formatter` is how Rust builds text, so a blanket ban would be wrong. A write to a real stream needs `std::io::stdout()` or `stderr()`, which are banned. | Yes, for the real streams |
| `forbidigo`: the scrypt test hooks | Guard tests, not a lint. `only_the_test_hooks_write_the_work_factors` (`crates/gage/src/crypt.rs`) proves nothing else writes the live values; `shipped_work_factor_reaches_a_real_age_file` and `shipped_enrollment_work_factor_reaches_a_real_age_file` prove the shipped numbers. See [ADR-0024](../decisions/0024-scrypt-hook.md). | Yes (`make test`) |
| test-file exclusions | A test opts out locally with `#[allow(...)]`, never crate-wide. `make lint-selftest` proves the purity lint still fires and that a local `#[allow]` is honoured. | Yes |
| `govulncheck` | `cargo-deny` advisories in `deny.toml`, run by `make audit` and `audit.yml`. It also bans yanked crates, wildcard versions, unknown registries and incompatible licences. | Yes (`audit.yml`) |

## Security rules

| `gosec` rule | Rust equivalent | Gates? |
|---|---|---|
| G101 hardcoded credentials | None. Neither clippy nor cargo-deny scans for secrets. The control is review, plus the fact that tests build their keys at run time (`make test` needs no pre-existing keys). | No |
| G103 use of `unsafe` | `unsafe_code = "deny"` in `[workspace.lints.rust]`. The syscall modules opt in locally with `#[allow(unsafe_code)]` (`memlock.rs`, `coredump.rs`, `editor.rs`, `prompter.rs`, `gitrepo/mod.rs`); nothing else may use `unsafe`. `clippy::undocumented_unsafe_blocks` requires a `// SAFETY:` comment on each block. | Yes |
| G104 unhandled errors | `unused_must_use`, as for `errcheck` above. | Yes |
| G204 subprocess with a variable | The purity lint: `std::process::Command::new` is in `crates/gage`'s `disallowed-methods`, and `make lint-selftest` proves it fires. The library never shells out: git is in-process, and tests that spawn child processes opt out locally. The CLI's one non-test spawn is `$EDITOR` (`editor.rs`), the operator's own program ([ADR-0036](../decisions/0036-editor.md)). | Yes (library) |
| G302, G306 file and directory modes | No lint. `atomicfile::write` and `write_new` take the mode as a parameter and set it before the rename; tests assert the resulting mode (`applies_the_requested_mode`). | Yes (`make test`) |
| G304 path from a variable | No lint. The control is validating every untrusted component before a path is built from it: `devicename`, the vault-id validator, host validation for token paths ([ADR-0006](../decisions/0006-vault-id-keying.md), [ADR-0010](../decisions/0010-device-name.md), [ADR-0020](../decisions/0020-remote-auth.md)). | No |
| G602 slice index out of range | Language guarantee: indexing is bounds-checked and a violation panics, never reads out of bounds. | n/a |

G101, G204 (outside the library) and G304 are the rows with no static check
here. They are held by their validators and tests instead.

## Not enabled

- `clippy::let_underscore_must_use`: it flags dropped `Result`s even when
  the drop is deliberate. It was tried and flagged about 100 sites across
  both crates, nearly all deliberate best-effort calls (`writeln!` to the
  terminal, cleanup `remove_file`/`kill`/`wait`, test channel sends).
  Satisfying it means respelling them as `.ok()` or `drop(..)`, which checks
  nothing more.
