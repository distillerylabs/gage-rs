# P3 Query & session — implementation plan

[← port plan](../index.md) · Parent issue
[#21](https://github.com/distillerylabs/gage-rs/issues/21) · Roadmap
[#2](https://github.com/distillerylabs/gage-rs/issues/2) · previous:
[P2 Crypto & entries](../p2-crypto-entries/index.md)

Addressing, the rest of the entry commands, and the second invocation
mode. At the end of P3 every query-taking command resolves through the
full order (exact title → substring title → exact UUID → substring UUID →
ambiguous); `show`, `edit`, `rename`, `generate`, `search` and `reindex`
exist; bare `gage` on a terminal opens a session that unlocks each vault
once, re-locks on `lock` or idle, and keeps a page-locked metadata index;
and the REPL completes on Tab and runs unambiguous command abbreviations.
No sync (P4), no recipients (P5), no `--clip`/`--qr`/`--script` (P6).

This plan does not restate the shared ground: layering rules, test
conventions, crate choices and cross-milestone contracts live in the
[port plan](../index.md). Decisions are recorded in
[open-questions.md](../open-questions.md).

---

## Why this phase looks the way it does

P3 ports Go's M5 (query resolution), M6 (session mode), M7 (metadata
index) and M13 (tab completion). Each maps one to one onto a Rust
milestone, so they keep Go's IDs. M13 sits here rather than after P6
because in Go it depends only on M0, M6 and M7.

P3 comes before P4 because M8a must invalidate M7's index after a pull,
and M7's index is what makes that contract exist.

The one new crate is the line editor. `rustyline` was checked against its
source in planning, not its docs, and one finding simplifies the port:

- **`rustyline` holds the terminal only during a `readline()` call.** Go's
  `chzyer/readline` keeps the terminal in raw mode and reads stdin from
  its own goroutine for the life of an instance, so Go had to route the
  passphrase prompt through the line editor to avoid two readers racing.
  gage-rs's terminal prompter can keep reading stdin directly between
  REPL lines. The REPL still reads through a `LineReader` trait (a plain
  implementation over `Io.stdin`, and a `rustyline` one) so that it is
  testable in-process, as Go's `plainLineReader` makes it.

The borrow checker also changes one of Go's mechanisms. Go's `Session`
holds `*Identity` pointers and needs an `ErrIdentityClosed` contract plus
a call-depth counter so a re-lock can't land under a command that still
holds a key. In Rust a command borrows `&Identity` from the `Session`, and
`lock` needs `&mut Session`, so the overlap can't compile. The depth rule
survives only as *when* the idle timeout is evaluated.

## Resolved decisions

| ID | Decision |
|---|---|
| [Q-LINE-EDITOR](../open-questions.md#q-line-editor) | `rustyline` 18.0.1 (MIT) in `gage-cli` only. History is added explicitly per submitted line (`auto_add_history` is off by default); `FileHistory` creates the file 0600 on Unix; a custom `Completer` supplies candidates. A `LineReader` trait keeps the REPL testable without a terminal. |
| [Q-SESSION-SHAPE](../open-questions.md#q-session-shape) | `Session` holds `Option<Identity>` per vault, commands borrow `&Identity` from it; idle timeout evaluated once at the outermost call; clock injected as `Box<dyn Fn() -> Instant>`; `Prompter::choose(&CandidateList)` is the session picker. |
| [Q-EDITOR](../open-questions.md#q-editor) | `$EDITOR` split on whitespace and resolved on `PATH` with the `which` crate (`gage-cli` only) before any scratch file exists; tmpfs scratch directory on Linux (`statfs` via `libc`), OS temp directory elsewhere; 0600, exclusive create, overwrite then unlink on every path; tests fake the editor by re-executing the test binary. |
| [Q-INDEX-ARENA](../open-questions.md#q-index-arena) | One `LockedBuf` arena per vault index, packed title/description bytes, grown by allocate-copy-drop, page-locked through the vault's `Arc<dyn Locker>`; metadata only, never written to disk. |
| Flag scope | `show --field`, `generate -l/--length` and `generate --no-symbols` land in M5 (all in the help fixtures; P2 deferred `show --field` to M5). `-c/--clip` and `-q/--qr` on `show`/`generate` stay in P6 · M12 with the clipboard and QR crates; so do root `--script`/`--stdin`. |
| Phase map | No re-tags. 4 P3 rows move to P6 by row (below). |
| Interim output | `search` prints `title  shortid` per match, as Go's `writeSearchResults`; `cat` keeps the raw YAML until P6 · M12. |

## Milestones

Legend: `[ ]` not started, `[~]` in progress, `[x]` done.

| # | Milestone | Model | Depends on | Inventory rows | Status |
|---|---|---|---|---|---|
| M5 | [Query resolution and the entry commands](m5-query-resolution.md) ([#22](https://github.com/distillerylabs/gage-rs/issues/22)) | Sonnet; ⚑ Opus review of the scratch-file tests | P2 | 79 + 3 from P2 | `[~]` local `make lint`/`make test` green; awaiting CI on Linux, macOS, Windows |
| M6 | [Session mode](m6-session-mode.md) ([#23](https://github.com/distillerylabs/gage-rs/issues/23)) | Sonnet* | M5 | 70 + 2 from P2 + 1 from P1 | `[~]` local `make lint`/`make test`/`make audit` green; awaiting CI on Linux, macOS, Windows |
| M7 | [Metadata index](m7-metadata-index.md) ([#24](https://github.com/distillerylabs/gage-rs/issues/24)) | Sonnet; ⚑ Opus review of the index tests | M6 | 26 + 2 from P2 | `[ ]` |
| M13 | [Tab completion & abbreviation](m13-tab-completion.md) ([#25](https://github.com/distillerylabs/gage-rs/issues/25)) | Sonnet* | M7 | 46 | `[ ]` |

`⚑` = run an Opus review pass over that milestone's *tests* before moving
on. M5's covers the `$EDITOR` scratch file, the one place P3 puts a
decrypted entry on disk: removal on every exit path, the overwrite before
unlink, and refusing before any file exists when `$EDITOR` is unusable.
M7's covers the index arena, which keeps titles and descriptions resident
for a whole session: no secret value in the arena, zeroed on discard,
page locks preserved across growth. `*` = borderline: start on Sonnet,
switch to Opus at a milestone boundary if the `rustyline` wiring (M6) or
the `Completer` integration (M13) turns awkward.

Switch models at milestone boundaries, not within them.

## Inventory accounting

P3 owns 225 rows of `tests/fixtures/go/test-inventory.tsv`. 221 are
claimed by M5 (79), M6 (70), M7 (26) or M13 (46). By file:

| Milestone | Files (rows) |
|---|---|
| M5 | `resolve` (12), `resolve_cli` (6), `show` (8 of 9), `edit` (5), `rename` (3), `entry_edit` (6), `generate` library (11), `generate` CLI (6 of 9), `insert_edit` (7), `editor` (11), `scratchdir*` (4) |
| M6 | `session` library (20 of 24), `repl` (27), `session` CLI (4), `session_pty` (1), `sessionhelp` (9), `shell` (9) |
| M7 | `index` (20), `search` library (2), `search` CLI (4) |
| M13 | `complete` (41), `complete_pty` (1), the 4 `EntryTitleCandidates` rows of `session` library |

Rows moved out of P3:

| Destination | Rows | Why |
|---|---|---|
| P6 · M12 | `generate` CLI `TestGenerateClipAndQRAreMutuallyExclusive`, `TestGenerateClipCopiesTheValueAndPrintsNone`, `TestGenerateQRRendersWithoutPrintingTheValue`; `show` `TestShowClipCopiesTheNamedFieldNotTheValue` | `-c`/`-q` need the clipboard and QR crates, which M12 introduces |
| P6 · M12 | The `--stdin` half of `repl` `TestSessionCommandArgumentCountsAreChecked` | `--stdin`/`--script` are M12's |

Rows earlier phases moved into P3:

| From | Rows | Claimed by |
|---|---|---|
| P1 · M0b | `usageerror` `TestSessionWrongArgCountShowsCommandUsage` | M6 |
| P2 · M4 | `insert_field` `TestInsertFieldWithEditFlagRejectedAsMutuallyExclusive`; the `show --field` halves of `TestInsertFieldRoundTripsThroughShow` and `TestInsertMultipleFieldFlagsEachIndependentlyReadable`; the `show` half of `vaultid_keying` `TestAVaultWithoutAnIDFailsLoudlyOnEveryDerivedPath` | M5 |
| P2 · M2 | `prompter` `TestChooseOnAnEmptyCandidateListIsInternal`, `TestChooseRetriesThenGivesUp` | M6 |
| P2 · M4 | `entry` (CLI) `TestLsRendersIdenticallyInBothModes`; the `search` half of `TestLsAndSearchPrintNothingWhenThereIsNothing` | M7 |

## Process notes

- Branches stack: `main` <- `21-p3-query-session` (these docs) <- M5 <-
  M6 <- M7 <- M13. Each milestone branch is cut from the previous one when
  the user says to start it.
- Tests first, at task granularity. Each task's tests are written, then
  implemented to green, before the next task starts.
- The failure mode to watch is quiet scope narrowing: most of a list done
  and the milestone reported done. Check the boxes and the claimed
  inventory rows, not a summary. A test that turns out not worth writing
  is struck from the list *in the doc* with a one-line reason.
- M7's definition of done includes M5's and M6's tests passing
  unmodified: the index is a cache, and that is the proof it didn't change
  behavior.
- Each milestone ends green on `make lint` and `make test` locally and on
  all three CI platforms. Session state (idle timeout, several identities
  held at once) is first proven on Windows here, as it was in Go.
- If context runs low mid-milestone, write a `*-sessionhandoff.md` here.
