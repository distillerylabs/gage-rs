# Divergence from Go gage — plan

Roadmap [#114](https://github.com/distillerylabs/gage-rs/issues/114) ·
follows the [Rust port](../../../../archive/rust-port/index.md) (P1–P9, closed)

The port reached behavioral parity with Go gage at `64b6cf0`. Go gage is
being deprecated, and development continues only here. This plan turns
gage-rs from a port that follows a spec into the reference implementation.

Status legend: `[ ]` not started, `[~]` in progress, `[x]` done.

---

## Why

The repo is still built to follow Go:

- **Process.** CLAUDE.md says "Go wins on behavior". Phase planning starts
  from the parity ledger and Go's test inventory, and every milestone doc
  has a "Go sources" section.
- **Guards.** `crates/gage-cli/tests/parity_inventory.rs` requires each of
  1,271 Go test names to match a Rust `#[test]` once both are normalized,
  or to have a written exception. So renaming a test fails the build.
  Thirteen test files and the QA harness compare the CLI to Cobra help
  captured from the Go binary.
- **Code.** The library mirrors Go's `internal/gage/*` package by package:
  - about 35 flat `pub mod`s
  - `String` fields where `""` means absent, and string constants used as
    enums (`TYPE_GIT`, `METHOD_PASSPHRASE`)
  - vault ids as bare `String`s, re-validated at every path builder
  - `Identity::new -> (Identity, Option<io::Error>)`
  - library errors that hard-code `gage:` and pick CLI exit codes
    (`HasCode`)
  - test hooks stored as fields on the production `Vault`
  - three string-keyed command matches in `gage-cli`
- **References.** About 1,240 `// Go:` provenance comments, and more than
  130 `Q-` anchors into a register about to be archived. Some decisions
  cited from code are defined only in Go's register.

Unless all of this is removed, every later change gets judged against Go,
and Go's patterns keep spreading.

## Resolved decisions

These are recorded here and become ADRs in D1c. They are not added to
the port's `open-questions.md`, which D1c archives.

- **On-disk format.** v2 is frozen as gage-rs's own format. The spec
  lives in `doc/format/v2.md` (D1a). It changes only through a
  `format_version` bump plus a migration ADR. Nothing is promised about
  vaults Go wrote.
- **Port-era docs.**
  - `plans/rust-port/` is archived and frozen.
  - The TDDs become gage-rs design docs with the Go references edited
    out, keeping the rationale.
  - A new ADR log replaces the `Q-` register. It carries forward the
    decisions that still bind, including those defined only in Go's
    register.
- **Code scope.** Retire the guards, then fix the API shape: types,
  errors, module surface and test seams. User-visible CLI behavior stays
  the same, and is now gage-rs's contract rather than Go's:
  - help layout and flags, including `-v/--version` and the hidden
    `--script/--stdin/--yes`
  - exit codes 0–7
  - the duration grammar and spelling
- **Release / 1.0.** A separate effort, later. The first release,
  0.1.0, is phase R1 ([plan](../r1-release/index.md)); 1.0 is still later.

## What stays

- the library/CLI purity lint
- typed `thiserror` errors
- the `Prompter`/`RemoteSyncer`/`Locker` seams
- process-level `vaultlock` tests
- validation of vault ids and device names
- scrypt 2^19 and its guard tests
- 3-OS CI and the coverage gate
- test-first milestones
- the command registry as the single source for one-shot and session help

## Phases

Rolling, as in the port: each phase's parent issue and sub-issues are
filed when the phase starts, so later phases benefit from what earlier
ones learn.

| Phase | Milestones | Status | Model | Depends on |
|---|---|---|---|---|
| D1 Independence ([plan](d1-independence/index.md), [#115](https://github.com/distillerylabs/gage-rs/issues/115)) | D1a format v2 spec + golden vault · D1b retire parity machinery · D1c docs archive, re-home, process | `[x]` | **Opus** D1a; Sonnet D1b/D1c | — |
| D2 Source sweep ([plan](d2-source-sweep/index.md), [#124](https://github.com/distillerylabs/gage-rs/issues/124)) | D2a `crates/gage` · D2b `crates/gage-cli` | `[x]` | Sonnet | D1 |
| D3 Types and errors ([plan](d3-types-errors/index.md), [#131](https://github.com/distillerylabs/gage-rs/issues/131)) | D3a `VaultId`, enums, `Option` config · D3b error/frontend boundary · D3c small Go-isms | `[x]` | **Opus** D3a/D3b; Sonnet D3c | D2 |
| D4 API surface ([plan](d4-api-surface/index.md), [#140](https://github.com/distillerylabs/gage-rs/issues/140)) | D4a module hierarchy + re-exports · D4b test seams off production types · D4c typed CLI dispatch | `[x]` | **Opus** D4b; Sonnet D4a/D4c | D3 |
| R1 First release 0.1.0 ([plan](../r1-release/index.md), [#149](https://github.com/distillerylabs/gage-rs/issues/149)) | R1a publishable crates + versioning · R1b release pipeline · R1c docs, pre-flight, cut | `[x]` | **Opus** R1b; Sonnet R1a/R1c | D4 |

**Order.**

- **D1** removes the constraint.
  - Its golden vault must exist before any serde type changes.
  - Its help snapshots must exist before anything in the CLI moves.
- **D2** goes before D3/D4 so their diffs don't carry thousands of
  comment-only lines.
- **D3** comes before **D4** because the public surface is curated once
  the types it exposes have settled.

### Scope of the later phases (refined when each is filed)

- **D2 Source sweep.**
  - Remove the `// Go:`, `TestXxx`, `.go`, milestone (`M9b`, `RK3`, `P5`)
    and archived `Q-` anchors.
  - Keep comments that explain *why*, rewritten without Go framing, and
    point them to an ADR where the decision still binds.
  - Rename the tests the inventory guard forced, e.g.
    `for_script_picks_the_source_in_go_s_order` and
    `normalize_lowercases_like_go`.
  - Collapse `go_duration_string` (`shell.rs`) and `go_duration`
    (`repl.rs`), which round differently, into one duration module.
    Output stays unchanged.
  - Out of scope, for a later docs pass: the milestone and `Q-`
    references in `doc/design/`, and `doc/format/v2.md`'s
    "Go's `time.ParseDuration`".
- **D3a.**
  - A `VaultId` newtype (validated `TryFrom<&str>`, `serde(try_from)`),
    taken by `paths`, `vaultlock`, `trustcache` and `identityfile`.
  - `VaultType` and `UnlockMethod` enums.
  - Config fields that use an empty `String` for "absent" become
    `Option`.
  - The golden vault proves the bytes are unchanged.
- **D3b.**
  - Library errors stop naming the binary or the package; the CLI adds
    `gage:` once, and `prefix_once` goes.
  - `HasCode` becomes a frontend-neutral error kind, and the numeric
    exit mapping moves to `gage-cli` (numbers unchanged).
  - `downcast_ref` used as Go's `errors.Is` gives way to matching enums.
  - `session::OpenError`'s `Box<dyn Error>` becomes a typed enum.
  - The `gage: …` commit messages are on-disk format and do not change.
- **D3c.**
  - `Identity::new` returns an identity plus a page-lock status value,
    not a tuple.
  - `session::HeldVault` becomes a state enum.
  - `marshal`/`unmarshal` are renamed, and `config` settles on one
    `load`/`save` naming.
- **D4a.**
  - Group the modules (storage, crypto, sync, trust, entry, enrollment,
    recovery).
  - Re-export the frontend-facing types at the crate root.
  - Default to `pub(crate)`.
  - Merge `recipient` and `recipients`.
  - Split `Vault`'s 17-file impl where focused handles read better.
- **D4b.**
  - Move the test hooks off `Vault` into a `#[cfg(test)]` seam bundle, so
    `Vault` stops deriving `PartialEq` through always-true impls.
  - Add a `Clock` trait in place of `Option<Box<dyn Fn() -> Instant>>`.
  - Drop `Any` from `Locker`.
  - The scrypt work-factor global and its guard tests stay as they are.
- **D4c.**
  - A typed command enum derived from the registry replaces the three
    string-keyed matches and the "not implemented yet" path.
  - The help snapshots prove the output is unchanged.

## Process notes

- **Same process as the port.** The rules are in CLAUDE.md: a parent
  issue plus sub-issues per phase, stacked branches, tests first at task
  granularity, and the milestone doc's test list as the definition of
  done.
- **Planning a phase** starts from this file's scope note for it. It no
  longer starts from the ledger or the test inventory.
- **The proof that refactors preserve behavior** is D1's golden-vault
  tests and help snapshots. From D2 on, a milestone that changes either
  one has to record why in an ADR. "Updated the snapshot" is not a
  reason.
- **Outside this repo.** Post a deprecation notice in the Go gage README.
  Archive the Go repo only after D1c lands, because D1c harvests Go's
  register, README and security model, and until then the Go repo holds
  the only copies.
