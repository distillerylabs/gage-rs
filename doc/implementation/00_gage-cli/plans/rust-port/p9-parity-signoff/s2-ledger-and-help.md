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
   - `fixture_flags` (`crates/gage-cli/src/query_tests.rs`) and its
     siblings in the per-phase tests move into one `#[cfg(test)]` module,
     proposed as `crates/gage-cli/src/fixture_support.rs`.
   - The per-phase tests switch to it, so the two can't drift.
   - The per-phase tests stay: they also check descriptions and long help.
5. **Coverage.**
   - The gate stays at `COVER_MIN = 80`, equal to Go's.
   - S2 records the measured total next to Go's 86.55% and does not raise
     the gate. Raising it would be a new policy, not parity.

## Tasks

### 1. Ledger closure

For each row, find the evidence, update the status cell, and cite the
evidence in the row:

- [ ] §1 #3 / PR #4: `SECURITY.md`, `CODE_OF_CONDUCT.md`,
      `CONTRIBUTING.md`, `.github/ISSUE_TEMPLATE/`, the three workflows.
- [ ] §1 PR #12: CLAUDE.md's multi-milestone process section.
- [ ] §1 #23 / PR #24: every parent (`vault`, `identity`, `recipient`,
      `recovery`, `git`, `auth`) lists its children. Cite task 3's test,
      which covers all six parents' fixtures.
- [ ] §1 #25 / PR #26: Codecov upload in `test.yml` (per-OS flags) and
      `codecov.yml`.
- [ ] §1 #27–#29 and §3 `test-coverage/`:
  - Cite the `make cover-check` gate in `test.yml`.
  - Record `make cover`'s current total line coverage beside Go's 86.55%.
  - Go's R0/R1 test lists are inventory rows that S1 already accounted
    for. Say so.
- [ ] §1 #34 / PR #35: link task 2's table.
- [ ] §1 #36 / PR #37: `scripts/install-pre-commit-hook.sh`, `make install-hooks`.
- [ ] §2 `bbb24ba`: Codecov plus `audit.yml` (`make audit`, cargo-deny
      advisories) as the analogue of govulncheck reporting.
- [ ] §2 `30b21df`: `LICENSE` (Apache-2.0). The fixture-data cleanup is Go
      only, since gage-rs keeps no Go-written fixture data
      ([Q-COMPAT](../open-questions.md#q-compat)).
- [ ] §2 E0 (`0d710d4`): normalize the status cell to `[x]`. Its own text
      already ends `[x]`.
- [ ] Sweep: `grep -nE '\[ \]|\[~\]' parity-ledger.md` finds only the status
      legend.

### 2. The gosec and golangci mapping

- [ ] A table in this doc, one row per linter enabled in Go's
      `.golangci.yml`, and one per gosec rule Go's config or `#nosec`
      comments mention. Each row gives the Rust equivalent:
  - a clippy lint (name it, and say whether `-D warnings` makes it gate)
  - a cargo-deny check
  - the `disallowed-macros`/`disallowed-methods` purity lint
    (`forbidigo`'s replacement)
  - a language guarantee (e.g. unchecked errors → `#[must_use]` /
    `Result`), with a one-line reason
  - n/a, with the reason
- [ ] Wherever a Go check has a cheap clippy equivalent that is not
      enabled, record it as a follow-up issue. Don't enable it in this
      milestone: a new lint is a code change with its own blast radius.
- [ ] Tick M0a's gosec box, pointing at the table.

### 3. Whole-surface help test, red first

- [ ] Hoist `fixture_flags` and its siblings into `fixture_support.rs`,
      and switch the per-phase tests to it. Behavior is unchanged, so they
      stay green.
- [ ] Tests for the fixture parser: the path mapping, the aliases line,
      and flag extraction including short-only and long-only flags.
- [ ] `every_go_help_fixture_has_its_registered_command`, which also
      asserts the fixture count is 47. That way the test can't pass
      because the directory went missing.
- [ ] `every_one_shot_command_has_a_go_help_fixture`, with the allow-list
      empty.
- [ ] Any mismatch the test finds is fixed in the registry (Go wins on
      behavior), or recorded as a new `Q-` entry and allow-listed.
- [ ] Proof it bites: remove an alias from one registry entry, see the
      test fail, revert. Rename one fixture locally, see the count check
      fail, revert.
- [ ] Opus review ⚑ of this task's tests.

## Definition of done

- No `[ ]` or `[~]` row anywhere in the parity ledger. Every closure cites
  evidence.
- The gosec table is complete, and M0a has no unchecked box.
- Both help-surface tests are green on Linux, macOS and Windows CI, and
  were proven to fail.
- `make lint` and `make test` clean.
