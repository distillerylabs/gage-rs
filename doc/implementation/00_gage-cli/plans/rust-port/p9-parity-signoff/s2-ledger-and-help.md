# S2 — Ledger closure and help surface

[← S1](s1-inventory-signoff.md) · [P9 plan](index.md) · [#87](https://github.com/distillerylabs/gage-rs/issues/87) · next: [S3](s3-user-docs.md)

> **Recommended model: Sonnet\*.** Most of this milestone is
> evidence-gathering against a known list. The help-surface test is new,
> but it generalizes nine existing per-phase tests.
>
> ⚑ Run an **Opus review of the help-surface test** (task 3) before calling
> it done. It is the last guard on the command surface, so it must not pass
> vacuously, for example by finding no fixtures on Windows.

## Goal

1. **Close every open ledger row.** No parity-ledger row is left `[ ]` or
   `[~]`, and each closure cites its evidence: a file, a test, a workflow
   or a CI run.
2. **Record the gosec mapping.** Go's `gosec`/golangci checks are mapped to
   their Rust equivalents, or marked not applicable with a reason. M0a
   deferred this to a PR description (ledger #34).
3. **Cover the whole help surface.** One test walks every
   `tests/fixtures/go/help/*.txt` against the command registry, and every
   registered one-shot command back to a fixture. It covers the 8 fixtures
   no test references today, and guards the surface as a whole.

## Depends on

S1. The exceptions file and guard exist, and the M0a boxes point here.

## Design references

- Ledger §1–§4.
- [Q-HELP-RENDER](../open-questions.md#q-help-render): the fixtures are a
  reference for name, aliases, group, arguments and flags, not a byte gate.
- Port plan "Cross-milestone contracts", M0b's command registry
  (`crates/gage-cli/src/registry.rs`).

## Go sources

- Ledger rows:
  - §1: #3, PR #12, #23, #25, #27–#29, #34, #36
  - §2: `bbb24ba`, `30b21df`, and E0 (`0d710d4`)
  - §3: `test-coverage/`
- Go `.golangci.yml` at the pinned SHA (`../gage/.golangci.yml`): the
  enabled linters, `gosec` and its test-file exclusions (G304, G302, G306,
  G602), and the `forbidigo` purity rules.
- Go `.testcoverage.yml`: the total-80 gate.
- Go plan `test-coverage/` (R0, R1): the 86.55% baseline figure, and the
  rule that coverage is raised with honest tests, not by excluding code.
- `tests/fixtures/go/help/`: all 47 fixtures.

## Decisions to make first

1. **The fixture-to-command mapping.**
   - Proposed: strip `.txt` and split on `_`. `vault_set-default` → `vault set-default`.
     `root` → the root command.
   - `-` is part of a name, never a separator.
2. **What "matches" means.** Proposed, per fixture:
   - the command path is registered
   - its aliases equal the fixture's `Aliases:` line
   - its group equals the fixture's group heading in `root.txt`
   - its positional arguments match the fixture's `Usage:` line
   - every flag in the fixture's `Flags:` block exists on the clap
     command, with the same short and long spelling

   Cobra's inherited `-h/--help` and Go's `--version` are ignored where
   clap supplies its own. Descriptions are left to the per-phase tests
   that already check them.
3. **The reverse direction.**
   - Every registered command available one-shot must have a fixture.
   - A Rust-only command or alias would fail the test unless an
     allow-list entry names its `Q-` entry. Proposed: the allow-list starts
     empty.
   - Session-only commands are already tested against the registry
     (`repl_tests`) and are out of this test's scope.
4. **Hoisting the helper.**
   - `fixture_flags` (`crates/gage-cli/src/query_tests.rs`) moves into one
     `#[cfg(test)]` module,
     proposed as `crates/gage-cli/src/fixture_support.rs`.
   - Its one user switches to it, so the two can't drift. The other
     per-phase tests parse inline for descriptions and are left alone.
   - The per-phase tests stay: they also check descriptions and long help.
5. **Coverage.**
   - The gate stays at `COVER_MIN = 80`, equal to Go's.
   - S2 records the measured total next to Go's 86.55% and does not raise
     the gate. Raising it would be a new policy, not parity.

## Tasks

### 1. Ledger closure

For each row, find the evidence, update the status cell, and cite the
evidence in the row:

- [x] §1 #3 / PR #4: `SECURITY.md`, `CODE_OF_CONDUCT.md`,
      `CONTRIBUTING.md`, `.github/ISSUE_TEMPLATE/`, the three workflows.
- [x] §1 PR #12: CLAUDE.md's multi-milestone process section.
- [x] §1 #23 / PR #24: every parent (`vault`, `identity`, `recipient`,
      `recovery`, `git`, `auth`) lists its children. Cite task 3's test,
      which covers all six parents' fixtures.
- [x] §1 #25 / PR #26: Codecov upload in `test.yml` (per-OS flags) and
      `codecov.yml`.
- [x] §1 #27–#29 and §3 `test-coverage/`:
  - Cite the `make cover-check` gate in `test.yml`.
  - Record `make cover`'s current total line coverage beside Go's 86.55%.
  - Go's R0/R1 test lists are inventory rows that S1 already accounted
    for. Say so.
- [x] §1 #34 / PR #35: link task 2's table.
- [x] §1 #36 / PR #37: `scripts/install-pre-commit-hook.sh`, `make install-hooks`.
- [x] §2 `bbb24ba`: Codecov plus `audit.yml` (`make audit`, cargo-deny
      advisories) as the analogue of govulncheck reporting.
- [x] §2 `30b21df`: `LICENSE` (Apache-2.0). The fixture-data cleanup is Go
      only, since gage-rs keeps no Go-written fixture data
      ([Q-COMPAT](../open-questions.md#q-compat)).
- [x] §2 E0 (`0d710d4`): normalize the status cell to `[x]`. Its own text
      already ends `[x]`.
- [x] Sweep: `grep -nE '\[ \]|\[~\]' parity-ledger.md` finds only the status
      legend.

### 2. The gosec and golangci mapping

- [x] A table in this doc, one row per linter enabled in Go's
      `.golangci.yml`, and one per gosec rule Go's config or `#nosec`
      comments mention. Each row gives the Rust equivalent:
  - a clippy lint (name it, and say whether `-D warnings` makes it gate)
  - a cargo-deny check
  - the `disallowed-macros`/`disallowed-methods` purity lint
    (`forbidigo`'s replacement)
  - a language guarantee (e.g. unchecked errors → `#[must_use]` /
    `Result`), with a one-line reason
  - n/a, with the reason
- [x] Wherever a Go check has a cheap clippy equivalent that is not
      enabled, record it as a follow-up issue. (Filed as [#94](https://github.com/distillerylabs/gage-rs/issues/94),
      [#95](https://github.com/distillerylabs/gage-rs/issues/95) and
      [#96](https://github.com/distillerylabs/gage-rs/issues/96); see the
      table below.) Don't enable it in this
      milestone: a new lint is a code change with its own blast radius.
- [x] Tick M0a's gosec box, pointing at the table.

### 3. Whole-surface help test, red first

- [x] Hoist `fixture_flags` into `fixture_support.rs`, and switch its one
      user (`query_tests.rs`) to it. The other nine per-phase tests parse
      their fixtures inline, line by line, for descriptions; that is a
      different shape, so they are left as they are.
- [x] Tests for the fixture parser: the path mapping, the aliases line,
      and flag extraction including short-only and long-only flags.
- [x] `every_go_help_fixture_has_its_registered_command`, which also
      asserts the fixture count is 47. That way the test can't pass
      because the directory went missing.
- [x] `every_one_shot_command_has_a_go_help_fixture`, with the allow-list
      empty.
- [x] Any mismatch the test finds is fixed in the registry (Go wins on
      behavior), or recorded as a new `Q-` entry and allow-listed.
- [x] Proof it bites: remove an alias from one registry entry, see the
      test fail, revert. Rename one fixture locally, see the count check
      fail, revert.
- [x] Opus review ⚑ of this task's tests. No finding let a path, alias,
      group or positional mismatch through. Three were applied: a pinned
      flag count (`FLAG_COUNT = 78`) so the flag check cannot run over
      nothing, the flag check also requiring the same `takes_value` and a
      non-hidden clap arg, and `\r` tolerance in `root_listing`. Left as
      latent: `fixture_positionals` handles only `--long` value flags,
      which no fixture exceeds.

## The gosec and golangci mapping

Go's `.golangci.yml` at the pinned SHA enables the `standard` set
(`errcheck`, `govet`, `ineffassign`, `staticcheck`, `unused`), plus
`forbidigo`, `gosec` and `gofmt`. gage-rs's gate is `make lint`:
`cargo fmt --check`, then `cargo clippy --workspace --all-targets --locked
-- -D warnings`. `-D warnings` turns every rustc and clippy warning into a
failure, so "gates" below means a failure of `make lint`, which CI and the
pre-commit hook both run.

### Linters

| Go check | Rust equivalent | Gates? |
|---|---|---|
| `errcheck` (unchecked errors) | `Result` is `#[must_use]`: rustc's `unused_must_use` flags a dropped one. `let _ = f()` is the visible "I considered this" spelling, like Go's `_ = f.Close()` (`check-blank: false`). | Yes |
| `errcheck` `check-type-assertions` | Language guarantee: Rust has no unchecked type assertion. `downcast` returns an `Option`/`Result`. | n/a |
| `govet` | rustc's lints plus clippy's `correctness` and `suspicious` groups, both in `clippy::all` (`Cargo.toml` `[workspace.lints.clippy]`). | Yes |
| `ineffassign` | rustc `unused_assignments` and `unused_variables`. | Yes |
| `staticcheck` | clippy's default groups (`clippy::all`). | Yes |
| `unused` | rustc `dead_code` and `unused_*` lints. | Yes |
| `gofmt` | `cargo fmt --check`, the first half of `make lint`. | Yes |
| `forbidigo`: `os.Stdin`, `os.Stdout`, `os.Exit` | `disallowed-methods` in `crates/gage/clippy.toml`: `std::io::stdin`, `std::io::stdout`, `std::process::exit`. Denied crate-wide in `crates/gage/src/lib.rs`. | Yes |
| `forbidigo`: `fmt.Print*` | `disallowed-macros`: `print!`, `println!`, `eprint!`, `eprintln!`, `dbg!`. Rust adds `std::io::stderr` and `dbg!`. | Yes |
| `forbidigo`: `fmt.Fprint*` | No lint. `write!` into a `String` or a `fmt::Formatter` is how Rust code builds text, so a blanket ban would be wrong. A write to a real stream needs `std::io::stdout()` or `stderr()`, which are banned. That closes the gap Go's config documents as accepted (`fmt.Fprintln(os.Stdout, ...)` as a bare statement). | Yes, for the real streams |
| `forbidigo`: `SetScryptWorkFactorForTests`, `SetEnrollmentWorkFactorForTests` | Guard tests, not a lint. `only_the_test_hooks_write_the_work_factors` (`crates/gage/src/crypt.rs`) proves nothing else writes the live values; `shipped_work_factor_reaches_a_real_age_file` and `shipped_enrollment_work_factor_reaches_a_real_age_file` prove the shipped numbers. | Yes (`make test`) |
| `forbidigo` and `gosec` test-file exclusions | A test opts out locally with `#[allow(...)]`, never crate-wide (`CLAUDE.md`). `make lint-selftest` proves the purity lint still fires and that a local `#[allow]` is honoured. | Yes |
| `govulncheck` (not a linter; the ledger's §2 row `bbb24ba`) | `cargo-deny` advisories in `deny.toml`, run by `make audit` and `audit.yml`. It also bans yanked crates, wildcard versions, unknown registries and incompatible licences. | Yes (`audit.yml`) |

### gosec rules

The rules Go's config excludes for tests (G304, G302, G306, G602) and the
ones its `#nosec` comments cite (G101, G103, G104, G204, G302, G304).

| gosec rule | Go count | Rust equivalent | Gates? |
|---|---|---|---|
| G101 hardcoded credentials | 1 | None. Neither clippy nor cargo-deny scans for secrets. The control is review, plus the fact that tests build their keys at run time (`make test` needs no pre-existing keys). | No |
| G103 use of `unsafe` | 6 | `unsafe_code = "deny"` in `[workspace.lints.rust]`. The syscall modules opt in locally with `#[allow(unsafe_code)]` (`memlock.rs`, `coredump.rs`, `editor.rs`, `prompter.rs`, `gitrepo/mod.rs`); nothing else may use `unsafe`. | Yes |
| G104 unhandled errors | 2 | `unused_must_use`, as for `errcheck` above. | Yes |
| G204 subprocess with a variable | 7 | The purity lint: `std::process::Command::new` is in `crates/gage`'s `disallowed-methods` (added after this milestone, [#94](https://github.com/distillerylabs/gage-rs/issues/94)), and `make lint-selftest` proves it fires. The library never shells out: `git` is in-process, and the tests that spawn child processes opt out locally. The CLI's one non-test spawn is `$EDITOR` (`editor.rs`), which is the operator's own program, as in Go's G204 suppression. | Yes (library) |
| G302, G306 file and directory modes | 2 | No lint. `atomicfile::write` and `write_new` take the mode as a parameter and set it before the rename; its tests assert the resulting mode (`applies_the_requested_mode`). | Yes (`make test`) |
| G304 path from a variable | 43 | No lint. The control is validating every untrusted component before a path is built from it: `devicename`, the vault-id validator, host validation for token paths (`CLAUDE.md`, "Key invariants"). | No |
| G602 slice index out of range | 0 | Language guarantee: indexing is bounds-checked and a violation panics, never reads out of bounds. | n/a |

G101, G204 and G304 are the three rows where Go has a static check and
Rust had none when this milestone closed. Go suppressed the last two at 50
call sites with a written reason each; Rust has no equivalent suppression
to audit, so those paths are held by their validators and tests instead.
G204 has since gained a library-side lint (#94); the CLI's `$EDITOR` spawn
stays unlinted, as Go's suppression left it.

### Follow-ups: cheap clippy equivalents that are not enabled

Not enabled here. Each is a code change with its own blast radius, so it
gets its own issue.

| Lint | Go check it echoes | Why it is cheap |
|---|---|---|
| `clippy::disallowed_methods` on `std::process::Command::new` in `crates/gage` | G204, and the "no shelling out" invariant | The library has no non-test caller today; tests already `#[allow]` locally for the purity lint. **Enabled in #94.** |
| `clippy::undocumented_unsafe_blocks` | G103: every Go `unsafe` use carries a `#nosec G103 -- reason` | It enforces a `// SAFETY:` comment on each `unsafe` block, the same discipline. **Enabled in #95.** |
| `clippy::let_underscore_must_use` | G104, and `errcheck` with `check-blank: true` | Go's config deliberately sets `check-blank: false`, so this is stricter than parity and is listed only as an option. **Not enabled (#96):** it flagged 101 sites across both crates, nearly all deliberate best-effort calls (`writeln!` to the terminal, cleanup `remove_file`/`kill`/`wait`, test channel sends). Satisfying it means respelling them as `.ok()` or `drop(..)`, which checks nothing more. |

## Definition of done

- No `[ ]` or `[~]` row anywhere in the parity ledger. Every closure cites
  evidence.
- The gosec table is complete, and M0a has no unchecked box.
- Both help-surface tests are green on Linux, macOS and Windows CI, and
  were proven to fail.
- `make lint` and `make test` clean.

**Done.** Every item above holds.

- `make lint`, `make lint-selftest` and `make test` are clean locally.
- On PR #97 at `0b415b0`, the Test (40274877436), Lint (40274877433) and
  Audit (40274877607) workflows all passed.
- The two help-surface tests and the fixture-parser tests ran and passed
  on Linux, macOS and Windows.
- The first CI run (`462d2c8`) passed Test and Lint on all three OSes, but
  Audit failed on two advisories published after S1's green run:
  - a yanked `clipboard-win` 5.4.1, fixed by bumping to 5.4.3
  - RUSTSEC-2026-0387 (`crypto-bigint` 0.5.5, through `age` 0.12.1's
    p256/hpke, which gage does not use). No fix is reachable until `age`
    releases on p256 0.14, so `deny.toml` ignores it. Removing the ignore
    is tracked in [#98](https://github.com/distillerylabs/gage-rs/issues/98).
- Follow-up lint issues: #94, #95, #96.
