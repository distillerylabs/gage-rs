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
([plan](p9-parity-signoff/index.md)) closes the ledger: no row may be left `[ ]`.

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
| #3 / PR #4 | Public-readiness: CI, SECURITY, CoC, contributor files | Bootstrap (#1 here) for the docs; M0a for CI; [P9 · S2](p9-parity-signoff/s2-ledger-and-help.md) closes | `[~]` |
| PR #5, #6, #38, #39 | Dependabot bumps (Go modules, actions) | n/a — Rust gets its own dependabot config | n/a |
| #7 / PRs #11, #19 | Recovery key: plan, phase 1 (R0–R2) and phase 2 (R3–R5) | P8 ([plan](p8-recovery-key/index.md), [#74](https://github.com/distillerylabs/gage-rs/issues/74)): RK1–RK4 | `[x]` |
| #8 / PR #13 | R0 recovery key library | P8 · RK1 | `[x]` |
| #9 / PR #14 | R1 recovery key at `gage init` (+ review-finding fixes) | P8 · RK3 | `[x]` |
| #10 / PR #15 | R2 `gage recovery verify` + docs | P8 · RK4 | `[x]` |
| #16 / PR #20 | R3 recovery identity, `RecoverDevice`; retired key can't come back; relative `--recovery-key-out` | P8 · RK2 (the relative-path guard: RK3) | `[x]` |
| #17 / PR #21 | R4 `gage recovery enroll` (+ seven review fixes) | P8 · RK4 (verify's label return: RK1) | `[x]` |
| #18 / PR #22 | R5 `gage recovery rotate`; reserved recovery label | P8: reservation RK1, `RotateRecoveryKey` RK2, `recovery rotate` RK3, docs RK4 | `[x]` |
| PR #12 | Multi-milestone GitHub process in CLAUDE.md | Bootstrap (`CLAUDE.md`); [P9 · S2](p9-parity-signoff/s2-ledger-and-help.md) closes | `[~]` |
| #23 / PR #24 | Parent commands (`vault`, `identity`, `recipient`, `recovery`, `git`, `auth`) list their children in their own help | P1 · M0b (registry/help), each parent's children as they land; [P9 · S2](p9-parity-signoff/s2-ledger-and-help.md) closes with the whole-surface help test | `[~]` M0b: mechanism; M1: real `vault` and `git` parents and their children |
| #25 / PR #26 | Codecov reporting alongside the cover-check gate | M0a; [P9 · S2](p9-parity-signoff/s2-ledger-and-help.md) closes | `[ ]` |
| #27, #28, #29 / PRs #30–#32 | Coverage toward 90% with honest tests; library and CLI coverage gate | M0a sets up the gate; every milestone keeps it; [P9 · S2](p9-parity-signoff/s2-ledger-and-help.md) closes | `[ ]` |
| #34 / PR #35 | gosec configuration audit | M0a — the Rust analogue is the clippy lint set + cargo-deny config; audit it there; [P9 · S2](p9-parity-signoff/s2-ledger-and-help.md) closes with the mapping table | `[ ]` |
| #36 / PR #37 | Pre-commit hook installer running `make lint` | M0a; [P9 · S2](p9-parity-signoff/s2-ledger-and-help.md) closes | `[ ]` |
| #40 / PR #41 | Go toolchain/x/net bump for govulncheck | n/a — Go toolchain; Rust gets `make audit` in M0a | n/a |

## 2. Pre-move work, identified by commit

Issue numbers in this table belong to the repository Go gage lived in
before `distillerylabs/gage`; the commit is the stable reference.

| Go commit | Old ref | Item | Port to | Status |
|---|---|---|---|---|
| `4604dc5`…`4c23f99` | — | Milestones M0–M12 as planned | See §3 | — |
| `db2731f` | #27 | Usage rejection prints the command's own usage block | P1 · M0b | `[x]` (session half: P3 · M6 `[x]` `session_wrong_arg_count_shows_command_usage`, through the interactive session; the `--stdin` spelling is P6 · M12d `[x]`, `stdin_session_command_argument_counts_are_checked`, `stdin_wrong_arg_count_shows_command_usage`) |
| `386c57e` | #38 | `init --remote` solicits an HTTPS token inline | P4 · M8a2 ([plan](p4-sync/m8a2-sync-wiring.md)) | `[x]` asked only for a token-candidate host with no stored token, before the push; blank skips; checked at the loopback recorder (`init_solicits_a_token_when_none_is_stored` and its three siblings) |
| `a0920b0`, `0a1beeb` | #39 | `vault remove` orphaned-identity cleanup (later revised by E0: compare public keys, `Confirm`, default keep) | P6 · M12b | `[x]` `gage::orphan`, `commands/vault.rs` (Opus review of the never-delete rows in the M12b doc) |
| `c433791` | — | `make reset-local-state` developer helper (`scripts/resetlocalstate`) | P1 · M0b (a separate binary, never a `gage` subcommand; needs `xdgpaths`) | `[x]` |
| `ddce8b1` | — | No doubled `gage: gage:` error prefix | P1 · M0b (`errorprefix_test.go`) | `[x]` errors (M0b); warnings: M2's (page lock, identity reuse) and P4 · M8a2's (push after a write, pull on unlock) name gage once through `Prompter::warn` (`no_sync_failure_or_warning_doubles_gages_own_prefix`) |
| `a1572d5` | — | `ls` shows title and `updated_by` | P2 · M4 | `[x]` `render_ls` prints title, short id, created, updated, `updated_by` |
| `a0dcf7c` | — | Resolution prefers title over UUID | P3 · M5 ([plan](p3-query-session/m5-query-resolution.md)) | `[x]` `resolve_query` runs both title stages before either UUID stage; `resolve_exact_title_wins_over_uuid_match`, `cat_resolves_by_exact_title_even_when_query_is_also_a_uuid_substring` |
| `844f880` | — | Memory-locking bug fix | P2 · M2 (regression test) | `[x]` `memlock::LockedBuf` gives each key its own pages; `alloc_gives_each_buffer_its_own_page`, `unlocking_one_buffer_leaves_another_locked` |
| `e52828a` | — | Scrypt work-factor test hook and its guard tests | P2 · M2 | `[x]` identity-file halves (Q-SCRYPT-HOOK); the enrollment halves in P7 · E2 ([plan](p7-enrollment/e2-sealed-request.md), [Q-ENROLL-SCRYPT-HOOK](open-questions.md#q-enroll-scrypt-hook)) |
| `0d710d4` | — | E0 vault-id keying | Folded: M0b (lock path), M1, M2, M10 | `[~]` M0b: id validator and id-keyed path builders; M1: `[vault].id`, format 2 with the directional refusal, the registration-vs-vault id check; M2: identity files filed under the committed id, the `vault-name.txt` marker, `init`'s rollback; M10: the trust cache under `$GAGE_STATE/<vault-id>/`, deleted by `vault remove` from the registration's recorded id (`two_vaults_under_one_local_name_share_neither_directory`) `[x]` |
| `3fd784c`, `fbc887f` | #46 | E1a unconditional re-encrypt + recipient regression fixes | Folded: P5 · M9b ([#43](https://github.com/distillerylabs/gage-rs/issues/43)): `recipient add` has no `--reencrypt` and always re-encrypts; the `RequireFullAccess` pre-flight with its clean-tree gate | `[x]` |
| `045bf24`, `9d72384` | #48 | E1b shared recipient write + fixes | P7 · E1b ([plan](p7-enrollment/e1b-shared-recipient-write.md)) | `[x]` |
| `798ed47` | — | E2 sealed request | P7 · E2 ([plan](p7-enrollment/e2-sealed-request.md)) | `[x]` |
| `c86ad4c` | — | E3 joining side | P7 · E3 ([plan](p7-enrollment/e3-joining-side.md)) | `[x]` |
| `f800af6`, `187c810`, `faf49b9` | #52 | E4 approving side (commit titled "Milestone 4") + fixes, docs | P7 · E4 ([plan](p7-enrollment/e4-approving-side.md)) | `[x]` |
| `a4b989e` | #58 / #59 | `insert --field NAME=VALUE` | P2 · M4 | `[x]` `parse_field_flags` (M4); the `show --field` halves and `--field`/`-e` exclusivity in P3 · M5 |
| `f0e17db`, `f1e391f` | #55 / #60, #61 | `cat`: display-only short id, column-aligned labels, flush-left multi-line values | P6 · M12b | `[x]` `entry::key_layout`, `commands/catfmt.rs` |
| `fb2f1e9` | #54 / #62 | `ls -H/--header` labelled `\|`-delimited table | P6 · M12b | `[x]` `render_ls_table` |
| `0aa325c` | #25 / #63 | M13 session Tab completion and command abbreviation | P3 · M13 ([plan](p3-query-session/m13-tab-completion.md)) | `[x]` |
| `bbb24ba` | #64 / #65 | Coverage and vulnerability reporting | M0a; [P9 · S2](p9-parity-signoff/s2-ledger-and-help.md) closes | `[ ]` |
| `30b21df` | #66 / #67 | Apache-2.0 license; fixture-data cleanup | Bootstrap (`LICENSE`); [P9 · S2](p9-parity-signoff/s2-ledger-and-help.md) closes | `[~]` |
| `df3c0fe` | #4, #5 | Enrollment-design issues resolved in the TDD | Covered by the copied TDD; P7 · E4 closes it with the shipped behavior | `[x]` |
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
| | M9 (+ A19), M10, M11 | P5 ([plan](p5-recipients-trust/index.md), [#41](https://github.com/distillerylabs/gage-rs/issues/41)): M9 → M9a + M9b, M10, M11 | `[x]` M9a ([#42](https://github.com/distillerylabs/gage-rs/issues/42)) `[x]`; M9b ([#43](https://github.com/distillerylabs/gage-rs/issues/43)) `[x]`; M10 ([#44](https://github.com/distillerylabs/gage-rs/issues/44)) `[x]` (38 rows + 3 inbound, both P4 follow-ups; CI green on all three platforms); M11 ([#45](https://github.com/distillerylabs/gage-rs/issues/45)) `[x]` (20 rows; CI green on all three platforms) |
| | M12 (+ post-plan items in §2) | P6 ([plan](p6-polish/index.md), [#52](https://github.com/distillerylabs/gage-rs/issues/52)): M12 → M12a `log`/`history`, M12b `cat`/`ls -H`/orphan cleanup, M12c `-c`/`-q`, M12d `--script`/`--stdin` | M12a ([#53](https://github.com/distillerylabs/gage-rs/issues/53)) `[x]`; M12b ([#54](https://github.com/distillerylabs/gage-rs/issues/54)) `[x]`; M12c ([#55](https://github.com/distillerylabs/gage-rs/issues/55)) `[x]` (`-c`/`-q`: `clipboard.rs`, `qr.rs`, `commands/secretout.rs`; manual clipboard check on macOS); M12d ([#56](https://github.com/distillerylabs/gage-rs/issues/56)) `[x]` (`--script`/`--stdin`: `repl::run_script`, `scriptprompt.rs`, `script_tests.rs`; CI green on Linux, macOS and Windows) |
| | "Post-plan changes" section | Rows in §2 | — |
| `gage-cli-init-design/` | E0, E1a | Folded (see §2) | `[x]` |
| | E1b, E2, E3, E4 | P7 ([plan](p7-enrollment/index.md), [#63](https://github.com/distillerylabs/gage-rs/issues/63)): one to one | `[x]` |
| `recovery-key/` | R0–R5 (D1–D12) | P8 ([plan](p8-recovery-key/index.md), [#74](https://github.com/distillerylabs/gage-rs/issues/74)): regrouped as RK1–RK4 ([Q-P8-SPLIT](open-questions.md#q-p8-split)) | `[x]` |
| `test-coverage/` | R0, R1 (coverage) | M0a gate + every milestone; [P9 · S2](p9-parity-signoff/s2-ledger-and-help.md) closes | `[ ]` |

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
| `root`, `vault`, `vault_list`, `vault_info`, `vault_remove`, `vault_set-default`, `init`, `git`, `git_set-remote` | P1 (`init` flags grow through P2, P4 and P8 — see [Q-INIT-INTERIM](open-questions.md#q-init-interim) and [Q-RECOVERY-INTERIM](open-questions.md#q-recovery-interim); `vault remove` cleanup in P6). `init`'s `--no-recovery-key`/`--recovery-key-out` and its whole long help and flag list: P8 · RK3 `[x]` (checked by `p8_recovery_commands_have_their_go_fixture_flags_and_descriptions`) |
| `insert`, `cat`, `rm`, `ls` | P2 · M4 `[x]` (name, group, arguments and the flags that exist match; `insert -e` P3 · M5 `[x]`; `ls -H` and `cat` formatting are P6 · M12) |
| `show`, `edit`, `rename`, `generate` | P3 · M5 `[x]`; `-c`/`-q` on `show` and `generate` P6 · M12c `[x]`. Every fixture flag, checked by `m5_commands_have_their_go_fixture_flags_and_descriptions` |
| `search`, `reindex` | P3 · M7 `[x]` (name, alias `grep`, group, arguments and `-u/--use` match; checked by `m7_commands_have_their_go_fixture_flags_and_descriptions`) |
| `auth`, `auth_login`, `auth_status`, `auth_logout` | P4 · M8a1 `[x]` (name, group Git-specific, both modes, `--host`, `login`'s long help; checked by `p4_auth_commands_have_their_go_fixture_flags_and_descriptions`) |
| `clone`, `sync`, `pull`, `push` | P4 · M8a2 `[x]` `clone` (one-shot, `--name`/`--dir`/`--device`), `sync`/`pull`/`push` (group Sync, both modes, `-u/--use`), `init --remote` (checked by `p4_sync_commands_have_their_go_fixture_flags_and_descriptions`); `sync`'s resolving half: P4 · M8b `[x]` (the `[l/r/b/s/q]` menu, lazy unlock, refusal with nobody to ask; `sync --yes` stays an unknown flag, pinned by `yes_does_not_resolve_conflicts`) (`clone`'s enrollment offer: P7 · E3 `[x]`, `clone_enroll_tests.rs`); `sync --yes`: P5 · M10, the root flag accepted and the conflict still refused (`yes_does_not_resolve_conflicts`, premise updated) `[x]` |
| `recipient`, `recipient_add`, `recipient_remove`, `recipient_list`, `recipient_verify`, `identity`, `identity_add`, `identity_list`, `mv`, `cp` | P5: `identity*`, `recipient`, `recipient_list`, `recipient_verify` in M9a `[x]` (CI green on all three platforms, matched by `p5_identity_commands_have_their_go_fixture_flags_and_descriptions`; `verify`'s `--repair` prose is M10's); `recipient_add`, `recipient_remove` in M9b `[x]` (matched by the same test); `recipient verify --repair` (full fixture prose and flag) and the root `--yes` (hidden, as Go's help lists no inherited flag) in M10 `[x]`, matched by the same test; `mv`, `cp` in M11 `[x]` (matched by `p5_mv_and_cp_have_their_go_fixture_flags_and_descriptions`) |
| `log`, `history` | P6 · M12a (`log` and `history` registered in group Sync, both modes, `-u/--use`, `history --decrypt`; checked by `p6_log_and_history_have_their_go_fixture_flags_and_descriptions`) |
| The output flags `--clip`, `--qr`, `--script` (`show --field` itself is P3 · M5) | `--clip`, `--qr`: P6 · M12c `[x]`; `--script`, `--stdin`: M12d `[x]`, hidden root flags like `--yes` (`the_script_flags_are_hidden_root_flags`) |
| `identity_enroll`, `recipient_pending`, `recipient_approve`, `recipient_deny` | P7: `identity_enroll` (and `identity`'s child list) in E3 `[x]`; `recipient_pending`, `recipient_approve`, `recipient_deny` (and `recipient`'s child list) in E4 `[x]` (checked by `p7_recipient_enrollment_commands_have_their_go_fixture_flags_and_descriptions`) |
| `recovery`, `recovery_enroll`, `recovery_rotate`, `recovery_verify` | P8: `recovery` (parent) and `recovery_rotate` in RK3 `[x]`; `recovery_enroll` (`--device`, `--replaces`, `--no-new-recovery-key`, `--recovery-key-out`, `-u/--use`) and `recovery_verify` (`-u/--use`), both group Recipients and one-shot only, and the parent's full child list, in RK4 `[x]`; `init`'s `--no-recovery-key`/`--recovery-key-out` in RK3 `[x]`. All checked by `p8_recovery_commands_have_their_go_fixture_flags_and_descriptions` |
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
[P4 plan](p4-sync/index.md#inventory-accounting). *Corrected in P9 · S1:*
P4 planning recorded no rows moving in, but P1 · M1 and P2 · M2 had sent it
three. One was ported unrecorded (`TestResolveConflictNeverGuesses`, M8b).
P7 claimed one (`TestConfirmDefaultYesDefaultsToYes`). One was lost until
S1 ported it (`TestCloneCopiesTheVaultsIDIntoGlobalConfig`).

Resolved in P5 planning: `recipient_batch_test.go` (15 rows) was re-tagged
P7 in the phase map, because it tests E1b's N-recipient write, whose only
consumer is E4 ([Q-E1B-RETAG](open-questions.md#q-e1b-retag)). The other
121 are split M9a (26), M9b (37), M10 (38) and M11 (20). P5 also claims
six rows left by earlier phases: two `vaultid_keying` rows, the
`identity list` half of a third, M13's `identity` completion form and two
`prompter` rows. See the
[P5 plan](p5-recipients-trust/index.md#inventory-accounting).

Resolved in P6 planning: no re-tags. The 87 rows are split M12a (24),
M12b (10), M12c (27) and M12d (26). P6 also claims 23 rows left by
earlier phases: 12 orphan-cleanup rows from P1, 2 `cat` and 4 `ls -H`
rows from P2, and 4 `-c`/`-q` rows plus the `--stdin` half of a session
row from P3. The `HeadCommitterTime` half of
`TestLogAndHeadCommitterTimeOnARepoWithNoCommitsYet` moves to P7
(enrollment is its only caller), and
`TestTerminalPrompterOfWalksDecoratorChains` is excused
([Q-SCRIPT-PASSPHRASE](open-questions.md#q-script-passphrase)). See the
[P6 plan](p6-polish/index.md#inventory-accounting).

Resolved in P7 planning: no re-tags out. The 201 rows are split E1b (15),
E2 (55), E3 (60) and E4 (71), by the function each Go file's tests call;
`enrollprune_test.go` is split between E2 (the primitive) and E4 (pruning
on `recipient add`/`remove`). P7 also claims 4 rows left by earlier
phases: the enrollment halves of two P2 `crypt` guards (E2), P6's
`HeadCommitterTime` half (E3), and P2 `prompter`
`TestConfirmDefaultYesDefaultsToYes` (E3). P2 had sent that row to clone's
offer under P4 · M8a, which kept the offer out of scope and never claimed
it. See the [P7 plan](p7-enrollment/index.md#inventory-accounting).

Resolved in P8 planning: no re-tags, and no rows moved in. The 92 rows
are split RK1 (11), RK2 (25), RK3 (30) and RK4 (26). Earlier phases left
P8 obligations that aren't inventory rows; see the
[P8 plan](p8-recovery-key/index.md#inventory-accounting).

Resolved in P9 · S1 ([Q-INVENTORY-GUARD](open-questions.md#q-inventory-guard)):

- **The guard.** `crates/gage-cli/tests/parity_inventory.rs` runs in
  `make test` on every platform.
  - A row passes when its normalized Go name matches a Rust `#[test]`
    fn, or when it has a row in `tests/parity/inventory-exceptions.tsv`.
  - The guard also fails on an exception that names a test which
    doesn't exist, one that already name-matches, one whose key isn't in
    the inventory, and one whose `ref` doesn't resolve to a `Q-` anchor
    or a plan doc.
  - Each of those failures was produced once by hand (S1, task 2).
- **The audit.** S1 read all 95 rows that didn't name-match against the
  Go test at the pinned SHA.
  - 76 are `ported` under other names.
  - 18 are `excused`: language guarantees, compile-time properties
    proven by `compile_fail` doctests, Go-only build metadata and test
    helpers, and one CI step (`make stamp-check`).
- **Ported late, against P1:** `vaultid_keying`
  `TestCloneCopiesTheVaultsIDIntoGlobalConfig`. Nothing checked that a
  clone inherits the vault's committed id. It now name-matches, so it
  has no exception row.
- **Ports tightened to match Go's assertions:**
  - `bad_field_flags_are_usage_errors_before_unlock` now checks that each
    `--field` refusal names its own reason (five `insert_field` rows).
  - `conflict_prompt_re_asks_on_an_unknown_answer` now checks the re-ask
    count (`TestResolveConflictNeverGuesses`).
- **Accounting fixes:**
  - The P4 paragraph above.
  - M1's task 5 heading now counts its 2 `vaultid_keying` rows.
  - This P8 paragraph.

  Every phase's milestone split adds up to its row count in the table
  above.

## 6. User docs (Go `doc/cli/`)

gage-rs ports these as the behavior lands, so docs describe shipped
behavior; each phase's last milestone updates the pages for its surface.

| Go page | Port to |
|---|---|
| `vaults.md`, `configuration.md`, `exit-codes.md` | P1 · M1: `doc/cli/`; P6 · M12d `[x]` `vault remove`'s identity-file cleanup, the `GAGE_PASSPHRASE` row and `[shell].clipboard_timeout`, exit 5 and the `source:line:` prefix for script runs; P8 · RK4 `[x]` `init`'s recovery key (`vaults.md`) and the `recovery` commands' exit meanings (`exit-codes.md`) |
| `entries.md` | P2 · M4: `doc/cli/entries.md` `[x]` for the shipped surface; P3 · M5 `[x]` `show`, `edit`, `rename`, `generate`, `insert -e`, addressing; P3 · M7 `[x]` `search`/`grep`, `reindex`; P6 · M12d `[x]` `cat`/`ls -H`, `-c`/`-q`, `log`, `history --decrypt` |
| `addressing-entries.md` | P3 · M5: `doc/cli/addressing-entries.md` `[x]` for one-shot; M6 `[x]` the session picker; M7 `[x]` the index; M13 `[x]` the title completion source; P7 · E4 `[x]` request ids (Go completes none: `request_id_positionals_complete_nothing`) |
| `session-mode.md` | P3 · M6: `doc/cli/session-mode.md` `[x]` for the interactive session; M7 `[x]` the metadata index and `reindex`; M13 `[x]` completion and abbreviation; M12d `[x]` `--script`/`--stdin`, `GAGE_PASSPHRASE` and `-c` in a session; enrollment's session behavior P7 · E4 `[x]` |
| `sync.md`, `git-and-auth.md` | P4 · M8b `[x]` (with `exit-codes.md`'s `Unreachable` and sync meanings of 1 and 5, and `configuration.md`'s tokens directory); describes P4 only: the recipient-change confirmation and enrollment exceptions arrive with P5 and P7 (E4 `[x]`) |
| `identities-and-recipients.md`, `sharing.md` | P5 · M11 `[x]` written (identities, recipients, `verify`/`--repair`, the trust cache and `--yes`, `mv`/`cp`); P6 · M12d `[x]` `--yes` in a script session; P7 · E4 `[x]` enrollment (`sharing.md` unchanged: Go's page has no enrollment content), P8 · RK4 `[x]` the recovery sections ("Storing your recovery key", "Checking it", "Using it", "If it leaks", "Skipping it") |
| `enrollment.md` | P7 · E4 `[x]` |
| `README.md`, `getting-started.md`, `command-reference.md` | Grown each phase; final pass in [P9 · S3](p9-parity-signoff/s3-user-docs.md) |

## Exclusions

| Item | Reason |
|---|---|
| TOTP (`gage-cli-totp.md`) | Designed, never built in Go — [Q-TOTP](open-questions.md#q-totp) |
| Go-specific tooling: golangci-lint/forbidigo config, govulncheck, go-test-coverage, `GOPROXY=off` | Replaced by Rust equivalents in M0a (clippy `disallowed-*`, cargo-deny, cargo-llvm-cov, `CARGO_NET_OFFLINE`); the *properties* they enforce are ported, the tools are not |
| Byte-level and cross-binary compatibility: Go's TOML/YAML bytes, Cobra's help layout and quirks, reading Go-written vaults | Behavioral parity only — [Q-COMPAT](open-questions.md#q-compat) |
| Go module path, Go toolchain bumps | Go-only |
