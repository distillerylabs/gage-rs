# P5 Recipients & trust — implementation plan

[← port plan](../index.md) · Parent issue
[#41](https://github.com/distillerylabs/gage-rs/issues/41) · Roadmap
[#2](https://github.com/distillerylabs/gage-rs/issues/2) · previous:
[P4 Sync](../p4-sync/index.md)

At the start of P5 a vault syncs, but only the device that ran `init` can
read it. By the end of P5:

- A second device can register its own identity: `identity add` and
  `identity list`.
- Access can be granted and revoked: `recipient add`, `recipient remove`,
  `recipient list` and `recipient verify`.
- `recipient add` always re-encrypts the whole vault (A19), and an actor
  who can't read every entry is refused up front.
- Re-encryption is all-or-nothing: an interrupted run leaves HEAD
  untouched, and the next write resets the leftovers with a warning.
- An unreviewed change to the recipient list is shown as a diff and
  confirmed before the next encrypt. A hand-edited `.age-recipients`
  can't be laundered by confirming or by running `recipient add`.
- `mv`/`cp --to-vault` share an entry into another vault, which is never
  unlocked.

P5 does not include enrollment (`identity enroll`, `recipient
pending/approve/deny`: P7) or the recovery key (P8). It also does not
include the N-recipient form of the recipient write (E1b), which moves to
P7 with its tests.

This plan does not restate the shared ground. The layering rules, test
conventions, crate choices and cross-milestone contracts are in the
[port plan](../index.md). Decisions are recorded in
[open-questions.md](../open-questions.md).

---

## Why this phase looks the way it does

P5 ports Go's M9 (with A19/E1a folded in), M10 and M11.

**Four milestones instead of Go's three.** Go's M9 claims about 63 rows
here. Two very different kinds of work share it:

- the read side: identity files, `list`, and a `verify` that needs no
  unlock
- the write side: re-encryption whose correctness only shows after a
  crash

So M9 splits by risk ([Q-P5-SPLIT](../open-questions.md#q-p5-split)).
M9a is additive and pattern-following, on Sonnet. M9b carries the
crash-safety, on Opus.

**E1b's tests leave the phase.** The phase map tagged
`recipient_batch_test.go` P5 because of its file name. It tests the
N-recipient form of `AddRecipient` (a slice, extra delete paths, a
caller-supplied push clause). Go built that form in E1b, and its only
consumer is E4's batch approval. Building it now would be scaffolding for a
caller two phases away, so it is re-tagged to P7 · E1b
([Q-E1B-RETAG](../open-questions.md#q-e1b-retag)). M9b builds the
single-recipient write so that E1b widens it rather than rewriting it.

Planning found these points that Go's documents don't make, because they
are specific to this tree:

- **Identity carries no prompter.** Go asks through `ident.frontend()`.
  In gage-rs every write already takes `prompter: &dyn Prompter`, and the
  questions use that parameter.
  [Q-RECIPIENT-PROMPT](../open-questions.md#q-recipient-prompt) restates
  Go's "`--yes` is refused in a session" argument in those terms.
- **Encryption always reads `.age-recipients` from disk**
  (`entry/store.rs`, `encrypt_recipients`). M9b adds an
  encrypt-to-an-explicit-list write. Go's crash ordering depends on it:
  every entry is written first and the recipient pair last.
- **The dirty-tree reset doesn't exist yet.** `gitrepo::reset_hard` was
  built in M8a1 and has its tests, but no write calls it. M9b adds
  `with_vault_write` and moves every mutating method onto it.
- **Locking one vault twice in one process waits out the full timeout**
  (one `flock` per open file description). M11 refuses a same-id source
  and destination before taking any lock
  ([Q-TWO-VAULT-LOCK](../open-questions.md#q-two-vault-lock)). The lock
  order is by vault id, which is Go's code after A20, not the name order
  its M11 doc gives.
- **`Session::vault` borrows `&mut self`**, so M11 needs an accessor that
  yields the source's identity alongside a destination it never unlocks.
- **There is no two-device gage harness.** The existing "other device" is
  a raw git clone. M9a builds one, and every later milestone reuses it.

## Resolved decisions

| ID | Decision |
|---|---|
| [Q-E1B-RETAG](../open-questions.md#q-e1b-retag) | `recipient_batch_test.go` (15 rows) is re-tagged to P7 · E1b. P5 owns 121 rows, P7 owns 201. |
| [Q-P5-SPLIT](../open-questions.md#q-p5-split) | Go's M9 becomes M9a (identities, recipient read side) and M9b (recipient writes, re-encryption, dirty-tree reset). |
| [Q-CRASH-SEAM](../open-questions.md#q-crash-seam) | Interruptions are injected through `#[cfg(test)]` hook fields on `Vault`, matching Go's `onReencryptEntry` and `onMoveDestCommitted`. Every test that uses one is an in-crate library test. There is no `test-support` feature. |
| [Q-TRUST-HASH](../open-questions.md#q-trust-hash) | The `.age-recipients` hash is SHA-256 from `sha2` (already in the tree through `age`). The diff is gage's own LCS unified diff, ported from Go with its test. No diff crate. |
| [Q-RECIPIENT-PROMPT](../open-questions.md#q-recipient-prompt) | `Prompter::confirm_recipient_change(&RecipientChangeWarning) -> Result<bool, PromptError>`. Its default refuses. `--yes` is a root flag that wraps the prompter and answers only this method. It is refused in a session. |
| [Q-TWO-VAULT-LOCK](../open-questions.md#q-two-vault-lock) | `mv`/`cp` hold both vault locks for the whole operation, acquired in vault-id order. A same-id source and destination are refused before any lock. |
| [Q-TRUST-CACHE-PATH](../open-questions.md#q-trust-cache-path) | `$GAGE_STATE/<vault-id>/known-config.toml` plus the hash file, written with `atomicfile`, keyed by id from the start. `vault remove` deletes the directory. |
| Interim | `identity add` has no recovery-label reservation (Go's `checkDeviceLabelFree`) until P8. |

## Milestones

Legend: `[ ]` not started, `[~]` in progress, `[x]` done.

| # | Milestone | Model | Depends on | Inventory rows | Status |
|---|---|---|---|---|---|
| M9a | [Identities & recipient read side](m9a-identity-recipient-read.md) ([#42](https://github.com/distillerylabs/gage-rs/issues/42)) | Sonnet | P4 | 26 (+3 inbound) | `[x]` |
| M9b | [Recipient writes & re-encryption](m9b-recipient-write.md) ([#43](https://github.com/distillerylabs/gage-rs/issues/43)) | **Opus** | M9a | 37 | `[~]` |
| M10 | [Trust cache](m10-trust-cache.md) ([#44](https://github.com/distillerylabs/gage-rs/issues/44)) | Sonnet\* ⚑ | M9b | 38 (+3 inbound) | `[ ]` |
| M11 | [Cross-vault `mv`/`cp`](m11-cross-vault.md) ([#45](https://github.com/distillerylabs/gage-rs/issues/45)) | Sonnet\* ⚑ | M10 | 20 | `[ ]` |

`⚑` means an Opus review pass over that milestone's tests before it is
called done:

- **M10:** the laundering tests (a pulled change confirmed through
  `recipient add`, and a mismatch confirmed with `--yes`) and the
  refusal-ordering tests (out-of-sync before trust, both before the first
  byte).
- **M11:** the lock-order test, which uses child processes, and the
  crash-between-commits test.

M9b runs on Opus throughout. Its crash tests only prove what they
simulate, so its doc names the windows each one reaches.

`*` means borderline: start on Sonnet and switch at the milestone
boundary if it turns awkward.

## Inventory accounting

P5 owned 136 rows of `tests/fixtures/go/test-inventory.tsv`. Planning
re-tagged 15 of them to P7, which leaves 121. They are claimed by M9a
(26), M9b (37), M10 (38) and M11 (20). By file:

| Milestone | Files (rows) |
|---|---|
| M9a | `identity_add_test` (6 of 7), `identity_cli` (7), `recipients/recipients_test` (6), `recipient_test` (4 of 21), `recipient_cli` (3 of 15) |
| M9b | `recipient_test` (17 of 21), `reencrypt_test` (9), `identity_add_test` (1 of 7), `recipient_cli` (10 of 15) |
| M10 | `trustcache_test` (25), `trustcache_cli` (11), `recipient_cli` (2 of 15) |
| M11 | `move_test` lib (13), `move_test` CLI (7) |

Rows that earlier phases left to P5:

| From | Rows | Claimed by |
|---|---|---|
| P1 · M1 | `vaultid_keying` `TestInitAndIdentityAddRecordThePubkeyAndCloneDoesNot` | M9a |
| P1 · M1 (via P2 · M4) | The `identity list` half of `TestAVaultWithoutAnIDFailsLoudlyOnEveryDerivedPath` | M9a |
| P3 · M13 | The `identity` form of `TestSubcommandCandidatesListsIdentityChildren` (written against `vault` until P5) | M9a |
| P1 · M1 | `vaultid_keying` `TestTwoVaultsUnderOneLocalNameShareNeitherDirectory` | M10 |
| P2 | `prompter` `TestConfirmRecipientChangeNamesBothDirectionsOfAMismatch`, `TestRecipientChangeCountSummarizesEveryShape` | M10 |

P4 also left two follow-ups to M10:

- `yes_does_not_resolve_conflicts` pins `sync --yes` as an unknown flag.
  Once `--yes` exists, it has to pin that `--yes` still doesn't resolve a
  conflict.
- The recipient-file conflict summary ends "run `gage sync`", which until
  M10 just refuses again.

Rows re-tagged out of P5:

| Destination | Rows | Why |
|---|---|---|
| P7 · E1b | `recipient_batch_test.go` (15) | It tests E1b's N-recipient write, whose only consumer is E4 ([Q-E1B-RETAG](../open-questions.md#q-e1b-retag)). |

## Process notes

- Branches stack: `main` <- `41-p5-recipients-trust` (these docs) <- M9a
  <- M9b <- M10 <- M11. Each milestone branch is cut from the previous one
  when the user says to start it.
- Tests come first, at task granularity. Each task's tests are written and
  then implemented to green before the next task starts.
- The failure mode to watch is quiet scope narrowing: most of a list done
  and the milestone reported done. Check the boxes and the claimed
  inventory rows, not a summary. A test that turns out not worth writing
  is struck from the list *in the doc*, with a one-line reason.
- Go's "Settled during implementation" and "Corrected after
  implementation" notes in M9 and M10 each name a property that was
  missed the first time. The Rust docs list each one as an explicit test:
  - the reset's coverage beyond `entries/`
  - `recipient add`/`remove` running the blocking check too, so a pulled
    change isn't laundered
  - the `RequireFullAccess` clean-tree gate
  - the CLI fake prompter echoing warnings to stderr
- **No network.** `make test` stays offline. Remotes are `gittest` bare
  repos, and the "other device" is the two-device fixture M9a adds.
- **Every new command** has a registry entry, the three `commands/mod.rs`
  edits, and a row in a `p5_*_commands_have_their_go_fixture_flags_and_descriptions`
  test, following P4's pattern.
- Each milestone ends green on `make lint` and `make test`, locally and on
  all three CI platforms.
- If context runs low mid-milestone, write a `*-sessionhandoff.md` here.
