# P1 Foundation — implementation plan

[← port plan](../index.md) · Parent issue
[#4](https://github.com/distillerylabs/gage-rs/issues/4) · Roadmap
[#2](https://github.com/distillerylabs/gage-rs/issues/2)

The Cargo workspace, CI and lint purity, the library primitives every later
phase publishes against, and vault lifecycle. **No crypto anywhere in this
phase.** At the end of P1 you can create, list, inspect, re-point and forget
vaults; nothing in them is encrypted yet and there are no entries.

This plan does not restate the shared ground: layering rules, test
conventions, crate choices and cross-milestone contracts live in the
[port plan](../index.md). Decisions are recorded in
[open-questions.md](../open-questions.md).

---

## Why this phase looks the way it does

P1 is the port of Go's M0 (scaffolding) and M1 (vault lifecycle), with E0's
vault-id keying folded into M1 ([Q-PORT-STATE](../open-questions.md#q-port-state)).
Go's M0 is split in two because the toolchain, CI and lint purity are
reviewable on their own and contain no library logic.

Planning P1 also changed the port's compatibility basis. The first draft
targeted byte-for-byte compatibility with Go and grew a go-toml-style
emitter, a Cobra-format help renderer with captured usage-error fixtures, a
Windows lock byte range matching Go's, Go-written fixture vaults and a
separate spike milestone. There are no Go-written vaults to protect, so
[Q-COMPAT](../open-questions.md#q-compat) was amended to **behavioral
parity**: Go is the spec (behavior, on-disk schema, test inventory), not a
peer. That removed roughly a third of this phase.

## Resolved decisions

| ID | Decision |
|---|---|
| [Q-COMPAT](../open-questions.md#q-compat) | Amended: behavioral parity, not byte parity. No cross-binary promise, no differential harness. |
| [Q-FILE-LOCK](../open-questions.md#q-file-lock) | `std::fs::File::lock`/`try_lock` on every platform. Contended: poll every 20 ms up to a timeout, then a typed `Contended { path, holder }`. |
| [Q-GIT-CRATE](../open-questions.md#q-git-crate) | Decided by an Opus spike as M0b's first task, against P4's push/merge needs. |
| [Q-AGE-CRATE](../open-questions.md#q-age-crate) | `age` crate for recipient validation in M1; the rest moves to P2 planning. |
| [Q-YAML-CRATE](../open-questions.md#q-yaml-crate), [Q-PLATFORM-CRATES](../open-questions.md#q-platform-crates) | Moved to P2 planning, where they block. |
| [Q-INIT-INTERIM](../open-questions.md#q-init-interim) | `init` requires `--recipient` until M2; CLI `--remote` waits for P4 · M8a. |
| [Q-HELP-RENDER](../open-questions.md#q-help-render) | clap renders help and usage errors; the registry is the single source for groups and availability; tests assert structure. |
| Git defaults | Initial branch and commit signature are set explicitly by gage, never inherited from the user's git config. |
| `vault_test.go` | Re-tagged P2 in `phase-map.awk`: it tests `Identity` close and page locking (M2), not vault lifecycle. P1 now owns 186 inventory rows. |

## Milestones

Legend: `[ ]` not started, `[~]` in progress, `[x]` done.

| # | Milestone | Model | Depends on | Inventory rows | Status |
|---|---|---|---|---|---|
| M0a | [Toolchain, CI, lint purity](m0a-toolchain.md) ([#5](https://github.com/distillerylabs/gage-rs/issues/5)) | Sonnet | — | 0 | `[x]` |
| M0b | [Git-crate spike, primitives, registry, help](m0b-primitives.md) ([#6](https://github.com/distillerylabs/gage-rs/issues/6)) | **Opus** for task 1 (spike); Sonnet after; ⚑ Opus review of the tests | M0a | 97 | `[~]` implemented; awaiting 3-OS CI |
| M1 | [Vault lifecycle and vault-id keying](m1-vault-lifecycle.md) ([#7](https://github.com/distillerylabs/gage-rs/issues/7)) | Sonnet; ⚑ Opus review of the untrusted-input tests | M0b | 89 | `[ ]` |

`⚑` = run an Opus review pass over that milestone's *tests* before moving on.
M0b's is for `vaultlock`: a thread-based test passes and proves nothing,
because file-lock semantics are per open file description. M1's is for the
vault-id and device-name validation, which builds filesystem paths from a
committed file any git-writer can edit.

Switch models at milestone boundaries, except M0b's spike, which is a
self-contained first task with its own deliverable (a resolved `Q-` entry)
and a clean seam before the TDD work starts.

## Inventory accounting

All 186 P1 rows of `tests/fixtures/go/test-inventory.tsv` are claimed by
M0b (97) or M1 (89). M1 also claims 7 P4-tagged `gitrepo_test.go` rows
(init-and-commit, set-remote, remote URL, `IsClean`), because it builds
that git surface. Each milestone doc lists what it ports and, for every
row or part of a row it does not finish, the destination milestone and the
reason. Rows that move out of P1 entirely:

| Destination | Rows |
|---|---|
| P2 · M2 | 3 from `vault_lifecycle` (identity generation), 1 from `vaultid_keying`, plus the identity halves of several M1 rows and the `Prompter::warn` half of `errorprefix`'s warning row |
| P2 · M4 | 1 from `vaultid_keying` (shared lock exercised by `insert`), plus the `insert`/`show` half of one more |
| P3 · M6 | 1 from `usageerror` (session-line usage errors) |
| P4 · M8a | 1 from `vaultid_keying` (`clone`) |
| P5 | 2 from `vaultid_keying` (`identity add` pubkey; trust cache under a reused name) |
| P6 · M12 | 3 from `vault_lifecycle` and 9 from `vaultid_keying` (orphaned-identity cleanup on `vault remove`) |

## Process notes

- Branches stack: `main` <- `4-p1-foundation` (these docs) <- M0a <- M0b <-
  M1. Each milestone branch is cut from the previous one when the user says
  to start it.
- Tests first, at task granularity. Each task's tests are written, then
  implemented to green, before the next task starts.
- The failure mode to watch is quiet scope narrowing: most of a list done
  and the milestone reported done. Check the boxes and the claimed
  inventory rows, not a summary. A test that turns out not worth writing is
  struck from the list *in the doc* with a one-line reason.
- Each milestone ends green on `make lint` and `make test` locally and on
  all three CI platforms.
- If context runs low mid-milestone, write a `*-sessionhandoff.md` here.
