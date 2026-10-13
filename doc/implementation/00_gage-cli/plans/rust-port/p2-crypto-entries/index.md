# P2 Crypto & entries — implementation plan

[← port plan](../index.md) · Parent issue
[#12](https://github.com/distillerylabs/gage-rs/issues/12) · Roadmap
[#2](https://github.com/distillerylabs/gage-rs/issues/2) · previous:
[P1 Foundation](../p1-foundation/index.md)

The first key material, the entry format, and the first usable slice. At
the end of P2, `gage init` generates this device's passphrase-wrapped
identity, and `insert`, `cat`, `rm` and `ls` store and read encrypted
entries, one commit per write, under the vault lock. No sync, no
resolver beyond exact matches, no session.

This plan does not restate the shared ground: layering rules, test
conventions, crate choices and cross-milestone contracts live in the
[port plan](../index.md). Decisions are recorded in
[open-questions.md](../open-questions.md).

---

## Why this phase looks the way it does

P2 ports Go's M2 (identity and memory protection), M3 (entry format) and
M4 (one-shot CRUD), with E0's vault-id-keyed identity paths folded into
M2 ([Q-PORT-STATE](../open-questions.md#q-port-state)). The three map one
to one onto Rust milestones, so they keep Go's IDs.

The crate questions P1 deferred (the rest of
[Q-AGE-CRATE](../open-questions.md#q-age-crate),
[Q-YAML-CRATE](../open-questions.md#q-yaml-crate),
[Q-PLATFORM-CRATES](../open-questions.md#q-platform-crates)) were settled
in planning with throwaway spikes against the real crates, not from
documentation. Three findings changed the plan:

- **Rust `age` reports a wrong passphrase differently from Go's.** Go's
  age returns "no identity matched" and gage inspects the stanza types;
  Rust's returns `DecryptionFailed`. The classification rule is rewritten
  for the Rust crate, not transliterated.
- **Debug-build scrypt is about 50x slower than release.** Work factor 19
  took 36 s in a debug build and 0.73 s in release, and every scrypt
  identity or recipient also runs an 87 ms calibration pass. Without a
  profile override the guard test that writes a real factor-19 file, and
  every test that unlocks a vault, would dominate `make test`.
- **Go owns its YAML output, and so must gage-rs.** Go builds the entry
  as an explicit node tree with chosen scalar styles and verifies its own
  round trip, because yaml.v3's styling corrupts some values. The Rust
  serde YAML crates offer no per-scalar style control, so the port keeps
  Go's design: a parser crate for reading and a gage-owned emitter.

Some P2-tagged Go test files belong to other phases, so the phase map was
re-tagged (see the table below), as P1 did for `vault_test.go`.

## Resolved decisions

| ID | Decision |
|---|---|
| [Q-AGE-CRATE](../open-questions.md#q-age-crate) | `age` 0.12.1, default features only. Wrong passphrase is `is_scrypt()` plus `DecryptionFailed`; a non-scrypt file or any other failure is a corrupt identity file. Max accepted work factor set explicitly to 22. `.age-recipients` round trip parses lines with `x25519::Recipient` (age's own reader needs `cli-common`). `deny.toml` gains BSD-3-Clause. |
| Scrypt test speed | `[profile.dev.package.*]` `opt-level = 3` for `scrypt`, `salsa20`, `pbkdf2`, `sha2`, `hmac`: debug builds then run scrypt at release speed (measured). |
| [Q-YAML-CRATE](../open-questions.md#q-yaml-crate) | `saphyr` 0.2 (no default features) to parse, with raw scalar representations; a gage-owned emitter with Go's verify-faithful check and always-quoted fallback; `jiff` for RFC 3339 timestamps. `deny.toml` gains Zlib. |
| [Q-PLATFORM-CRATES](../open-questions.md#q-platform-crates) | `libc` + `windows-sys` directly for memlock and core-dump suppression; page-aligned per-key allocation; `zeroize`; `rpassword` 7.5 for masked input; `portable-pty` 0.9 (dev-only) for Unix-only real-terminal tests, as Go's are `linux \|\| darwin`. |
| [Q-IDENTITY-SECRET](../open-questions.md#q-identity-secret) | `Identity` holds the secret key line in a locked page-aligned buffer and parses the X25519 key per operation; `Drop` wipes, unlocks, frees; not `Clone`. Go's `Close` misuse tests become `compile_fail` doctests or written excuses. |
| [Q-SCRYPT-HOOK](../open-questions.md#q-scrypt-hook) | Live work factor in an atomic; setter only under `cfg(test)` / `test-support`; an idempotent fixture helper replaces Go's `TestMain`; Go's three guard tests ported. |
| Phase map | `entry_edit`, `insert_edit`, `editor`, `scratchdir*` re-tagged P3 (Go M5 owns `edit`/`rename`/`$EDITOR`); `cat_test.go` re-tagged P6 (#55 display only). P2 now owns 193 inventory rows. |
| M4 resolution | Exact title, then exact UUID; ambiguity is a typed candidate list. P3 · M5 inserts the substring stages between them. |
| Interim output | `ls` prints Go's current five columns; `cat` prints the raw entry YAML until P6 · M12. |
| Retry policy | Three passphrase attempts, owned by the CLI prompter (Go's `maxPassphraseAttempts`). |

## Milestones

Legend: `[ ]` not started, `[~]` in progress, `[x]` done.

| # | Milestone | Model | Depends on | Inventory rows | Status |
|---|---|---|---|---|---|
| M2 | [Identity, age wrapper, memory protection](m2-identity-crypto.md) ([#13](https://github.com/distillerylabs/gage-rs/issues/13)) | **Opus**; ⚑ Opus review of the tests | P1 | 83 + 4 moved from P1 (+ identity halves of 5) | `[~]` |
| M3 | [Entry format](m3-entry-format.md) ([#14](https://github.com/distillerylabs/gage-rs/issues/14)) | Sonnet | M2 | 37 | `[~]` |
| M4 | [One-shot CRUD](m4-crud.md) ([#15](https://github.com/distillerylabs/gage-rs/issues/15)) | Sonnet; ⚑ Opus review of the multi-process lock tests XX

`⚑` = run an Opus review pass over that milestone's *tests* before moving
on. M2's is the one Go's plan calls out as not to economize on: crypto
correctness, page locking and core dumps on three platforms, where subtly
wrong is the worst outcome and a passing suite is least likely to notice.
M4's covers the write lock held across read-modify-commit, which is only
proven by child processes (file-lock semantics are per open file
description, so a thread-based test passes and proves nothing).

Switch models at milestone boundaries, not within them.

## Inventory accounting

After the re-tag, P2 owns 193 rows of `tests/fixtures/go/test-inventory.tsv`.
The 5 `agekey_test.go` rows were ported by P1 · M1. The other 188 are
claimed by M2 (83), M3 (37) or M4 (54), or moved by row (14):

| Destination | Rows |
|---|---|
| P3 · M5 | `insert_field` `TestInsertFieldWithEditFlagRejectedAsMutuallyExclusive` (`-e` arrives with `edit`); the `show --field` halves of two `insert_field` rows |
| P3 · M6 | `prompter` `TestChooseOnAnEmptyCandidateListIsInternal`, `TestChooseRetriesThenGivesUp` (the session picker) |
| P3 · M7 | `entry` (CLI) `TestLsRendersIdenticallyInBothModes`; the `search` half of `TestLsAndSearchPrintNothingWhenThereIsNothing` |
| P4 · M8a | `prompter` `TestConfirmDefaultYesDefaultsToYes` (clone's offer) |
| P4 · M8b | `prompter` `TestResolveConflictNeverGuesses` |
| P5 · M10 | `prompter` `TestConfirmRecipientChangeNamesBothDirectionsOfAMismatch`, `TestRecipientChangeCountSummarizesEveryShape` |
| P6 · M12 | `entry` (CLI) `TestAlignCatOutputLeavesNonMappingInputAlone`, `TestPadKeyLinesWithNoKeysIsANoOp`, `TestLsHeaderEmptyVaultNoOutput`, `TestLsHeaderPrintsLabelledPipeDelimitedTable`, `TestLsHeaderRendersIdenticallyInBothModes`, `TestLsDefaultOutputUnchangedByHeaderFeature` |
| P7 | The enrollment halves of `crypt` `TestOnlyTheTestHooksWriteTheWorkFactors` and `TestEveryScryptIdentityCapsItsWorkFactor` |

Rows P1 moved into P2 are claimed in [M2](m2-identity-crypto.md) (task
8) and [M4](m4-crud.md) (task 6). Rows re-tagged away by file:

| File | Rows | New phase | Why |
|---|---|---|---|
| `internal/gage/entry_edit_test.go` | 6 | P3 | `rename` and the entry-update path are Go M5's |
| `cmd/gage/insert_edit_test.go` | 7 | P3 | `insert -e` shares M5's `$EDITOR` helper |
| `cmd/gage/editor_test.go` | 11 | P3 | The `$EDITOR` scratch-file round trip |
| `cmd/gage/scratchdir*_test.go` | 4 | P3 | Where that scratch file lives |
| `cmd/gage/cat_test.go` | 10 | P6 | `cat`'s id line and alignment (#55), M12 |

## Process notes

- Branches stack: `main` <- `12-p2-crypto-entries` (these docs) <- M2 <-
  M3 <- M4. Each milestone branch is cut from the previous one when the
  user says to start it.
- Tests first, at task granularity. Each task's tests are written, then
  implemented to green, before the next task starts.
- The failure mode to watch is quiet scope narrowing: most of a list done
  and the milestone reported done. Check the boxes and the claimed
  inventory rows, not a summary. A test that turns out not worth writing
  is struck from the list *in the doc* with a one-line reason.
- Each milestone ends green on `make lint` and `make test` locally and on
  all three CI platforms.
- If context runs low mid-milestone, write a `*-sessionhandoff.md` here.
