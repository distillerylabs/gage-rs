# Parity ledger

[← port plan](index.md)

Everything Go gage shipped, as of the pinned commit
[`64b6cf0`](https://github.com/distillerylabs/gage/commit/64b6cf078f3e756efb9d868cc1518dc1d9060748)
([`tests/fixtures/go/GO_GAGE_SHA`](../../../../../tests/fixtures/go/GO_GAGE_SHA)),
mapped to the gage-rs milestone that ports it or to an explicit exclusion.

**Why this exists.** The Go milestone index is not a complete inventory: the
recovery key (Go #7, six milestones, four commands) shipped through its own
plan directory and is absent from that index, and several user-visible
changes (`cat` formatting, parent-command help) appear in no milestone's
plan at all. So this ledger draws on five independent sources, and an item
any one of them knows about must have a row here:

1. Go's GitHub issues and merged PRs (§1)
2. Go's git history, which predates the current repository (§2,
   [Q-GO-ISSUE-REFS](open-questions.md#q-go-issue-refs))
3. Go's plan directories (§3)
4. The help surface captured from the Go binary (§4,
   `tests/fixtures/go/help/`)
5. The Go test inventory (§5, `tests/fixtures/go/test-inventory.tsv`)

Go's user docs (`doc/cli/`) are a sixth, cross-checking view (§6).

**How it's used.** Planning a phase starts from its rows here and its rows in
the test inventory. Each milestone doc lists the rows it claims under **Go
sources**; the status column is updated when that milestone merges. P9
closes the ledger: no row may be left `[ ]`.

**Moving the pin.** Run `scripts/go-reference/capture.sh <go-checkout>`, then
add rows for every Go issue, PR and commit since the previous pin. A test
file the phase map doesn't know fails the script.

Status: `[ ]` not ported, `[~]` in progress, `[x]` ported, `n/a` excluded
(the reason is in the row).

---

## 1. GitHub issues and merged PRs (`distillerylabs/gage`)

| Go ref | Item | Port to | Status |
|---|---|---|---|
| #1 / PR #2 | Module path `github.com/distillerylabs/gage` | n/a — Go module path; the crate/package names are decided in M0a | n/a |
| #3 / PR #4 | Public-readiness: CI, SECURITY, CoC, contributor files | Bootstrap (#1 here) for the docs; M0a for CI | `[~]` |
| PR #5, #6, #38, #39 | Dependabot bumps (Go modules, actions) | n/a — Rust gets its own dependabot config | n/a |
| #7 / PRs #11, #19 | Recovery key: plan, phase 1 (R0–R2) and phase 2 (R3–R5) | P8 | `[ ]` |
| #8 / PR #13 | R0 recovery key library | P8 | `[ ]` |
| #9 / PR #14 | R1 recovery key at `gage init` (+ review-finding fixes) | P8 | `[ ]` |
| #10 / PR #15 | R2 `gage recovery verify` + docs | P8 | `[ ]` |
| #16 / PR #20 | R3 recovery identity, `RecoverDevice`; retired key can't come back; relative `--recovery-key-out` | P8 | `[ ]` |
| #17 / PR #21 | R4 `gage recovery enroll` (+ seven review fixes) | P8 | `[ ]` |
| #18 / PR #22 | R5 `gage recovery rotate`; reserved recovery label | P8 | `[ ]` |
| PR #12 | Multi-milestone GitHub process in CLAUDE.md | Bootstrap (`CLAUDE.md`) | `[~]` |
| #23 / PR #24 | Parent commands (`vault`, `identity`, `recipient`, `recovery`, `git`, `auth`) list their children in their own help | P1 · M0b (registry/help), each parent's children as they land | `[~]` M0b: mechanism; M1: real `vault` and `git` parents and their children |
| #25 / PR #26 | Codecov reporting alongside the cover-check gate | M0a | `[ ]` |
| #27, #28, #29 / PRs #30–#32 | Coverage toward 90% with honest tests; library and CLI coverage gate | M0a sets up the gate; every milestone keeps it | `[ ]` |
| #34 / PR #35 | gosec configuration audit | M0a — the Rust analogue is the clippy lint set + cargo-deny config; audit it there | `[ ]` |
| #36 / PR #37 | Pre-commit hook installer running `make lint` | M0a | `[ ]` |
| #40 / PR #41 | Go toolchain/x/net bump for govulncheck | n/a — Go toolchain; Rust gets `make audit` in M0a | n/a |

## 2. Pre-move work, identified by commit

Issue numbers in this table belong to the repository Go gage lived in
before `distillerylabs/gage`; the commit is the stable reference.

| Go commit | Old ref | Item | Port to | Status |
|---|---|---|---|---|
| `4604dc5`…`4c23f99` | — | Milestones M0–M12 as planned | See §3 | — |
| `db2731f` | #27 | Usage rejection prints the command's own usage block | P1 · M0b | `[x]` (session half: P3 · M6 `[x]` `session_wrong_arg_count_shows_command_usage`, through the interactive session; the `--stdin` spelling is P6 · M12's) |
| `386c57e` | #38 | `init --remote` solicits an HTTPS token inline | P4 · M8a2 ([plan](p4-sync/m8a2-sync-wiring.md)) | `[x]` asked only for a token-candidate host with no stored token, before the push; blank skips; checked at the loopback recorder (`init_solicits_a_token_when_none_is_stored` and its three siblings) |
| `a0920b0`, `0a1beeb` | #39 | `vault remove` orphaned-identity cleanup (later revised by E0: compare public keys, `Confirm`, default keep) | P6 · M12 | `[ ]` |
| `c433791` | — | `make reset-local-state` developer helper (`scripts/resetlocalstate`) | P1 · M0b (a separate binary, never a `gage` subcommand; needs `xdgpaths`) | `[x]` |
| `ddce8b1` | — | No doubled `gage: gage:` error prefix | P1 · M0b (`errorprefix_test.go`) | `[x]` errors (M0b); warnings: M2's (page lock, identity reuse) and P4 · M8a2's (push after a write, pull on unlock) name gage once through `Prompter::warn` (`no_sync_failure_or_warning_doubles_gages_own_prefix`) |
| `a1572d5` | — | `ls` shows title and `updated_by` | P2 · M4 | `[x]` `render_ls` prints title, short id, created, updated, `updated_by` |
| `a0dcf7c` | — | Resolution prefers title over UUID | P3 · M5 ([plan](p3-query-session/m5-query-resolution.md)) | `[x]` `resolve_query` runs both title stages before either UUID stage; `resolve_exact_title_wins_over_uuid_match`, `cat_resolves_by_exact_title_even_when_query_is_also_a_uuid_substring` |
| `844f880` | — | Memory-locking bug fix | P2 · M2 (regression test) | `[x]` `memlock::LockedBuf` gives each key its own pages; `alloc_gives_each_buffer_its_own_page`, `unlocking_one_buffer_leaves_another_locked` |
| `e52828a` | — | Scrypt work-factor test hook and its guard tests | P2 · M2 | `[~]` identity-file halves done (Q-SCRYPT-HOOK); the enrollment halves are P7's |
| `0d710d4` | — | E0 vault-id keying | Folded: M0b (lock path), M1, M2, M10 | `[~]` M0b: id validator and id-keyed path builders; M1: `[vault].id`, format 2 with the directional refusal, the registration-vs-vault id check; M2: identity files filed under the committed id, the `vault-name.txt` marker, `init`'s rollback |
| `3fd784c`, `fbc887f` | #46 | E1a unconditional re-encrypt + recipient regression fixes | Folded: P5 · M9 | `[ ]` |
| `045bf24`, `9d72384` | #48 | E1b shared recipient write + fixes | P7 · E1b | `[ ]` |
| `798ed47` | — | E2 sealed request | P7 · E2 | `[ ]` |
| `c86ad4c` | — | E3 joining side | P7 · E3 | `[ ]` |
| `f800af6`, `187c810`, `faf49b9` | #52 | E4 approving side (commit titled "Milestone 4") + fixes, docs | P7 · E4 | `[ ]` |
| `a4b989e` | #58 / #59 | `insert --field NAME=VALUE` | P2 · M4 | `[x]` `parse_field_flags` (M4); the `show --field` halves and `--field`/`-e` exclusivity in P3 · M5 |
| `f0e17db`, `f1e391f` | #55 / #60, #61 | `cat`: display-only short id, column-aligned labels, flush-left multi-line values | P6 · M12 | `[ ]` |
| `fb2f1e9` | #54 / #62 | `ls -H/--header` labelled `\|`-delimited table | P6 · M12 | `[ ]` |
| `0aa325c` | #25 / #63 | M13 session Tab completion and command abbreviation | P3 · M13 ([plan](p3-query-session/m13-tab-completion.md)) | `[x]` |
| `bbb24ba` | #64 / #65 | Coverage and vulnerability reporting | M0a | `[ ]` |
| `30b21df` | #66 / #67 | Apache-2.0 license; fixture-data cleanup | Bootstrap (`LICENSE`) | `[~]` |
| `df3c0fe` | #4, #5 | Enrollment-design issues resolved in the TDD | Covered by the copied TDD; P7 | `[ ]` |
| `6aa019a` | — | TOTP design begun | n/a — [Q-TOTP](open-questions.md#q-totp) | n/a |

## 3. Plan directories (Go `doc/implementation/00_gage-cli/plans/`)

| Go plan | Milestones | Port to | Status |
|---|---|---|---|
| `gage-cli-design/` | M0 | P1 · M0a + M0b | `[x]` |
| | M1 | P1 · M1 (+ E0) | `[x]` |
| | M2 | P2 · M2 (+ E0 identity paths) | `[x]` ([#13](https://github.com/distillerylabs/gage-rs/issues/13)) |
| | M3 | P2 · M3 | `[x]` ([#14](https://github.com/distillerylabs/gage-rs/issues/14)) |
| | M4 | P2 · M4 | `[x]` ([#15](https://github.com/distillerylabs/gage-rs/issues/15)) |
| | M5, M6, M7, M13 | P3 ([#21](https://github.com/distillerylabs/gage-rs/issues/21)) | `[x]` |
| | M8a, M8b | P4 ([#32](https://github.com/distillerylabs/gage-rs/issues/32)): M8a → M8a1 + M8a2, M8b → M8b | `[x]` M8a1 ([#33](https://github.com/distillerylabs/gage-rs/issues/33)), M8a2 ([#34](https://github.com/distillerylabs/gage-rs/issues/34)), M8b ([#35](https://github.com/distillerylabs/gage-rs/issues/35)) |
| | M9 (+ A19), M10, M11 | P5 ([plan](p5-recipients-trust/index.md), [#41](https://github.com/distillerylabs/gage-rs/issues/41)): M9 → M9a + M9b, M10, M11 | `[ ]` |
| | M12 (+ post-plan items in §2) | P6 | `[ ]` |
| | "Post-plan changes" section | Rows in §2 | — |
| `gage-cli-init-design/` | E0, E1a | Folded (see §2) | `[ ]` |
| | E1b, E2, E3, E4 | P7 | `[ ]` |
| `recovery-key/` | R0–R5 (D1–D12) | P8 | `[ ]` |
| `test-coverage/` | R0, R1 (coverage) | M0a gate + every milestone | `[ ]` |

## 4. Help surface (`tests/fixtures/go/help/`)

One fixture per command, captured from the Go binary. Under behavioral
parity they are a **reference, not a byte gate**
([Q-HELP-RENDER](open-questions.md#q-help-render)): a command is ported
when it exists with the same name, aliases, group, arguments and flags as
its fixture (flags that land later, per the phase, excepted), and its help
says what it does. gage-rs's help is rendered by clap and is not expected
to match Cobra's layout.

| Fixtures | Port to |
|---|---|
| `root`, `vault`, `vault_list`, `vault_info`, `vault_remove`, `vault_set-default`, `init`, `git`, `git_set-remote` | P1 (`init` flags grow through P2, P4 and P8 — see [Q-INIT-INTERIM](open-questions.md#q-init-interim) and [Q-RECOVERY-INTERIM](open-questions.md#q-recovery-interim); `vault remove` cleanup in P6) |
| `insert`, `cat`, `rm`, `ls` | P2 · M4 `[x]` (name, group, arguments and the flags that exist match; `insert -e` P3 · M5 `[x]`; `ls -H` and `cat` formatting are P6 · M12) |
| `show`, `edit`, `rename`, `generate` | P3 · M5 `[x]` (all fixture flags but `-c`/`-q` on `show` and `generate`, which are P6 · M12; checked by `m5_commands_have_their_go_fixture_flags_and_descriptions`) |
| `search`, `reindex` | P3 · M7 `[x]` (name, alias `grep`, group, arguments and `-u/--use` match; checked by `m7_commands_have_their_go_fixture_flags_and_descriptions`) |
| `auth`, `auth_login`, `auth_status`, `auth_logout` | P4 · M8a1 `[x]` (name, group Git-specific, both modes, `--host`, `login`'s long help; checked by `p4_auth_commands_have_their_go_fixture_flags_and_descriptions`) |
| `clone`, `sync`, `pull`, `push` | P4 · M8a2 `[x]` `clone` (one-shot, `--name`/`--dir`/`--device`), `sync`/`pull`/`push` (group Sync, both modes, `-u/--use`), `init --remote` (checked by `p4_sync_commands_have_their_go_fixture_flags_and_descriptions`); `sync`'s resolving half: P4 · M8b `[x]` (the `[l/r/b/s/q]` menu, lazy unlock, refusal with nobody to ask; `sync --yes` stays an unknown flag, pinned by `yes_does_not_resolve_conflicts`) (`clone`'s enrollment offer is P7's; `sync --yes` arrives with P5 · M10) |
| `recipient`, `recipient_add`, `recipient_remove`, `recipient_list`, `recipient_verify`, `identity`, `identity_add`, `identity_list`, `mv`, `cp` | P5: `identity*`, `recipient`, `recipient_list`, `recipient_verify` in M9a; `recipient_add`, `recipient_remove` in M9b; `recipient verify --repair` and the root `--yes` (with `sync --yes`'s meaning) in M10; `mv`, `cp` in M11 |
| `log`, `history` (and the output flags `--clip`, `--qr`, `--script`; `show --field` itself is P3 · M5) | P6 |
| `identity_enroll`, `recipient_pending`, `recipient_approve`, `recipient_deny` | P7 |
| `recovery`, `recovery_enroll`, `recovery_rotate`, `recovery_verify` | P8 |
| Session-mode `help` and session-only commands | P3 · M6 `[x]` — tested against the registry, not captured (`repl_tests`: `session_help_matches_registry_exactly`, `both_help_surfaces_render_the_same_registry` and the rest of Go's `sessionhelp` rows) |

## 5. Test inventory (`tests/fixtures/go/test-inventory.tsv`)

All 1,271 Go test functions (`Test*`, including both `TestMain`s), each
tagged with the phase that owns its file by
`scripts/go-reference/phase-map.awk`. Phase tags are coarse: a milestone
doc claims individual rows, and moving a row to another phase is a
deliberate edit to the phase map. Each row is ported (by name or by
an equivalent Rust test the milestone doc names) or excused in writing.

| Phase | Go test functions | Main Go test files |
|---|---|---|
| P1 | 186 | `exitcode`, `atomicfile`, `vaultlock`, `xdgpaths`, `config`, `devicename`, `vaultconfig`, `gittest`; `vault_lifecycle`, `vaultid_keying`, `vault_create`, `help`, `registry`, `usageerror`, `errorprefix`, `version`, `main_integration` |
| P2 | 193 | `agekey`, `memlock`, `vault` (`Identity` close and page lock), `crypt`, `unlock`, `identityfile_marker`, `init_identity`, `prompter`(`_pty`), `coredump`, `testsupport`; `entry`, `field`, `entry_crud`, `insert_field`, `insert_multiline_pty` |
| P3 | 225 | `resolve`(`_cli`), `generate`, `rename`, `edit`, `show`, `session`, `shell`, `repl`, `session_pty`, `sessionhelp`, `index`, `search`, `complete`(`_pty`); `entry_edit`, `insert_edit`, `editor`, `scratchdir*` |
| P4 | 166 | `gitrepo/*` (less `history`), `remoteauth`, `syncerr`, `remotesync`, `sync_index`, `syncresolve`(`_cli`), `sync_cli`, `clone`, `auth`, `init_remote_auth` |
| P5 | 121 | `recipients`, `recipient`, `reencrypt`, `recipient_cli`, `identity_add`, `identity_cli`, `trustcache`(`_cli`), `move` |
| P6 | 87 | `clip`, `qr`, `log`, `history`, `script`, `cat`, `gitrepo/history` |
| P7 | 201 | `enroll*`, `enrollment`, `approve_cli`, `approve_scope_cli`, `clone_enroll`, `enroll_cli`, `enroll_prompter`, `recipient_batch` (E1b) |
| P8 | 92 | `recovery_key`, `recovery_enroll`, `recovery`, `init_recoverykey` |

Known coarse spots to resolve when the phase is planned:
`vault_lifecycle_test.go` and `vaultid_keying_test.go` (P1) hold #39
orphan-cleanup tests that belong to P6 · M12; `init_*` files split across
phases by flag.

Resolved in P1 planning: the P1 rows of `vault_lifecycle_test.go` and
`vaultid_keying_test.go` that belong elsewhere (P2, P4, P5, P6) are listed
with their destination in [M1's doc](p1-foundation/m1-vault-lifecycle.md);
`vault_test.go` was re-tagged P2 in the phase map (it tests `Identity`
close and page locking); and M1 claims 7 P4-tagged rows of
`gitrepo_test.go` (init-and-commit, set-remote, remote URL, `IsClean`)
because it builds that surface.

Resolved in M1: M1 also claims the 5 P2-tagged `agekey_test.go` rows,
because it ports `agekey` for recipient validation
([Q-AGE-CRATE](open-questions.md#q-age-crate)).

Resolved in P2 planning: `entry_edit`, `insert_edit`, `editor` and
`scratchdir*` were re-tagged P3 (Go's M5 owns `edit`, `rename` and the
`$EDITOR` round trip), and `cat_test.go` was re-tagged P6 (it holds only
#55's `cat` display tests). The P2 rows that belong elsewhere, including
`entry_test.go`'s `ls -H` tests and the `prompter_test.go` rows for later
phases' questions, are listed with their destination in the
[P2 plan](p2-crypto-entries/index.md#inventory-accounting).

Resolved in P3 planning: no re-tags. Four P3 rows (`generate` clip/QR
×3, `show --clip`) and the `--stdin` half of
`TestSessionCommandArgumentCountsAreChecked` move to P6 · M12, and the
4 `TestEntryTitleCandidates*` rows of the library `session_test.go` are
M13's; see the [P3 plan](p3-query-session/index.md#inventory-accounting).

Resolved in P4 planning: `gitrepo/history_test.go` (6 rows) was
re-tagged P6 in the phase map, because Go's `gitrepo.Log` shipped with Go
M12 for `log`/`history`. M1 already claims 7 of P4's 166 rows. The other
159 are split M8a1 (57), M8a2 (53) and M8b (49, `TestPluralEntries` having
moved from M8a2), with M8b excusing one
(`TestNilPrompterRefusesRatherThanPanicking`, per
[Q-CONFLICT-PROMPTER](open-questions.md#q-conflict-prompter)). See the
[P4 plan](p4-sync/index.md#inventory-accounting).

Resolved in P5 planning: `recipient_batch_test.go` (15 rows) was re-tagged
P7 in the phase map, because it tests E1b's N-recipient write, whose only
consumer is E4 ([Q-E1B-RETAG](open-questions.md#q-e1b-retag)). The other
121 are split M9a (26), M9b (37), M10 (38) and M11 (20). P5 also claims
six rows left by earlier phases: two `vaultid_keying` rows, the
`identity list` half of a third, M13's `identity` completion form and two
`prompter` rows. See the
[P5 plan](p5-recipients-trust/index.md#inventory-accounting).

## 6. User docs (Go `doc/cli/`)

gage-rs ports these as the behavior lands, so docs describe shipped
behavior; each phase's last milestone updates the pages for its surface.

| Go page | Port to |
|---|---|
| `vaults.md`, `configuration.md`, `exit-codes.md` | P1 · M1: `doc/cli/` |
| `entries.md` | P2 · M4: `doc/cli/entries.md` `[x]` for the shipped surface; P3 · M5 `[x]` `show`, `edit`, `rename`, `generate`, `insert -e`, addressing; P3 · M7 `[x]` `search`/`grep`, `reindex`; P6 for `cat`/`ls` display |
| `addressing-entries.md` | P3 · M5: `doc/cli/addressing-entries.md` `[x]` for one-shot; M6 `[x]` the session picker; M7 `[x]` the index; M13 `[x]` the title completion source (enrollment ids are P7's) |
| `session-mode.md` | P3 · M6: `doc/cli/session-mode.md` `[x]` for the interactive session; M7 `[x]` the metadata index and `reindex`; M13 `[x]` completion and abbreviation; `--script`/`--stdin` and `GAGE_PASSPHRASE` are P6 · M12's, enrollment's session behavior P7's |
| `sync.md`, `git-and-auth.md` | P4 · M8b `[x]` (with `exit-codes.md`'s `Unreachable` and sync meanings of 1 and 5, and `configuration.md`'s tokens directory); describes P4 only: the recipient-change confirmation and enrollment exceptions arrive with P5 and P7 |
| `identities-and-recipients.md`, `sharing.md` | P5 · M11 (+ P7 for enrollment, P8 for the recovery sections) |
| `enrollment.md` | P7 |
| `README.md`, `getting-started.md`, `command-reference.md` | Grown each phase; final pass in P9 |

## Exclusions

| Item | Reason |
|---|---|
| TOTP (`gage-cli-totp.md`) | Designed, never built in Go — [Q-TOTP](open-questions.md#q-totp) |
| Go-specific tooling: golangci-lint/forbidigo config, govulncheck, go-test-coverage, `GOPROXY=off` | Replaced by Rust equivalents in M0a (clippy `disallowed-*`, cargo-deny, cargo-llvm-cov, `CARGO_NET_OFFLINE`); the *properties* they enforce are ported, the tools are not |
| Byte-level and cross-binary compatibility: Go's TOML/YAML bytes, Cobra's help layout and quirks, reading Go-written vaults | Behavioral parity only — [Q-COMPAT](open-questions.md#q-compat) |
| Go module path, Go toolchain bumps | Go-only |
