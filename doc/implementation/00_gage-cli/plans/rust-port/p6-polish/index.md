# P6 Polish — implementation plan

[← port plan](../index.md) · Parent issue
[#52](https://github.com/distillerylabs/gage-rs/issues/52) · Roadmap
[#2](https://github.com/distillerylabs/gage-rs/issues/2) · previous:
[P5 Recipients & trust](../p5-recipients-trust/index.md)

At the start of P6, every command in the design doc exists except the ones
that control where plaintext goes and how gage runs without a human. By
the end of P6:

- `gage log` shows when an entry changed, decrypting nothing, and
  `gage history --decrypt <query>` walks every past revision of one entry.
- `cat` prints a display-only short-id line, aligned labels and flush-left
  multi-line values (#55). `ls -H/--header` prints a labelled table (#54),
  and plain `ls` is byte-for-byte unchanged.
- `vault remove` offers to delete this device's identity file once its
  public key is no longer a recipient, and only behind a confirmation
  (#39 as revised by E0).
- `show` and `generate` take `-c/--clip` and `-q/--qr`. A copy is cleared
  after `[shell] clipboard_timeout` (45s), and only if the clipboard still
  holds what gage wrote.
- `gage --script FILE` and `gage --stdin` run a session's commands
  non-interactively, unlocking from `GAGE_PASSPHRASE`. `--yes` is let
  through in such a session.

P6 does not include enrollment (P7) or the recovery key (P8), whose
`init` display uses M12c's QR renderer.

This plan does not restate the shared ground. The layering rules, test
conventions, crate choices and cross-milestone contracts are in the
[port plan](../index.md). Decisions are recorded in
[open-questions.md](../open-questions.md).

---

## Why this phase looks the way it does

P6 ports Go's M12 and the post-plan items Go recorded under it. Go calls
M12's items "independent of each other and of the trust model", so the
split is by layer and by what each one needs that doesn't exist yet
([Q-P6-SPLIT](../open-questions.md#q-p6-split)):

- **M12a** is library-heavy: a history walk in `gitrepo` and two `Vault`
  methods, with thin commands on top. It goes first, as library work does.
- **M12b** is display and one irreversible deletion. The cleanup's
  decision moves into the library
  ([Q-ORPHAN-CLEANUP](../open-questions.md#q-orphan-cleanup)), and it is
  the one place in P6 where a wrong answer destroys the only copy of a
  private key.
- **M12c** brings the two new frontend crates (clipboard, QR) and the
  only platform-specific behavior in the phase. Isolating it means a CI
  failure on one OS doesn't hold up anything else.
- **M12d** is last because `--script` drives commands from every earlier
  phase and milestone, including M12c's `-c` in a scripted session. It
  also writes the phase's user docs, so they describe shipped behavior.

Go's two security-relevant items are not polish, despite living here:
`-c`/`-q` are the design's answer to principle 6 ("plaintext should be
surprising to produce"), and `history --decrypt` is the design's one more
dangerous tier. Each milestone doc names the tests that pin those
properties.

Planning found these points that Go's documents don't make, because they
are specific to this tree:

- **The clipboard and QR crates are frontend crates.** They go in
  `gage-cli` only, which is also where Go has them (`cmd/gage`). The
  `no_frontend_deps` check already forbids them in `crates/gage`.
- **In-process tests share one environment.** Go's script tests call
  `t.Setenv(GAGE_PASSPHRASE)`. Here `main` reads the variable into `Io`,
  as `editor` already is, so no test touches the process environment
  ([Q-SCRIPT-PASSPHRASE](../open-questions.md#q-script-passphrase)).
- **No prompter is ever unwrapped.** Go's `terminalPrompterOf` exists so
  that a session can hand its line editor to the terminal prompter behind
  `--yes`. gage-rs's terminal prompter reads stdin directly between lines
  ([Q-LINE-EDITOR](../open-questions.md#q-line-editor)), so there is
  nothing to unwrap, and its row is excused.
- **`--yes` waits on script mode.** M10 refused `--yes` in every session
  because no script session existed yet. M12d adds Go's exception
  ([Q-RECIPIENT-PROMPT](../open-questions.md#q-recipient-prompt),
  amended).
- **`gitrepo` has no history walk yet.** M8b proved that a merge commit
  can be walked, but nothing reads a path across history. M12a builds it
  on `git2`'s revwalk.
- **`HeadCommitterTime` is enrollment's.** One Go row tests it alongside
  `Log` on an empty repository. Its only caller is E3's clock-skew check,
  so M12a ports the `Log` half and the other half moves to P7.

## Resolved decisions

| ID | Decision |
|---|---|
| [Q-P6-SPLIT](../open-questions.md#q-p6-split) | Go's M12 becomes M12a (`log`/`history`), M12b (`cat`, `ls -H`, orphan cleanup), M12c (`-c`/`-q`) and M12d (`--script`/`--stdin`, docs). |
| [Q-CLIPBOARD](../open-questions.md#q-clipboard) | `arboard` in `gage-cli`, default features off, `wayland-data-control` on, behind a clipboard trait with injectable timer and wait hooks. One-shot Ctrl-C is caught with `ctrlc`, installed only on the one-shot `-c` path. |
| [Q-QR-CRATE](../open-questions.md#q-qr-crate) | `qrcode`, default features off; EC level M, quiet zone 4, gage's own half-block renderer. Tests compare the rendered grid with the crate's module grid. |
| [Q-SCRIPT-PASSPHRASE](../open-questions.md#q-script-passphrase) | `GAGE_PASSPHRASE` reaches the CLI through an `Io` field. Three `gage-cli` wrapper prompters (env, refusing, no-create) carry the policy. `TestTerminalPrompterOfWalksDecoratorChains` is excused. |
| [Q-ORPHAN-CLEANUP](../open-questions.md#q-orphan-cleanup) | The library assesses the identity file and returns a typed value. The CLI asks through `confirm`, which defaults to no. |
| [Q-RECIPIENT-PROMPT](../open-questions.md#q-recipient-prompt) (amended) | `--yes` is accepted in a `--script`/`--stdin` session and still refused in an interactive one. |
| Help | `--script`, `--stdin` and `--yes` are hidden root flags: Go's root help lists none. |

## Milestones

Legend: `[ ]` not started, `[~]` in progress, `[x]` done.

| # | Milestone | Model | Depends on | Inventory rows | Status |
|---|---|---|---|---|---|
| M12a | [`log` / `history --decrypt`](m12a-log-history.md) ([#53](https://github.com/distillerylabs/gage-rs/issues/53)) | Sonnet ⚑ | P5 | 24 | `[x]` |
| M12b | [`cat` display, `ls -H`, `vault remove` cleanup](m12b-display-cleanup.md) ([#54](https://github.com/distillerylabs/gage-rs/issues/54)) | Sonnet ⚑ | M12a | 10 (+18 inbound) | `[x]` |
| M12c | [`-c/--clip`, `-q/--qr`](m12c-clip-qr.md) ([#55](https://github.com/distillerylabs/gage-rs/issues/55)) | Sonnet\* | M12b | 27 (+4 inbound) | `[x]` |
| M12d | [`--script` / `--stdin`, `GAGE_PASSPHRASE`, docs](m12d-script-mode.md) ([#56](https://github.com/distillerylabs/gage-rs/issues/56)) | Sonnet ⚑ | M12c | 26 (+1 inbound) | `[ ]` |

`⚑` means an Opus review pass over that milestone's named tests before it
is called done:

- **M12a:** the merge-walk rows. A walk that diffs neighbours instead of
  parents invents changes and deletions on every synced vault, and Go
  shipped that bug once.
- **M12b:** the never-delete rows of the orphan cleanup. Deleting the
  wrong identity file can't be undone.
- **M12d:** the rows proving the command stream is never read as a
  passphrase, and that `GAGE_PASSPHRASE` never answers a new identity.

`*` means borderline: start on Sonnet and switch at the milestone boundary
if the clipboard on three platforms turns awkward.

## Inventory accounting

P6 owns 87 rows of `tests/fixtures/go/test-inventory.tsv`. Planning
re-tags none. They are claimed by M12a (24), M12b (10), M12c (27) and
M12d (26). By file:

| Milestone | Files (rows) |
|---|---|
| M12a | `gitrepo/history_test` (6), `history_test` lib (6), `log_test` CLI (12) |
| M12b | `cat_test` (10) |
| M12c | `clip_test` (20), `qr_test` (7) |
| M12d | `script_test` (26) |

Rows that earlier phases left to P6:

| From | Rows | Claimed by |
|---|---|---|
| P1 · M1 | `vault_lifecycle` `TestVaultRemoveDeletesAnOrphanedIdentityFile`, `TestVaultRemoveKeepsIdentityFileStillListedAsRecipient`, `TestVaultRemoveKeepsIdentityFileWhenRecipientsUnreadable` | M12b |
| P1 · M1 | `vaultid_keying` `TestNonInteractiveVaultRemoveNeverDeletesAnIdentity`, `TestVaultRemoveIdentifiesAStaleKeyWhoseNameNowLabelsAnother`, `TestVaultRemoveKeepsARelabelledRecipientsKey`, `TestVaultRemoveKeepsTheFileWhenNoPubkeyIsRecorded`, `TestVaultRemoveNeverDeletesAKeyBelongingToADifferentVault`, `TestVaultRemoveNeverDeletesWithoutConfirming`, `TestVaultRemoveOnAPendingEnrollmentAsksBeforeDeleting`, `TestVaultRemoveLeavesTheVaultsOwnFilesUntouched`, `TestOneRepositoryRegisteredTwiceSharesOneIdentitiesDirectory` | M12b |
| P2 | `entry` (CLI) `TestAlignCatOutputLeavesNonMappingInputAlone`, `TestPadKeyLinesWithNoKeysIsANoOp` | M12b |
| P2 | `entry` (CLI) `TestLsHeaderEmptyVaultNoOutput`, `TestLsHeaderPrintsLabelledPipeDelimitedTable`, `TestLsHeaderRendersIdenticallyInBothModes`, `TestLsDefaultOutputUnchangedByHeaderFeature` | M12b |
| P3 | `generate` CLI `TestGenerateClipAndQRAreMutuallyExclusive`, `TestGenerateClipCopiesTheValueAndPrintsNone`, `TestGenerateQRRendersWithoutPrintingTheValue`; `show` `TestShowClipCopiesTheNamedFieldNotTheValue` | M12c |
| P3 | The `--stdin` half of `repl` `TestSessionCommandArgumentCountsAreChecked` | M12d |

Rows moved out of P6, or excused:

| Row | Destination | Why |
|---|---|---|
| The `HeadCommitterTime` half of `gitrepo/history` `TestLogAndHeadCommitterTimeOnARepoWithNoCommitsYet` | P7 | Its only caller is enrollment's clock-skew check (Go `enroll.go`). M12a ports the `Log` half. |
| `script` `TestTerminalPrompterOfWalksDecoratorChains` | Excused | gage-rs never unwraps a prompter to reach the terminal one ([Q-SCRIPT-PASSPHRASE](../open-questions.md#q-script-passphrase)). M12d adds a test that every wrapper delegates every method it doesn't answer. |

## Process notes

- Branches stack: `main` <- `52-p6-polish` (these docs) <- M12a <- M12b
  <- M12c <- M12d. Each milestone branch is cut from the previous one when
  the user says to start it.
- Tests come first, at task granularity. Each task's tests are written and
  then implemented to green before the next task starts.
- The failure mode to watch is quiet scope narrowing: most of a list done
  and the milestone reported done. Check the boxes and the claimed
  inventory rows, not a summary. A test that turns out not worth writing
  is struck from the list *in the doc*, with a one-line reason.
- **No network, no real clipboard.** `make test` stays offline. No test
  writes the developer's clipboard or needs one to exist; headless CI
  runners have none. The real `arboard` port runs only in production and
  in a manual check recorded in M12c's doc.
- **Every new command or flag** has a registry entry where it's a command,
  the `commands/mod.rs` edits, and a row in a
  `p6_*_have_their_go_fixture_flags_and_descriptions` test, following
  P4's and P5's pattern.
- Each milestone ends green on `make lint` and `make test`, locally and on
  all three CI platforms. New crates also pass `make audit` and the
  `no_frontend_deps` check; any new license is recorded in the `Q-` entry
  that brought the crate in.
- If context runs low mid-milestone, write a `*-sessionhandoff.md` here.
