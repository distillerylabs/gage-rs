# D1b — Retire the parity machinery

[← D1a](d1a-format-spec.md) · [D1 plan](index.md) · [#117](https://github.com/distillerylabs/gage-rs/issues/117) · next: [D1c](d1c-docs-process.md)

> **Recommended model: Sonnet.** This milestone is mostly deletion, plus
> one snapshot harness that follows existing test patterns.

## Goal

No test, script or CI step reads Go reference material. gage-rs's own
help output is pinned by snapshots that are captured before anything is
deleted, so it cannot change by accident.

## Depends on

D1a. Its golden vault is the format guard that replaces "follows Go".

## Design references

- Archived register entries:
  - Q-HELP-RENDER: help is rendered by clap from the registry.
  - Q-INVENTORY-GUARD: being retired here.
  - Q-RECIPIENT-PROMPT: why `--yes` is hidden.
  - Q-SCRIPT-PASSPHRASE: why `--script` and `--stdin` exist.
- `crates/gage-cli/src/tests.rs`: the structural registry ↔ clap tests,
  which stay.

## Sources

The [D1 index](index.md#where-things-stand-at-the-start-of-d1) lists every
file that reads `tests/fixtures/go/`. The full list is regenerated with
`grep -rln 'fixtures/go\|help_dir\|go_fixture' crates/ scripts/`.

## Decisions to make first

1. **Snapshot mechanism.**
   - Proposed: hand-rolled, with no new dependency.
   - A test renders each surface and compares it to
     `tests/fixtures/help/<path>.txt`. With `UPDATE_SNAPSHOTS=1` it
     writes the files instead and fails, so an update is never silent.
   - Alternative: `insta`. Rejected for now, because the crate's tests
     have no snapshot dependency and the surfaces are plain text.
2. **Which surfaces to snapshot.** Proposed:
   - `gage --help`
   - `gage help <path>` for every registered one-shot command and
     parent
   - session-mode `help` and `help <cmd>`
   - one usage error per parent

   Each file is named by command path, e.g. `vault_list.txt`, as the Go
   fixtures were.
3. **What happens to assertions that only cite Go.** They are kept and
   re-justified, not deleted:
   - `--script`, `--stdin` and `--yes` are hidden. Script mode is for
     automation, not discovery, and `--yes` is a deliberate footgun
     guard.
   - `-v/--version` is kept as the documented spelling.
   - `all_codes_are_distinct_and_keep_gos_values` is renamed and
     re-commented: exit codes 0–7 are a script-facing contract owned by
     `doc/cli/exit-codes.md`.

## Tasks

### 1. Snapshots first, before any deletion

- [x] `help_snapshots_match`, in `crates/gage-cli/src/help_snapshot_tests.rs`,
      covers the decision-2 surfaces.
- [x] Generate the snapshots on the unmodified tree. Commit them with
      `tests/fixtures/help/README.md` explaining the update switch.
- [x] `help_snapshots_cover_every_registered_command`: every registry
      path has a snapshot file and every snapshot file has a registry
      path, so a new command can't ship without one.
- [x] Prove the guard fails once by hand: change one description, watch
      the test go red, revert. Record that here.
      Done: changed "Create a new vault" to "Create a NEW vault" in
      `registry.rs`; `help_snapshots_match` failed (`root.txt differs`),
      then passed after the revert.

### 2. Delete the Go guards

- [x] Delete `crates/gage-cli/tests/parity_inventory.rs`, `tests/parity/`,
      `tests/fixtures/go/` and `scripts/go-reference/`.
- [x] Delete `crates/gage-cli/src/fixture_support.rs` and
      `help_surface_tests.rs`, and their `mod` lines in `lib.rs`.
- [x] Delete the eleven `*_have_their_go_fixture_flags_and_descriptions`
      tests, plus the parent-fixture checks at `approve_cli_tests.rs:601`,
      `auth_tests.rs:271` and `recovery_cli_tests.rs:1514`. Before
      deleting each one, confirm the snapshots cover everything it
      asserted: flags, aliases, descriptions and long help.
- [x] `script_tests.rs` `the_script_flags_are_hidden_root_flags`: drop
      the Go fixture read, assert against the clap tree, and re-justify
      per decision 3.
- [x] `exitcode.rs`: rename and re-comment the code-values test per
      decision 3.

### 3. QA harness

- [x] `scripts/qa/run-all.sh`: compute command coverage from
      `tests/fixtures/help/root.txt` instead of Go's.
- [x] `scripts/qa/scenarios/11-help-surface.sh`: compare the built
      binary against `tests/fixtures/help/`.
- [x] `scripts/qa/README.md`: update the wording.
- [x] `make qa` is green locally.

### 4. Tooling and templates

- [x] `.github/pull_request_template.md`: replace the two Go checkboxes
      with:
  - "on-disk format change → `format_version` bump, new fixture, ADR"
  - "help snapshot changed → intended, and the reason given"
- [x] Reword the Go-citing comments in `Makefile` (`stamp-check`),
      `.github/workflows/test.yml`, `deny.toml`, `Cargo.toml`,
      `crates/gage/clippy.toml` and `crates/gage-cli/Cargo.toml`.
- [x] `shipped_features.rs`: drop the Go references only.

### 5. Check

- [x] `grep -rn 'fixtures/go\|GO_GAGE_SHA\|go-reference\|parity_inventory'`
      over `crates/ scripts/ .github/ Makefile` is empty.

## Definition of done

- Every box above is ticked.
- `make lint`, `make test` and `make qa` are clean.
- CI is green on all three OSes.
- The snapshots committed in task 1 are unchanged at the end of the
  milestone.
