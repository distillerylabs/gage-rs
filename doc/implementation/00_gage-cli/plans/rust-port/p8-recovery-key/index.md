# P8 Recovery key — implementation plan

[← port plan](../index.md) · Parent issue
[#74](https://github.com/distillerylabs/gage-rs/issues/74) · Roadmap
[#2](https://github.com/distillerylabs/gage-rs/issues/2) · previous:
[P7 Enrollment](../p7-enrollment/index.md)

At the start of P8, a vault whose only recipient is one device's
passphrase-wrapped identity file has no recovery story. If that file or its
passphrase is lost, the vault is unreadable forever. `init` makes no
recovery key ([Q-RECOVERY-INTERIM](../open-questions.md#q-recovery-interim)).

By the end of P8:

- **`gage init`** generates a second X25519 keypair by default and
  registers its public key as the `recovery-paper-key` recipient.
  - It shows the private key once, as text and a QR on stderr, after the
    vault is committed. Then it asks for the key's last six characters back.
  - `--recovery-key-out FILE` writes the key to a new 0600 file instead.
    `--no-recovery-key` opts out.
  - With neither flag and no terminal on both stdin and stderr, `init`
    fails before creating anything.
- **`gage recovery verify`** checks a pasted key against the recipient
  list, with no unlock. It reports "decrypts this vault" and "can run
  `recovery enroll`" as two separate claims.
- **`gage recovery enroll`** is the key's one power inside gage. In a single
  commit it enrolls a fresh device identity, retires the key just used and
  mints a replacement. `--replaces DEVICE` also removes the lost device in
  that commit.
- **`gage recovery rotate`** replaces the recovery key from an unlocked
  device, or adds the first one, in one commit.
- The label `recovery-paper-key` is **reserved**. No device can take it
  through `init`, `identity add`, `recipient add`, `identity enroll`,
  approval or `recovery enroll`.

The spec is the copied TDD's "Recovery key at `init`"
([gage-cli-design.md](../../../tdds/gage-cli-design.md)). It was copied
after Go's R5 and is current. Go's decisions D1–D12 and every detail its
milestone docs settled are carried over and cited, not re-argued. They are
in Go's `recovery-key/` plan at the
[pinned SHA](https://github.com/distillerylabs/gage/tree/64b6cf078f3e756efb9d868cc1518dc1d9060748/doc/implementation/00_gage-cli/plans/recovery-key).

This plan does not restate the shared ground. The layering rules, test
conventions, crate choices and cross-milestone contracts are in the
[port plan](../index.md). Decisions are recorded in
[open-questions.md](../open-questions.md).

---

## Why this phase looks the way it does

Go built the recovery key as six milestones in two phases:

- R0–R2: the key at `init`, and `recovery verify`
- R3–R5: the recovery identity, `recovery enroll`, `recovery rotate`

Its later milestones changed the earlier ones:
- R4 made `verify` report the matched label.
- R5 reserved the label everywhere and flipped an R1 test.
- R4 and R5 rewrote R1's failure messages twice.

gage-rs ports the end state, as four milestones **RK1–RK4**
([Q-P8-SPLIT](../open-questions.md#q-p8-split)). They regroup Go's
material, so they don't reuse Go's R-numbers: a Go ID here would mean two
different things.

| Rust | Go material |
|---|---|
| RK1 | R0, R4's label-returning `VerifyRecoveryKey`, R5's label reservation |
| RK2 | R3, R5's library half (`RotateRecoveryKey`, `remove_if_present`, `inspect`) |
| RK3 | R1, R5's CLI (`recovery rotate`) |
| RK4 | R4, R2 (`recovery verify`), R5's docs |

**The order is not Go's, on purpose.** Go's R1 and R4 shipped stopgap
messages, because `recovery rotate` did not exist yet: "use `age-keygen`
then `recipient add`". In the current state that stopgap is impossible.
The reservation makes `recipient add --device recovery-paper-key` fail.
Porting the current state therefore forces two rules:

- **Every command a message names exists in the same milestone.**
  - `init`'s delivery-failure messages and its refused-confirmation message
    name `gage recovery rotate`, so rotate ships with `init` (RK3).
  - `verify`'s refusal and the `clone`/`use` hints name
    `gage recovery enroll`, so they ship with enroll (RK4).
- **The reservation exists from the first milestone that writes the
  label** (RK1). It is what makes rotate's eviction safe. Rotate cannot tell
  a recovery key from any other X25519 key: the label *is* the
  distinction.

Both swaps are library work (RK2), so they come before both CLI milestones.
The library goes first and is easy to test. The security-sensitive CLI
ordering follows in two pieces, split by which delivery path they add.

Planning found these points about this tree that Go's documents don't make:

- **There is no stderr-terminal flag.**
  - `Io` has `stdin_is_terminal` only.
  - Go's R1 widened D2 to "both stdin and stderr", because
    `gage init foo 2>build.log` is the run that would file a private key
    into a log.
  - `Io` gains `stderr_is_terminal`, set in `main.rs`, with no fallback
    from one flag to the other
    ([Q-RECOVERY-TERMINAL](../open-questions.md#q-recovery-terminal)).
- **About 190 in-process `init` calls would start refusing.**
  - Every in-process test runs with `stdin_is_terminal: false`. Go's R1
    hit the same thing at 57 call sites.
  - Here the shared helpers (`Env::init`, `sync_tests::init_with_remote`)
    pass `--no-recovery-key` unless a test opts in. The roughly 70 literal
    `["init", …]` calls state the flag
    ([Q-RECOVERY-TEST-INIT](../open-questions.md#q-recovery-test-init)).
- **`atomicfile::write` always overwrites.**
  - Go checks with stat, then writes, which leaves a window where another
    file can appear.
  - gage-rs adds a no-clobber write (`persist_noclobber`) and keeps Go's
    up-front checks, so refusals still come first
    ([Q-RECOVERY-NOCLOBBER](../open-questions.md#q-recovery-noclobber)).
- **Go's "a Go string can't be zeroed" gap mostly closes.**
  - `Prompter::value` and `render_qr` already return `Zeroizing<String>`.
  - The library parses a pasted key through `KeyLine::parse` into a
    `LockedBuf`, exactly as an identity file is parsed.
  - What remains is that the display copy is not page-locked
    ([Q-RECOVERY-SECRET-MEMORY](../open-questions.md#q-recovery-secret-memory)).
- **The recovery identity is crate-private by construction.**
  - `KeyLine::parse` and `Identity::new` are already `pub(crate)`, and
    `Identity` has private fields.
  - `recovery_identity` is a private function in `crates/gage`, so no
    caller outside the crate can obtain one. This matches Go's "structural,
    not a convention".
- **`commit_recipient_list` is private** to `recipient.rs`. The swap needs
  it.
  - E4 built approval on `add_recipient_locked`, which only adds.
  - The swap both adds and removes, so it calls `commit_recipient_list`
    directly, as Go's `swapRecipients` does. That function widens to
    `pub(crate)`; nothing else changes.
- **The pull before an unlock-less write already exists.**
  - Go's `RecoverDevice` calls `syncOnUnlock` first, because that path
    skips `Unlock`.
  - gage-rs has `sync_on_unlock` (`remotesync.rs`, `pub(crate)`).
- **`identityfile::create` reuses an existing file and asks for its
  passphrase.**
  - For a forgotten passphrase that is exactly wrong.
  - Enroll's refuse-or-replace policy, and its in-memory backup, live in
    `gage-cli`, as Go's do.
  - That needs read access to the file's bytes and path. The path comes
    from `paths::identity_file_path`. RK4 decides whether the backup reads
    through a small library helper, or through `std::fs` on that validated
    path.
- **`$GAGE_DATA` is not an environment variable here.**
  - It is the XDG data dir (`Dirs.data`), which `crates/reset-local-state`
    wipes.
  - The "places gage deletes from" guard uses `Dirs`
    ([Q-RECOVERY-DATA-DIR](../open-questions.md#q-recovery-data-dir)).
- **The root help's grouping differs, as allowed.**
  - Go lists `recovery enroll/rotate/verify` in the Recipients group,
    between `recipient verify` and `recipient pending`.
  - gage-rs orders that group as E4 left it. The three commands join the
    Recipients group, one-shot only, as in Go's registry. Order within a
    group is layout, not behavior
    ([Q-HELP-RENDER](../open-questions.md#q-help-render)).

## Resolved decisions

| ID | Decision |
|---|---|
| [Q-P8-SPLIT](../open-questions.md#q-p8-split) | RK1–RK4 as above: library, library, `init` + rotate, enroll + verify + docs. Every message names only commands in its own milestone, and the label reservation lands in RK1. |
| [Q-RECOVERY-TERMINAL](../open-questions.md#q-recovery-terminal) | `Io.stderr_is_terminal`, set in `main.rs`. Key display needs both flags, and neither falls back to the other. |
| [Q-RECOVERY-TEST-INIT](../open-questions.md#q-recovery-test-init) | The shared helpers pass `--no-recovery-key` by default, literal `init` calls state the flag, and `init_vault_with_recovery_key_file` mirrors Go's helper. |
| [Q-RECOVERY-NOCLOBBER](../open-questions.md#q-recovery-noclobber) | `atomicfile::write_new`: temp file plus `persist_noclobber`, 0600. It refuses an existing destination atomically. Go's up-front checks stay. |
| [Q-RECOVERY-SECRET-MEMORY](../open-questions.md#q-recovery-secret-memory) | `Zeroizing<String>` in the CLI, and a `LockedBuf` once parsed in the library. `recover_device` takes the secret by value. The display copy is not page-locked: a recorded gap. |
| [Q-RECOVERY-DATA-DIR](../open-questions.md#q-recovery-data-dir) | The guard refuses the vault being created, any ancestor holding `.gage/config.toml`, and anything under `Dirs.data`. Paths are absolutized and symlinks resolved as far as they exist. |
| [Q-RECOVERY-ERRORS](../open-questions.md#q-recovery-errors) | RK2: the retired key offered back, and rotate evicting the acting device, are `RetiredKeyReused` and `WouldEvictActingDevice` (`Conflict`), not `RecipientExists`. |

## Milestones

Legend: `[ ]` not started, `[~]` in progress, `[x]` done.

| # | Milestone | Model | Depends on | Inventory rows | Status |
|---|---|---|---|---|---|
| RK1 | [The recovery key, library](rk1-recovery-key-library.md) ([#75](https://github.com/distillerylabs/gage-rs/issues/75)) | Sonnet\* ⚑ | P7 | 11 | `[x]` |
| RK2 | [The recovery identity and the swap, library](rk2-recovery-swap-library.md) ([#76](https://github.com/distillerylabs/gage-rs/issues/76)) | **Opus** ⚑ | RK1 | 25 | `[~]` |
| RK3 | [`init` gains the key; `recovery rotate`](rk3-init-and-rotate.md) ([#77](https://github.com/distillerylabs/gage-rs/issues/77)) | **Opus** ⚑ | RK2 | 30 | `[ ]` |
| RK4 | [`recovery enroll`, `recovery verify`, hints, docs](rk4-enroll-verify-and-docs.md) ([#78](https://github.com/distillerylabs/gage-rs/issues/78)) | **Opus** ⚑ | RK3 | 26 | `[ ]` |

`⚑` means an Opus review pass over that milestone's named tests before it
is called done. Each is in that milestone's last task:

- **RK1:** the scoping of the duplicate check, and the reservation sites.
  - The duplicate check must leave a repeated `--recipient` legal, so
    `create_repeated_recipients_all_written` stays green.
  - The reservation must be in every path that names a device, including
    approval's `--device` relabel, and in none that writes the recovery
    recipient.
- **RK2:** the swap's atomicity, and four more properties:
  - a crash at any re-encrypted entry leaves HEAD untouched
  - the retired key cannot come back as the device or as its own
    replacement
  - no error, on any path, contains the pasted secret
  - M10's question still runs, and a decline writes nothing

  A swap that commits half a list is invisible until a crash.
- **RK3:** the secret-hygiene walk, plus four properties:
  - the hygiene walk covers the XDG roots, the vault tree and every git
    object, decompressed
  - the key is shown only after the commit exists
  - the both-terminals rule
  - the destination guard runs before anything is created, and the held
    delivery error still lets `--remote` publish

  Go's R1 review found three data-loss or leak bugs in exactly these.
- **RK4:** enroll's step ordering, and three more properties:
  - every refusal comes before the paste, and the paste is checked before
    the new passphrase
  - the backup restore works when the displaced file was the vault's only
    identity (Go's review bug: the directory had been pruned)
  - the new key is delivered before `record_local_identity` can fail

`*` means borderline: start on Sonnet, and switch at the milestone boundary
if it turns awkward. RK1 is additive and pattern-following, which is Go's
R0 recommendation. The reservation sites are the part to watch.

## Inventory accounting

P8 owns 92 rows of `tests/fixtures/go/test-inventory.tsv`. Planning re-tags
none of them out. Each Go file goes to the milestone that owns the function
its tests call:

| Milestone | Rows |
|---|---|
| RK1 | `internal/gage/recovery_key_test.go` (10); `internal/gage/recovery_enroll_test.go` `TestTheRecoveryLabelIsReservedForTheRecoveryKey` (1) |
| RK2 | the other 25 rows of `internal/gage/recovery_enroll_test.go`: 17 for `recover_device` and the identity, 8 for `rotate_recovery_key` |
| RK3 | `cmd/gage/init_recoverykey_test.go` (21); `cmd/gage/recovery_enroll_test.go`'s 8 `TestRecoveryRotate*` rows and `TestNoCommandLetsADeviceTakeTheRecoveryLabel` (1) |
| RK4 | `cmd/gage/recovery_enroll_test.go`'s other 19 rows (16 `TestRecoveryEnroll*`, `TestCloneAndSessionPointAtRecoveryEnroll`, `TestNoRecoveryKeyWarningNamesARealCommand`, `TestRecoveryVerifyDistinguishesDecryptingFromEnrolling`); `cmd/gage/recovery_test.go` (7) |

`recovery_enroll_test.go` (CLI) is split because Go put rotate's CLI rows
in the same file. `TestNoCommandLetsADeviceTakeTheRecoveryLabel` is
claimed by RK3, which ports its `init`, `identity add` and `recipient add`
subtests. RK4 adds its `recovery enroll` subtest to the same Rust test.

Obligations that earlier phases left to P8, which aren't inventory rows:

| From | Obligation | Claimed by |
|---|---|---|
| P1 · M1 / [Q-RECOVERY-INTERIM](../open-questions.md#q-recovery-interim) | `init`'s `--no-recovery-key` and `--recovery-key-out` (`init.txt`); `init.rs`'s "the recovery key is P8's" module note; `lifecycle_tests::init_help_describes_this_milestone`'s "no recovery" assertion flips | RK3 |
| P1 · M0b (ledger §1 #23) | The `recovery` parent lists its children in its own help (`recovery.txt`) | RK3 (`recovery` parent with `rotate`), RK4 (`enroll`, `verify`) |
| P4 · M8a2, P7 · E3 | `clone`'s access message names `gage recovery enroll` too: a sole owner holding a paper key can't get an enrollment request approved by anybody | RK4 |
| P3 · M6, P7 · E3 | The no-identity hint (`vault.rs`'s missing-identity message, the one `use` surfaces) names `gage recovery enroll` too | RK4 |
| P5 · M9a | `identity add`'s long help calls it "the recovery path for a lost identity file". Reword it to point at `recovery enroll` | RK4 |
| P5 · M11 (ledger §6) | The recovery sections of `doc/cli/identities-and-recipients.md`, replacing its "Not yet in gage-rs" stub, and `vaults.md`'s `init` text | RK4 |

Not claimed:

- `getting-started.md` and `command-reference.md` stay P9's (ledger §6).
  Go's versions mention the recovery key, and P9's final pass picks that up.
- The copied TDD needs no edit. It already describes R5's end state.

## Process notes

- Branches stack: `main` <- `74-p8-recovery-key` (these docs) <- RK1 <- RK2
  <- RK3 <- RK4. Each milestone branch is cut from the previous one when the
  user says to start it.
- Tests come first, at task granularity. Each task's tests are written and
  implemented to green before the next task starts.
- **Quiet scope narrowing** is the failure mode to watch: most of a list
  done and the milestone reported done. Check the boxes and the claimed
  inventory rows, not a summary. A test that turns out not to be worth
  writing is struck from the list *in the doc*, with a one-line reason.
- **Go's milestone docs are the test lists' source.** Their "Found by
  review" sections are the most important part, and each Rust doc carries
  those findings as tests. When a Rust row's port seems redundant, re-read
  the Go bullet before striking it.
- **No network.** Remote behavior runs against `gittest` bare repos. An
  unreachable remote comes from the injected `RemoteSyncer`, never a real
  timeout. Interrupted swaps use the existing `with_reencrypt_hook` crash
  seam ([Q-CRASH-SEAM](../open-questions.md#q-crash-seam)), not a new one.
- **Secret-hygiene tests search everywhere a key could land.** That means
  stdout, stderr, every file under the three XDG roots and the vault tree,
  and every git object read through the object database, so compression
  can't hide a match. A substring search of the `.git` directory on disk
  proves nothing, because git compresses its objects.
- **Every new command or flag** gets a registry entry, its
  `commands/mod.rs` edits, and a row in a
  `p8_*_have_their_go_fixture_flags_and_descriptions` test, following
  `approve_cli_tests.rs`.
- **Exit codes are part of every milestone's definition of done.** A
  milestone that adds an error adds its code and a test pinning it. P8
  adds no new code. Every error maps to an existing one, as in Go.
- Each milestone ends green on `make lint` and `make test`, locally and on
  all three CI platforms.
- If context runs low mid-milestone, write a `*-sessionhandoff.md` here.
