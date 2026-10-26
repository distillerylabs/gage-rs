# S3 — User docs final pass

[← S2](s2-ledger-and-help.md) · [P9 plan](index.md) · [#88](https://github.com/distillerylabs/gage-rs/issues/88) · next: retiring Go is a decision for after P9 (port plan, "Deferred")

> **Recommended model: Sonnet.** This milestone is docs, plus one small
> test that reuses S2's registry walk.

## Goal

The user docs describe gage-rs as it ships at the end of the port:

- `doc/cli/` gains `getting-started.md` and `command-reference.md`, ported
  from Go's.
- Every existing `doc/cli/` page is checked section by section against
  Go's, and any gap is filled or justified.
- `doc/cli/README.md` indexes every page.
- The top-level README states the post-P9 status
  ([Q-README-STATUS](../open-questions.md#q-readme-status)).
- A test keeps `command-reference.md` naming every registered command.

Then P9 closes: ledger §6 rows `[x]`, and the port plan's P9 row `[x]`.

## Depends on

S2: the help-surface test, and its fixture and registry support, which
this milestone's test reuses. The ledger otherwise closed.

## Design references

- Ledger §6, and the P8 index's "Not claimed" note: Go's `getting-started.md`
  and `command-reference.md` mention the recovery key.
- CLAUDE.md "Compatibility and parity": bytes and layout are gage-rs's own,
  behavior follows Go.
- Every deliberate deviation in [open-questions.md](../open-questions.md)
  that a user can observe, e.g. [Q-SSH](../open-questions.md#q-ssh). A page
  describing that behavior links it.

## Go sources

- `../gage/doc/cli/` at the pinned SHA: all 14 pages.
- `../gage/README.md`, for what a top-level README covers. gage-rs's says
  less and links more.

## Decisions to make first

1. **How faithful the ported pages are.**
   - Proposed: port the structure and the content.
   - Rewrite anything that differs in gage-rs: SSH remotes, help layout,
     paths printed in examples. Link the `Q-` entry for each difference.
   - Don't reproduce Cobra's help output in examples. Show gage-rs's.
2. **`command-reference.md`'s source.**
   - Proposed: hand-written, like Go's, grouped as the registry groups
     commands.
   - The guard test checks only that every registered command path (and
     every alias) appears as a heading or table entry.
   - Generating the page from the registry was considered and rejected:
     Go's page carries prose the registry doesn't.
3. **The README wording.** Per
   [Q-README-STATUS](../open-questions.md#q-readme-status):
   - gage-rs reaches behavioral parity with Go gage at `64b6cf0`
   - it has not been released or independently audited
   - use Go gage for real secrets until a release decision is made
   - link `doc/cli/`, the ledger, and the open-questions register for
     deliberate deviations

## Tasks

### 1. Command-reference guard, red first

- [x] `command_reference_names_every_registered_command`
      (`crates/gage-cli/src/command_reference_tests.rs`). It walks the
      registry and reads `doc/cli/command-reference.md`. It fails listing
      each missing path or alias, and was red until task 2. A second test,
      `names_matches_a_command_or_its_arguments_not_a_prefix`, pins the
      matching rule (`vault` does not stand in for `vault list`).
      Decision 2 said "through S2's support module", but the registry is
      all it needs: S2's `fixture_support` reads Go's help fixtures, which
      this test has no use for.

### 2. New pages

- [x] `doc/cli/command-reference.md`, ported from Go's. The guard goes
      green.
- [x] `doc/cli/getting-started.md`, ported from Go's: build from source
      (`make build`), `init` with the recovery key, a first entry, `clone`
      onto a second device with enrollment, and `sync`.
- [x] Run each getting-started example against a debug build in a scratch
      `$HOME`, and confirm it behaves as written. **Run 2026-10-25**, with
      `expect` driving a pty (two scratch homes, a local bare repo as the
      remote). It found, and the page now reflects:
      - `insert` prompts `Value for "<title>":`, not Go's `Enter value:`.
      - `ls` prints plain rows. The header is `ls -H`.
      - `init` asks for the last six characters of the recovery key before
        it finishes.
      - The unlocked session prompt is `personal🔓 gage>`, with no
        brackets.
      - Two devices need different `--device` names: a clone on the same
        hostname is refused. The walk-through passes `--device laptop-2`.
      - After `recipient approve` on the first device and `sync` on the
        second, `ls` on the second device lists the entry.
- [x] Proof the guard bites: deleted the `rm` row and the "(alias `grep`)"
      text from `command-reference.md`. The test failed with
      `does not name: {"grep", "rm"}`. Reverted; green again.

### 3. Cross-check of existing pages

One box per page. Diff its section headings against Go's page, and for
each Go section that is missing here, either add it or record the reason
in this doc:

- [x] `README.md` (the `doc/cli/` index, which lists every page). Added the
      two new pages and Go's "Conventions used in these docs" section, plus
      a convention that example output shows gage-rs's wording.
- [x] `vaults.md`. Added "Cloning an existing vault" and "Selecting which
      vault a command targets". Dropped a stale "Status" note that said
      sync arrives in a later phase. Rust-only sections (identity file,
      recovery key, `vault info` refusals, remote) are gage-rs additions.
- [x] `configuration.md`. Added the `pubkey` line to the example and the
      paragraph on what it is for, replacing "Later phases add `pubkey`".
      "Where things live" and "Inside a vault" differ from Go's headings in
      wording only.
- [x] `exit-codes.md`. No heading differences.
- [x] `entries.md`. Added "The entry format". Go's per-task headings
      (creating, reading, editing, deleting, listing, history) are gage-rs's
      per-command headings (`gage insert`, `gage show`, ...), same content.
      Dropped the stale "Status" note.
- [x] `addressing-entries.md`. Go's "One-shot vs. session mode" is gage-rs's
      "An ambiguous query" (exit `4` one-shot, picker in a session). Dropped
      a stale note that `mv` and `cp` arrive later.
- [x] `session-mode.md`. Heading differences are wording only. Dropped a
      stale "Status" note that sync arrives with phase 4.
- [x] `sync.md`. Go's "resolving a divergence" is gage-rs's "Resolving an
      entry conflict" and "With nobody to ask". Wording only.
- [x] `git-and-auth.md`. Wording only ("Giving a vault a remote", "What gage
      doesn't model"). SSH is documented as best-effort
      ([Q-SSH](../open-questions.md#q-ssh)).
- [x] `identities-and-recipients.md`. No Go sections missing. gage-rs adds
      "Checking it" and "Sharing an entry with someone else".
- [x] `sharing.md`. No heading differences.
- [x] `enrollment.md`. No heading differences.

### 4. READMEs and close-out

- [x] Top-level `README.md` per decision 3.
- [x] `CONTRIBUTING.md`: its opening sentence called the port plan the
      place for contributions "right now". Reworded: the plan is complete,
      and parity gaps, bugs and platform problems are what's wanted.
- [x] Ledger §6: every row `[x]`.
- [x] Port plan `index.md`: P9 row `[x]` with plan link and #85. The
      "Interim gaps in `init`" paragraph is reworded in the past tense.
- [x] P9 `index.md`: S1–S3 `[x]`.

## Definition of done

- `doc/cli/getting-started.md` and `doc/cli/command-reference.md` exist and
  describe gage-rs. The getting-started walk-through was run by hand.
- Every existing page has its cross-check box ticked, with every gap
  filled or explained.
- `command_reference_names_every_registered_command` is green on Linux,
  macOS and Windows CI, and was proven to fail.
- The ledger has no open row, and the port plan's P9 row is `[x]`.
- `make lint` and `make test` clean.
