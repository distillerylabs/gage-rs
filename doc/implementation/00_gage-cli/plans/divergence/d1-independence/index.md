# D1 Independence — implementation plan

[← divergence plan](../index.md) · Parent issue
[#115](https://github.com/distillerylabs/gage-rs/issues/115) · Roadmap
[#114](https://github.com/distillerylabs/gage-rs/issues/114)

D1 removes everything that makes Go gage the spec, and puts gage-rs's own
guards in its place, so D2–D4 can change the shape of the code without
changing what it does.

D1 adds no behavior and changes no CLI output. Its outputs are:

- a format spec and a golden fixture
- help snapshots
- deletions
- docs

---

## Where things stand at the start of D1

Measured on `main` at `97f9681`.

**Parity machinery**

| What | Where | Read by |
|---|---|---|
| Go test inventory (1,271 rows) and exceptions (94 rows) | `tests/fixtures/go/test-inventory.tsv`, `tests/parity/inventory-exceptions.tsv` | `crates/gage-cli/tests/parity_inventory.rs` |
| Go help captured from the Go binary (47 files) and the pinned SHA | `tests/fixtures/go/help/`, `tests/fixtures/go/GO_GAGE_SHA` | `fixture_support.rs` (a Cobra-help parser), `help_surface_tests.rs`, eleven per-phase `*_have_their_go_fixture_*` tests across `query_tests`, `search_tests`, `sync_tests`, `auth_tests`, `identity_cli_tests`, `movecopy_cli_tests`, `log_cli_tests`, `entry_tests`, `enroll_cli_tests`, `approve_cli_tests` and `recovery_cli_tests`; `script_tests.rs:965`; QA `run-all.sh:75` and `scenarios/11-help-surface.sh` |
| Fixture regeneration | `scripts/go-reference/capture.sh`, `phase-map.awk` | nothing in CI |

**What already exists without Go.** The structural registry ↔ clap tests
live in `crates/gage-cli/src/tests.rs`: `registry_completeness`,
`one_shot_help_matches_registry_exactly` and the drift detectors.
`command_reference_tests.rs` reads only the registry and `doc/cli`.
`shipped_features.rs` guards the scrypt test hook, not parity. All of
these stay.

**On-disk format.** It is defined across:

- `vaultconfig/mod.rs`
- `config.rs`
- `recipients.rs`
- `vault.rs` (`GITATTRIBUTES`, `.gitignore`, the init commit)
- `entry/yaml.rs`
- `identityfile.rs`
- `paths.rs`
- `trustcache.rs`
- `enrollment/{mod,name}.rs`
- `recovery/mod.rs`
- `gitrepo/mod.rs` (author, branch, remote)
- the commit messages in `entry/crud.rs`, `movecopy.rs`, `recipient.rs`,
  `remotesync.rs`, `enrollment/` and `recovery/swap.rs`

There is no single spec. The only thing guarding it is "follows Go".

**Things that break when the Go repo is archived**

- `CODE_OF_CONDUCT.md:64` sends reports to the Go repo's advisory form.
- `SECURITY.md` defers to the Go README's security model.
- The README defers to the Go README for what gage is.
- These decisions are cited from code but defined only in Go's register:
  Q-DEVICE-NAME (8 references), Q-METHOD-SCOPE (7), Q-ORPHAN-BY-NAME (4),
  Q-CMD-AVAILABILITY (4), Q-HELP-SURFACES (3), Q-SYNC-CONFLICT,
  Q-METHOD-FLAG, A19–A22 and D-ENROLL-*.
- The TDDs link six Go-only plan directories.

## Resolved decisions

See the [divergence plan](../index.md#resolved-decisions). Specific to D1:

- **The golden fixture is written once, by gage-rs, and never
  regenerated in place.**
  - A format change adds a `format-v3/` fixture beside it. `format-v2/`
    stays, as the proof that v2 still reads (or that the migration
    works).
  - The generator lives in the tree, `#[ignore]`d, as documentation of
    how the fixture was made.
- **Help snapshots are captured before anything is deleted** (D1b's
  first task). So the CLI output the snapshots pin is, by construction,
  the output P9 signed off.
- **ADRs, not `Q-` entries, from D1c on.** Until D1c lands, D1 decisions
  live in these docs.

## Milestones

| Milestone | Status | Model | Depends on |
|---|---|---|---|
| [D1a Format v2 spec + golden vault](d1a-format-spec.md) ([#116](https://github.com/distillerylabs/gage-rs/issues/116), PR [#120](https://github.com/distillerylabs/gage-rs/pull/120)) | `[x]` | **Opus** | parent branch |
| [D1b Retire parity machinery](d1b-retire-parity.md) ([#117](https://github.com/distillerylabs/gage-rs/issues/117)) | `[x]` | Sonnet | D1a |
| [D1c Docs archive, re-home, process](d1c-docs-process.md) ([#118](https://github.com/distillerylabs/gage-rs/issues/118), PR [#122](https://github.com/distillerylabs/gage-rs/pull/122)) | `[x]` | Sonnet (⚑ Opus review of the security ADRs) | D1b |

**Why D1a is Opus.** The spec becomes the contract that D3 and D4 are
checked against. A fixture that is wrong, or that leaves out an artifact,
lets a format drift through unnoticed. Deciding what a reader must
reject, and what it must preserve, is the same kind of judgement as the
port's security milestones.

**Why D1a comes first.** It adds guards and removes nothing, so it can
merge on its own. D1b removes the Go guards only once gage-rs's own guards
exist. D1c moves the docs D1b's deleted tests still cited.

## Acceptance

- Every milestone doc's test list is green.
- `make lint`, `make test` and `make qa` are clean.
- The golden-vault tests are green on Linux, macOS and Windows CI.
- `grep -rn 'Go gage\|GO_GAGE_SHA\|parity'` outside `doc/archive/` hits
  only ADR-0001.
- The help snapshots are unchanged between D1b's first task and D1c's
  end.

## Process notes

- **Branches stack:** `main` <- `115-divergence-independence` (these docs)
  <- D1a <- D1b <- D1c. Each milestone branch is cut from the previous one
  when the user says to start it.
- **Tests come first, at task granularity**, as everywhere else.
- **Each new guard must be proven to fail once, by hand:**
  - change a serde field name, or one byte of help text
  - watch the guard go red
  - revert

  Record in the milestone doc that this was done.
- **If context runs low mid-milestone,** write a `*-sessionhandoff.md`
  here.
