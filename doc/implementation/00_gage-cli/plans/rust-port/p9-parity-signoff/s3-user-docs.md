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

- [ ] `command_reference_names_every_registered_command`. It walks the
      registry through S2's support module, and reads
      `doc/cli/command-reference.md`. It fails listing each missing path
      or alias, and is red until task 2.

### 2. New pages

- [ ] `doc/cli/command-reference.md`, ported from Go's. The guard goes
      green.
- [ ] `doc/cli/getting-started.md`, ported from Go's: install from source
      (`make build`), `init` with the recovery key, a first entry, `clone`
      onto a second device with enrollment, and `sync`.
- [ ] Run each getting-started example against a debug build in a scratch
      `$HOME`, and confirm it behaves as written. This is a manual check.
      Record it here.
- [ ] Proof the guard bites: delete one command's line, see it fail,
      revert.

### 3. Cross-check of existing pages

One box per page. Diff its section headings against Go's page, and for
each Go section that is missing here, either add it or record the reason
in this doc:

- [ ] `README.md` (the `doc/cli/` index, which lists every page)
- [ ] `vaults.md`
- [ ] `configuration.md`
- [ ] `exit-codes.md`
- [ ] `entries.md`
- [ ] `addressing-entries.md`
- [ ] `session-mode.md`
- [ ] `sync.md`
- [ ] `git-and-auth.md`
- [ ] `identities-and-recipients.md`
- [ ] `sharing.md`
- [ ] `enrollment.md`

### 4. READMEs and close-out

- [ ] Top-level `README.md` per decision 3.
- [ ] `CONTRIBUTING.md`: check that it doesn't describe the port as
      unfinished in ways P9 made false.
- [ ] Ledger §6: every row `[x]`.
- [ ] Port plan `index.md`: P9 row `[x]` with plan link and #85. The
      "Interim gaps in `init`" paragraph is reworded in the past tense.
- [ ] P9 `index.md`: S1–S3 `[x]`.

## Definition of done

- `doc/cli/getting-started.md` and `doc/cli/command-reference.md` exist and
  describe gage-rs. The getting-started walk-through was run by hand.
- Every existing page has its cross-check box ticked, with every gap
  filled or explained.
- `command_reference_names_every_registered_command` is green on Linux,
  macOS and Windows CI, and was proven to fail.
- The ledger has no open row, and the port plan's P9 row is `[x]`.
- `make lint` and `make test` clean.
