# P9 Parity sign-off — implementation plan

[← port plan](../index.md) · Parent issue
[#85](https://github.com/distillerylabs/gage-rs/issues/85) · Roadmap
[#2](https://github.com/distillerylabs/gage-rs/issues/2) · previous:
[P8 Recovery key](../p8-recovery-key/index.md)

P1–P8 have ported every milestone Go gage shipped at the pinned SHA
([`64b6cf0`](https://github.com/distillerylabs/gage/commit/64b6cf078f3e756efb9d868cc1518dc1d9060748)).
P9 is the port plan's exit criterion, made checkable:

- every [parity-ledger](../parity-ledger.md) row closed
- every row of `tests/fixtures/go/test-inventory.tsv` ported or excused
- user docs complete

P9 also leaves guards in `make test`, so that a later rename or deletion
can't quietly reopen what it closed.

P9 adds no behavior. It may port a test that turns out to be missing, and
then fix whatever that test finds. Its outputs are evidence, guards and docs.

This plan does not restate the shared ground. The layering rules, test
conventions, crate choices and cross-milestone contracts are in the
[port plan](../index.md). Decisions are recorded in
[open-questions.md](../open-questions.md).

---

## Where things stand at the start of P9

Measured on `main` at `6552930` (P8 merged).

**Test inventory.** All 1,271 Go test functions are named in at least one
milestone doc. 1,176 match a Rust `#[test]` fn by name once both are
normalized: the Go name loses `Test` and is lowercased, and the Rust name
loses `_`. The other **95** are:

| Phase | Not name-matched |
|---|---|
| P1 | 24 |
| P2 | 43 |
| P3 | 7 |
| P4 | 6 |
| P5 | 9 |
| P6 | 5 |
| P7 | 0 |
| P8 | 1 |

They are ports under another name (e.g. Go's `atomicfile.WriteFile` tests
against Rust's `write_atomic`), split halves, `TestMain`s, and written
excuses such as
`TestNilPrompterRefusesRatherThanPanicking` ([Q-CONFLICT-PROMPTER](../open-questions.md#q-conflict-prompter))
and `TestTerminalPrompterOfWalksDecoratorChains` ([Q-SCRIPT-PASSPHRASE](../open-questions.md#q-script-passphrase)).
Nothing checks any of this mechanically.

**Parity ledger.** These rows are not closed:

- **Shipped by M0a, never marked:** §1 #3 `[~]`, PR #12 `[~]`, #25 `[ ]`,
  #27–#29 `[ ]`, #34 `[ ]`, #36 `[ ]`; §2 `bbb24ba` `[ ]`, `30b21df` `[~]`;
  §3 `test-coverage/` `[ ]`.
- **#23** (parent-command help) is `[~]`, though every parent now exists.
- **E0** is `[~]` in the status column, while its own text ends `[x]`.
- **§6:** `README.md`, `getting-started.md` and `command-reference.md` read
  "final pass in P9".

**Milestone docs.** Five boxes are unchecked:

- M0a has four: the gosec mapping, which was recorded only in a PR
  description, and three CI workflow items.
- M8a1 has one: the cold CI build.

**Help fixtures.** 8 of the 47 are referenced by no test: `git`,
`git_set-remote`, `identity`, `rm`, `vault`, `vault_info`, `vault_list`,
`vault_set-default`. Per-phase `*_have_their_go_fixture_flags_and_descriptions`
tests cover the other 39.

**User docs.** `doc/cli/` lacks Go's `getting-started.md` and
`command-reference.md`. The top-level README says "in progress, not usable
yet".

**Open questions.** Every entry is resolved.

## Resolved decisions

- **Three milestones, split by kind of evidence**
  ([Q-P9-SPLIT](../open-questions.md#q-p9-split)): tests, then the ledger
  and help surface, then docs. Docs go last, so they describe the final
  state.
- **The inventory guard is name match plus an exceptions file**
  ([Q-INVENTORY-GUARD](../open-questions.md#q-inventory-guard)). A row
  passes when its normalized name matches a Rust `#[test]`, or when it has
  a row in `tests/parity/inventory-exceptions.tsv`. The guard is a test run
  by `make test`.
- **The README's post-P9 status**
  ([Q-README-STATUS](../open-questions.md#q-readme-status)) is "behavioral
  parity with Go gage at the pinned SHA; not released or audited". Go
  remains the recommendation for real secrets until a release decision.
  Retiring Go stays out of scope (port plan, "Deferred").

## Milestones

| Milestone | Status | Model | Depends on |
|---|---|---|---|
| [S1 Inventory sign-off](s1-inventory-signoff.md) ([#86](https://github.com/distillerylabs/gage-rs/issues/86)) | `[~]` | **Opus** ⚑ | parent branch |
| [S2 Ledger closure + help surface](s2-ledger-and-help.md) ([#87](https://github.com/distillerylabs/gage-rs/issues/87)) | `[ ]` | Sonnet\* (⚑ Opus review of the help-surface test) | S1 |
| [S3 User docs final pass](s3-user-docs.md) ([#88](https://github.com/distillerylabs/gage-rs/issues/88)) | `[ ]` | Sonnet | S2 |

**S1 is Opus** because it is the check against quiet scope narrowing. Each
of the 95 rows needs someone to read the Go test and the Rust test the doc
names, and judge whether they are equivalent. That judgement is the point
of the milestone.

S2 is mostly evidence-gathering, plus one test over a surface the existing
per-phase tests already exercise. S3 is docs.

## Inventory accounting

P9 owns no inventory rows. S1 audits the 95 rows that don't name-match,
and its guard test covers all 1,271. A row S1 finds genuinely unported is
ported in S1 and recorded against its original phase in the ledger's §5.

Obligations earlier phases left to P9:

| From | Obligation | Claimed by |
|---|---|---|
| P1 · M0a | Four unchecked boxes: the gosec mapping and three CI workflow items | S1 ticks the CI items; S2 writes the gosec mapping |
| P4 · M8a1 | Cold CI build on all three OSes | S1 |
| Ledger §1–§3 | The rows listed above | S2 |
| Ledger §6, P8 index "Not claimed" | `getting-started.md`, `command-reference.md` (Go's versions mention the recovery key), READMEs | S3 |

## Process notes

- Branches stack: `main` <- `85-p9-parity-signoff` (these docs) <- S1 <- S2
  <- S3. Each milestone branch is cut from the previous one when the user
  says to start it.
- Tests come first, at task granularity, as everywhere else.
- **The guard tests must be proven to fail.** For each guard, once, by
  hand:
  - rename a test it relies on, remove an alias, or drop a command from
    the reference page
  - watch the guard go red
  - revert

  Record that it was done in the milestone doc.
- **An excuse needs a reason a reader can check.** It must name a `Q-`
  entry, or a milestone doc that explains why the Go test has no Rust
  counterpart. "Covered elsewhere" is not an excuse. Name the test that
  covers it, and record the row as `ported`.
- Go's sources are at `../gage` (a checkout at the pinned SHA) for reading.
  Never vendor them.
- Each milestone ends green on `make lint` and `make test`, locally and on
  all three CI platforms.
- If context runs low mid-milestone, write a `*-sessionhandoff.md` here.
