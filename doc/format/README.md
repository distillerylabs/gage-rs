# gage's on-disk format

gage-rs's on-disk format is its own contract, specified here and guarded by
tests. It does not follow any other program's format.

| Version | Spec | Golden fixture |
|---|---|---|
| 2 (current) | [v2.md](v2.md) | [`tests/fixtures/format-v2/`](../../tests/fixtures/format-v2/README.md) |

The version is `[vault].format_version` in each vault's
`.gage/config.toml`. A reader refuses any version it doesn't implement.

## The rule

A change to any artifact in the current spec is a **format change**. That
covers adding, removing, renaming or retyping a field, changing a file's
name or location, changing what a reader accepts or refuses, and changing
a commit-message shape. A format change needs all four of these, in the
same change:

1. **A `format_version` bump** (`CURRENT_FORMAT_VERSION` in
   `crates/gage/src/vaultconfig/mod.rs`).
2. **A new spec, `v<N>.md`**, beside the old one. The old spec is not
   edited, except to link the new one.
3. **A new fixture, `tests/fixtures/format-v<N>/`**, written once by a
   generator kept in the tree, and its own `format_v<N>_*` tests.
4. **An ADR** that records the change and its migration.

Some things are never format changes: help text, error wording, exit
codes, and the writer's choice of TOML or YAML whitespace, quoting or
scalar style. [v2.md](v2.md) lists them.

## Old fixtures stay

Every committed fixture stays readable, or migratable, forever:

- `format-v2/` is never regenerated or edited. `format_v2_fixture_is_byte_stable`
  fails on any change to it.
- After a bump, the old fixture's tests either keep reading it as they
  are, or are replaced by tests that migrate it and read the result.
  Either way, the old vault still works.
- The `format_v2_*` tests may not be edited to make a change pass. A
  change that needs them edited is a format change, and takes the four
  steps above.

## Where the tests are

- `crates/gage/src/format_v2_tests.rs` covers the library. Every schema
  parses from the committed bytes and round-trips, and unknown entry keys
  survive. Readers refuse what the spec refuses. The commit history has
  the specified shape. A vault created today has the fixture's layout.
  The fixture is byte-stable.
- `crates/gage-cli/src/format_v2_tests.rs` covers the CLI, end to end and
  in process: `ls`, `show`, `cat`, `recipient verify` and `recipient
  pending` against the installed fixture, plus the global config's
  `[shell]` durations. It also holds the `#[ignore]`d generator that
  wrote the fixture.
- `gage::testsupport::format_v2` holds the fixture's expected contents
  and the helper that installs a copy of it into a temp directory.
