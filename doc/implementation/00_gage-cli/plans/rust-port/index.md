# Rust port — implementation plan

Rolling, phased plan for porting [Go gage](https://github.com/distillerylabs/gage)
to Rust with **behavioral parity** ([Q-COMPAT](open-questions.md#q-compat)):
same behavior and on-disk schema, no byte-level or cross-binary
compatibility promise. The spec is the two copied TDDs,
[gage-cli-design.md](../../tdds/gage-cli-design.md) and
[gage-cli-init-design.md](../../tdds/gage-cli-init-design.md); the Go binary at
the pinned SHA ([`tests/fixtures/go/GO_GAGE_SHA`](../../../../../tests/fixtures/go/GO_GAGE_SHA))
is the reference implementation for behavior.

This file is the shared ground every phase stands on: the phase table, the
Go-to-Rust milestone mapping, the layering and testing rules, the crate
choices, and the contracts milestones publish to each other. Per-phase
detail lives in `plans/rust-port/<phase-slug>/` once that phase is planned.

Companion files:
- [parity-ledger.md](parity-ledger.md) — everything Go shipped, mapped to a
  phase. Phase planning starts here.
- [open-questions.md](open-questions.md) — the Rust-specific decision
  register. The Go register
  ([open-questions.md](https://github.com/distillerylabs/gage/blob/main/doc/implementation/00_gage-cli/plans/gage-cli-design/open-questions.md))
  still holds every inherited decision and is not copied.

Status legend: `[ ]` not started, `[~]` in progress, `[x]` done.

---

## Port the current state, not the history

The Go plans are a record of how Go gage got built, including amendments
that landed after the milestone they amend: A19 (`recipient add` always
re-encrypts, shipped as E1a), A20 (vault-id keying, shipped as E0), and the
post-plan changes. gage-rs ports **what the Go code does now**:

- `format_version = 2` and `[vault].id` from M1 onward; there is no v1.
- Identities, the vault lock and the trust cache are keyed by vault id from
  the first milestone that creates them.
- `recipient add` has no `--reencrypt` flag and always re-encrypts, with the
  `CannotGrantFullAccess` pre-flight, from M9 onward.
- Post-plan behavior is folded into the milestone that owns the surface (see
  the mapping below).

So E0 and E1a have no Rust milestone of their own; their tests land in the
milestones that own the surfaces they changed.

**Interim gaps in `init`:** it requires `--recipient` until M2 and gains
`--remote` in M8a ([Q-INIT-INTERIM](open-questions.md#q-init-interim)), and
has no recovery key until P8, since that depends on M9 and the enrollment
phase ([Q-RECOVERY-INTERIM](open-questions.md#q-recovery-interim)). There is
no installed base, so nothing depends on the gaps.

---

## Phases

Each phase is one parent issue with 2–4 milestone sub-issues, filed when the
phase starts (see CLAUDE.md). Milestones keep their Go IDs wherever the
mapping is one to one, so Go plan docs, the ledger and the test inventory
line up.

| Phase | Milestones | Status | Model | Depends on |
|---|---|---|---|---|
| P1 Foundation ([plan](p1-foundation/index.md), [#4](https://github.com/distillerylabs/gage-rs/issues/4)) | M0a toolchain, CI, lint purity · M0b git-crate spike + primitives · M1 vault lifecycle | `[~]` planned | Sonnet; **Opus** for M0b's git-crate spike and an Opus review of M0b's tests ⚑ | — |
| P2 Crypto & entries | M2 identity + memlock · M3 entry format · M4 CRUD | `[ ]` | **Opus** for M2 and the P2 crate decisions (Q-YAML-CRATE, Q-PLATFORM-CRATES, rest of Q-AGE-CRATE) ⚑; Sonnet for M3/M4 | P1 |
| P3 Query & session | M5 resolver · M6 session/REPL · M7 metadata index · M13 completion | `[ ]` | Sonnet* | P2 |
| P4 Sync | M8a transport & detection · M8b conflict resolution | `[ ]` | **Opus** | P3 |
| P5 Recipients & trust | M9 recipients · M10 trust cache · M11 cross-vault | `[ ]` | **Opus** for M9; Sonnet* for M10/M11 | P4 |
| P6 Polish | M12 output modes, `log`/`history`, `--script` | `[ ]` | Sonnet | P5 |
| P7 Enrollment | E1b · E2 · E3 · E4 | `[ ]` | **Opus** | P5 |
| P8 Recovery key | R0–R5, regrouped into 3–4 milestones | `[ ]` | **Opus** for the R1/R3/R4 material | P7; P6 (R1 displays the key through M12's QR renderer) |
| P9 Parity sign-off | Ledger closed, every test-inventory row ported or excused, user docs complete | `[ ]` | **Opus** review | P8 |

`⚑` = run an Opus review pass over that milestone's *tests* before moving
on, even where Sonnet wrote them. `*` = borderline; start on Sonnet, switch
if it turns awkward. **Switch models at milestone boundaries, not within
them**: task-level TDD has no clean tests-then-implementation seam.

**The failure mode to watch** is quiet scope narrowing — most of a
milestone's test list implemented and the milestone reported done. Check
against the doc and the claimed test-inventory rows, not against a summary.

The P1–P6 order follows Go's dependency graph. P3 before P4 because M8a must
invalidate M7's index after a pull. M10 is in P5 because M11 runs its check
against the destination vault. P6 after P5 because `--script` and `history`
cover commands from every earlier phase.

---

## Go → Rust milestone mapping

| Go source | Rust milestone | Notes |
|---|---|---|
| M0 Scaffolding | P1 · M0a + M0b | Split because Cargo/CI/lint purity and the primitives are separately reviewable. #23 (parent-command help) and #27 (usage-error help) fold into M0b's registry/help tests. |
| M1 Vault lifecycle | P1 · M1 | Includes E0's vault-id keying, `format_version = 2`, id validation, and the `[vaults.<name>].id` field. |
| M2 Identity & memory protection | P2 · M2 | Includes E0's identity path and marker file; the scrypt work-factor guard tests. |
| M3 Entry format | P2 · M3 | Same entry schema as Go; faithful round trip, not Go's YAML bytes ([Q-YAML-CRATE](open-questions.md#q-yaml-crate)). |
| M4 CRUD | P2 · M4 | Includes `insert --field` (#58). |
| M5 Query resolution | P3 · M5 | |
| M6 Session mode | P3 · M6 | |
| M7 Metadata index | P3 · M7 | |
| M13 Tab completion | P3 · M13 | Depends only on M0/M6/M7 in Go; placed with them. |
| M8a Sync transport | P4 · M8a | Includes `init --remote` inline token (#38). |
| M8b Sync conflicts | P4 · M8b | |
| M9 Recipients | P5 · M9 | A19/E1a folded in: no `--reencrypt` on `add`, `RequireFullAccess`. |
| M10 Trust cache | P5 · M10 | Vault-id-keyed path from the start. |
| M11 Cross-vault sharing | P5 · M11 | |
| M12 Polish | P6 · M12 | Includes `vault remove` orphaned-identity cleanup by public key with `Confirm` (#39 as revised by E0), `ls -H/--header` (#54), and `cat`'s display-only short id, label alignment and flush-left multi-line values (#55) — Go records both under M12. |
| E0 Vault-id keying | — | Folded into M1/M2/M10 and the vault lock (M0b/M1). |
| E1a Unconditional re-encrypt | — | Folded into M9. |
| E1b–E4 Enrollment | P7 · E1b, E2, E3, E4 | |
| R0–R5 Recovery key | P8 | Go issue #7; sources are `plans/recovery-key/` and the TDD's "Recovery key at `init`". **Not listed in the Go core index.** |
| Go test-coverage plan (R0/R1 coverage) | M0a | Coverage gating via `cargo-llvm-cov` from the first milestone, applied by every milestone after it. |
| Pre-commit hook | M0a | Developer tooling, not CLI surface. |
| `make reset-local-state` | M0b | Developer tooling; needs M0b's `xdgpaths`, so not M0a. |
| TOTP TDD | — | Designed, never built in Go. Out of scope; see the ledger. |

---

## Layering rules

Every milestone builds its logic into `crates/gage` first and wires
`crates/gage-cli` on top as a thin frontend — see the design doc's "Library
architecture". Concretely, enforced from M0a:

- `crates/gage` never uses the print macros or `dbg!`, never calls
  `std::io::stdin`/`stdout`/`stderr`, never calls `std::process::exit`
  (`clippy.toml` `disallowed-macros`/`disallowed-methods`, denied at crate
  level; tests opt out locally).
- `crates/gage` depends on no frontend crate (clap, terminal input,
  readline, clipboard, QR); a CI check over `cargo metadata` enforces it.
- Anything needing a human decision returns a typed value; the CLI renders it.
- `Identity` is a parameter, never internal state. `Vault::unlock` is the
  only producer; every CRUD method takes it explicitly. It zeroizes and
  unlocks its pages in `Drop`.
- The in-session metadata index lives on `Session`, never on `Vault`.

---

## Crates

Locked here only once decided, each as a `Q-` entry. P1 settles what P1
uses; Q-YAML-CRATE and Q-PLATFORM-CRATES move to P2 planning. "First used" names the milestone that introduces it.

| Purpose | Choice | Status | First used | Go counterpart |
|---|---|---|---|---|
| CLI parsing/dispatch, help rendering | `clap`, grouped from the registry | decided ([Q-HELP-RENDER](open-questions.md#q-help-render)) | M0a | spf13/cobra |
| TOML | `toml` + `serde` (no custom emitter) | decided ([Q-COMPAT](open-questions.md#q-compat)) | M0b | pelletier/go-toml v2 |
| Typed errors | `thiserror` (library), `anyhow` (CLI only) | likely | M0b | — |
| Advisory file lock | `std::fs::File::lock` (Rust ≥ 1.89) | decided ([Q-FILE-LOCK](open-questions.md#q-file-lock)) | M0b | x/sys flock/LockFileEx |
| Git | `gix` or `git2` (vendored libgit2) | **M0b spike** ([Q-GIT-CRATE](open-questions.md#q-git-crate)) | M0b | go-git |
| Encryption | `age` (rage) | decided for recipient parsing; rest in P2 planning ([Q-AGE-CRATE](open-questions.md#q-age-crate)) | M1 | filippo.io/age |
| Memory locking | `region`, or `libc`/`windows-sys` directly | **decide in P2** ([Q-PLATFORM-CRATES](open-questions.md#q-platform-crates)) | M2 | x/sys mlock/VirtualLock |
| Secret hygiene | `zeroize`, `secrecy` | likely | M2 | manual zeroing |
| Masked terminal input | `rpassword` or `crossterm` | **decide in P2** | M2 | x/term |
| PTY (test-only) | `expectrl` or `portable-pty` | **decide in P2** | M2 | creack/pty |
| YAML | `serde_yaml_ng`, `serde_norway` or `serde_yml` | **decide in P2** ([Q-YAML-CRATE](open-questions.md#q-yaml-crate)) | M3 | gopkg.in/yaml.v3 |
| UUID | `uuid` (v4) | likely | M0b (vault-id validation) | google/uuid |
| Line editing | `rustyline` | likely | M6 | chzyer/readline |
| Clipboard | `arboard` | likely | M12 | atotto/clipboard |
| QR encoding | `qrcode` (encoding only; half-block rendering is gage's own) | likely | M12 | rsc.io/qr |
| CLI test harness | `assert_cmd` (binary smoke tests only) | likely | M0a | `os/exec` in tests |

As in Go: no host-specific code paths or dependencies, no OAuth broker, no
shelling out to `git`. Remote auth is HTTPS + user-supplied token stored at
`$GAGE_STATE/tokens/<host>` (`0600`).

---

## Test conventions

The Go conventions carry over unchanged; see CLAUDE.md for the full list.
The Rust-specific parts:

- **Workflow per milestone:** the test list is the definition of done, walked
  task by task with each task's tests written first.
- **Driving the CLI:** in-process through `gage_cli::run(args, io, prompter)`;
  a pty only for masked passphrase entry and `insert -m`; `assert_cmd` only
  for real-binary smoke tests.
- **No network:** `make test` runs with `CARGO_NET_OFFLINE=true` after a
  `cargo fetch`. Git is exercised against `gittest` bare repos; network
  failure via a fake `RemoteSyncer`.
- **Process-level properties get process-level tests** (`vaultlock`,
  core-dump suppression): spawn the test binary as a child process, never
  threads.
- **Scrypt cost:** a test-only hook lowers the live work factor in test
  builds, with the three guard tests Go has (shipped constant, sole writer,
  shipped value reaches a real age header).
- **Cross-platform CI from M0a:** Linux, macOS and Windows, natively.
- **Coverage:** `cargo-llvm-cov` from M0a, with the gate in CI.

---

## Compatibility and parity strategy

Behavioral parity ([Q-COMPAT](open-questions.md#q-compat)): Go is the spec,
not a peer.

1. **Go reference material** (`tests/fixtures/go/`): the pinned SHA, the
   captured help surface, and the test inventory, regenerated by
   `scripts/go-reference/capture.sh`. The help fixtures are a reference
   for wording and grouping, not a byte gate
   ([Q-HELP-RENDER](open-questions.md#q-help-render)). No Go-written vaults
   are committed: cross-binary reading is not promised.
2. **On-disk schema follows Go**: file names, field names, directory
   layout, what is committed and what stays local, `format_version = 2`.
   Bytes (TOML quoting, YAML style) are gage-rs's own.
3. **Parity ledger** ([parity-ledger.md](parity-ledger.md)): every shipped
   Go item mapped to a milestone or an explicit exclusion. It is the
   feature checklist.
4. **Per-milestone parity check** (CLAUDE.md "Before considering any change
   done"): claimed ledger rows and test-inventory rows all accounted for.
5. **P9 sign-off:** every ledger row closed, every test-inventory row
   ported or excused, user docs complete.

There is no differential harness running both binaries; it was withdrawn
with byte parity.

---

## Cross-milestone contracts

Adapted from the Go plan's table. Milestone IDs are the Rust ones.

| Established in | Contract | Consumed by |
|---|---|---|
| M0b | `Prompter` trait; unlock exchange is typed and method-agnostic (`UnlockRequest`/`UnlockResponse` carrying a `Kind`) | M2 (passphrase), every interactive decision after |
| M0b | `Identity` type + `Vault::unlock(&dyn Prompter) -> Result<Identity, _>` signature, decided before any real crypto exists | M2 implements it; M4/M5 take `&Identity`; M6 caches it |
| M0b | `vaultlock` advisory lock primitive, keyed by vault id | M4 onward: held across every read-modify-commit |
| M0b | Atomic file write (temp + rename) | Global config, `.gage/config.toml`, `known-config.toml` (M10) |
| M0b | Exit-code taxonomy (Go's codes; no reason to differ) | Every command; M9's `verify` 0/1 contract |
| M0b | Command registry — name, aliases, description, group, one-shot/session availability | Every milestone that adds a command registers it; one-shot help (clap) and M6's session help render from it |
| M0b | `gittest::new_bare_remote` | M1's `set-remote` tests; M8a's sync harness |
| M1 | `.gage/config.toml` schema with `format_version = 2` and `[vault].id`; unknown versions rejected with a directional message | Every later reader |
| M1 | Global config schema, incl. per-vault `device`, `method`, `id`, `pubkey` | M2 populates; M4's `updated_by`; `unlock` dispatches on `method`; M12's orphan cleanup reads `pubkey` |
| M1 | `.gitattributes` marking `.age-recipients`/`config.toml` `-merge` | M8a, M10 |
| M1 | Device-name and vault-id normalization **and validation** (path components from untrusted input) | M2, M9, M10, E-phase |
| M2 | `Identity` `Drop` releases the page lock and zeroes key material | M4's one-shot handler; M6's lock and idle timeout |
| M2 | Thin age encrypt-to-recipients / decrypt-with-identity wrapper (bytes in, bytes out) | M3, M9's re-encrypt, M11, E2, R0 |
| M4 | Commit-per-write, under the vault lock | M8a's auto-push; M9's single-commit re-encrypt |
| M5 | Resolver returns a resolved entry *or* a candidate list, as a value | M6 prompts with it; one-shot fails with it |
| M6 | `Session` owns all "how long does this stay unlocked" bookkeeping | M7's index lives there too |
| M7 | Session-scoped metadata index | M8a invalidates after a pull; M8b after each resolution; M11 updates both vaults |
| M9 | Recipient list changes touch `.age-recipients` and `config.toml` together, in one commit, via one `commit_recipient_list` primitive | M10 diffs that pair; E4 and R3 build on the primitive |
| M10 | `RecipientChangeWarning` typed value + cache regeneration rules | M11 (destination vault), E4, R4 |

---

## Deferred / not on the critical path

Inherited from the Go plan unchanged: additional identity methods, additional
vault types, signed recipient changes, a GUI/TUI frontend, identity backup
tooling, release engineering. Plus, for the port:

- **TOTP** — Go has a settled design (`gage-cli-totp.md`) but no
  implementation. Not ported. If Go builds it, it enters the ledger when the
  pin moves past that commit.
- **Retiring Go gage** — a decision for after P9, not part of this plan.
