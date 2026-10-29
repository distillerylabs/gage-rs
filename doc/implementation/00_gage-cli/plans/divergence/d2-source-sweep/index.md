# D2 Source sweep — implementation plan

[← divergence plan](../index.md) · Parent issue
[#124](https://github.com/distillerylabs/gage-rs/issues/124) · Roadmap
[#114](https://github.com/distillerylabs/gage-rs/issues/114)

D2 strips the port's provenance out of the source, so D3 and D4's diffs
carry only the change they are about, and no comment frames a change
against Go.

D2 changes comments and names only. It adds no behavior, changes no CLI
output and writes no different bytes. The help snapshots and the
`format_v2_*` tests prove it, and neither is edited.

---

## Where things stand at the start of D2

Measured on `main` at `8d651d1`.

| Anchor | `crates/gage` | `crates/gage-cli` |
|---|---|---|
| `// Go: TestXxx` lines | 651 (58 files) | 600 (38 files) |
| `.go` file names | 37 | 29 |
| `Q-` ids | 82 | 52 |
| milestone ids (`M10`, `RK4`, `P5`, `S1`, …) | 73 | 77 |
| Go register labels (`A19`, `A20`, `A3`) | most of 39 | rest |
| design labels (`D-ENROLL-*`) | 14 | — |
| Go-named fns | 3 | 5, plus 2 Go-style duration formatters |

Outside the crates:

- root `Cargo.toml:28` (P2)
- `deny.toml:37-44` (four `Q-` ids)
- `rust-toolchain.toml:3` (Q-FILE-LOCK)
- `scripts/qa/scenarios/03-share-by-key.sh:67` (Q-RECIPIENT-PROMPT)

Regenerate the counts with:

```
grep -rEn '// Go:|\bGo\b|\.go\b|Test[A-Z][a-z]+[A-Z]|\bQ-[A-Z]|\b(M|RK|P|S)[0-9]+[a-z]?\b|\bA[0-9]{1,2}\b|\bD-[A-Z]+-' crates Cargo.toml deny.toml rust-toolchain.toml scripts/qa
```

### Anchor → replacement

**`Q-` ids with an ADR.** Use the ADR. This table comes from the
`Origin:` lines in [doc/decisions/](../../../../../decisions/README.md).

| Id | ADR | Id | ADR |
|---|---|---|---|
| Q-AGE-CRATE | 0022 | Q-METHOD-FLAG | 0012 |
| Q-CLIPBOARD | 0035 | Q-METHOD-SCOPE | 0011 |
| Q-CMD-AVAILABILITY | 0032 | Q-ORPHAN-BY-NAME | 0013 |
| Q-COMPAT | 0002 | Q-RECIPIENT-PROMPT | 0033 |
| Q-CONFIRM-DEFAULT-YES | 0033 | Q-RECOVERY-NOCLOBBER | 0028 |
| Q-CONFLICT-PROMPTER | 0021 | Q-RECOVERY-SECRET-MEMORY | 0027 |
| Q-CROCKFORD | 0039 | Q-REMOTE-AUTH | 0020 |
| Q-DEVICE-NAME | 0010 | Q-REMOTE-TIMEOUT | 0018 |
| Q-EDITOR | 0036 | Q-SCRIPT-PASSPHRASE | 0034 |
| Q-ENROLL-SCRYPT-CAP | 0026 | Q-SCRYPT-HOOK | 0024 |
| Q-ENROLL-SCRYPT-HOOK | 0025 | Q-SSH | 0015 |
| Q-FILE-LOCK | 0003 | Q-SYNC-CLASSIFY | 0017 |
| Q-FILE-MERGE | 0019 | Q-SYNC-CONFLICT | 0021 |
| Q-GIT-CRATE | 0014 | Q-SYNC-PUSHED | 0017 |
| Q-GIT-ISOLATION | 0016 | Q-TRUST-HASH | 0004 |
| Q-HELP-RENDER | 0030 | Q-IDENTITY-SECRET | 0023 |
| Q-HELP-SURFACES | 0031 | Q-INDEX-ARENA | 0029 |

**Register labels.** These come from the `Origin:` lines; use the ADR
that fits the context:

- A10 → 0031
- A11, A12 → 0032
- A13 → 0012
- A14 → 0011
- A15 → 0010
- A16 → 0021
- A17 → 0035
- A18 → 0034
- A19 → 0009
- A20 → 0006 (vault-id keying), or 0013 when the code is about orphan
  identities
- A21 → 0007
- A22 → 0008

A3, the sole-recipient invariant, has no ADR. State the invariant inline.

**Design labels.** The enrollment ADRs carry each `D-ENROLL-*` label in
their `Origin:` lines:

| Label | ADR |
|---|---|
| D-ENROLL-PROMPTER | 0038 |
| D-ENROLL-CODE-FORMAT | 0039 |
| D-ENROLL-SEAL-COST | 0040 |
| D-ENROLL-TTL | 0041 |
| D-ENROLL-EXPIRY-IN-NAME | 0042 |
| D-ENROLL-COLLISIONS | 0043 |
| D-ENROLL-REMOTE | 0045 |

Recovery code cites Go's recovery-design labels (`D2`, `D3`, `D5`,
`D12`). Inline what each one says.

**`Q-` ids with no ADR.** D1c deliberately did not carry these forward.
Drop the tag; the comment keeps or gains the reason. New ADRs are written
only where the decision still constrains code. In the table, the ADR
column gives the new ADR's number, or "—" for inline only.

| Id | Refs | ADR | Milestone |
|---|---|---|---|
| Q-TWO-VAULT-LOCK | `movecopy.rs`, `entry/crud.rs` | **new**: two vaults are locked in vault-id order | D2a |
| Q-YAML-CRATE | `entry/yaml.rs`, `entry/layout.rs`, `deny.toml` | **new**: the YAML crate | D2a |
| Q-QR-CRATE | `qr.rs`, `qr_tests.rs`, `Cargo.toml` | **new**: the QR crate, encode-only, no decoder | D2b |
| Q-LINE-EDITOR | `lineread.rs`, `history.rs`, `Cargo.toml`, `deny.toml` | **new**: the line editor and the history file mode | D2b |
| Q-RECOVERY-DATA-DIR, Q-RECOVERY-EMPTY-OUT | `commands/recoverykey.rs`, `init_recoverykey_tests.rs` | **new**: what `--recovery-key-out` refuses | D2b |
| Q-CRASH-SEAM | `vault.rs`, `reencrypt_tests.rs` | — (D4b moves these seams) | D2a |
| Q-APPROVE-SURFACE, Q-ENROLL-AMBIGUOUS, Q-ENROLLMENT-CODE-HISTORY, Q-INIT-INTERIM, Q-INIT-ORDER, Q-OAUTH-APP, Q-ORPHAN-CLEANUP, Q-PENDING-TZ, Q-RECOVERY-ERRORS, Q-RECOVERY-IDENTITY-BACKUP, Q-RECOVERY-LOCAL-IDENTITY-ERROR, Q-RECOVERY-TERMINAL, Q-RECOVERY-TEST-INIT, Q-SESSION-SHAPE | one or two each | — | where cited |

The new ADRs number from 0047 in the order written, and each carries
`Origin: Q-… ([entry](../archive/rust-port/open-questions.md#q-…))`.

## Resolved decisions

See the [divergence plan](../index.md#scope-of-the-later-phases-refined-when-each-is-filed).
Specific to D2:

- **Scope.** D2 covers:
  - `crates/gage` and `crates/gage-cli`
  - the root `Cargo.toml`, `deny.toml` and `rust-toolchain.toml`
  - `scripts/qa`

  It does not cover:
  - `doc/design/`'s milestone and `Q-` references (M5, M8b, M10,
    Q-GIT-AUTH, Q-IDENTITY-VAULT-NAME, …)
  - `doc/format/v2.md:430`'s "Go's `time.ParseDuration`"

  Those go to a later docs pass, recorded in the roadmap index.
- **A permanent guard, written first.** It lives in
  `crates/gage-cli/tests/port_anchors.rs`.
  - It is std-only and reuses `doc_links.rs`'s repo-root and walk
    pattern.
  - It scans `.rs`, `.toml` and `.sh` files under an explicit list of
    roots, and fails with `path:line: text` for each hit.
  - The patterns are the ones in the grep above.
  - It has an allowlist of `(path, substring, reason)` entries, empty
    unless a real false positive survives.
  - D2a enables `crates/gage` and the three root config files. D2b adds
    `crates/gage-cli` and `scripts/qa`. Extending it is one list entry.
- **Sweep rules.**
  - `// Go: TestXxx.` followed by a reason: drop the prefix and keep the
    reason. A bare `// Go: TestXxx` line is deleted.
  - Module headers such as "Ports Go's `foo_test.go`" are rewritten to
    say what the file tests.
  - A milestone id becomes the behavior it names: "M10's trust cache"
    becomes "the trust cache", and "Ported late, in P9 · S1." is deleted.
  - Go framing goes and the reason stays: "refuses it too, in Go and
    here" becomes "refuses it too".
  - `⚑` marks on security tests stay. They flag security-relevant tests
    whatever their origin.
- **Test renames.** A test is renamed only where its name mentions Go or
  reads badly because of the old guard. Names that read fine stay. A
  churned name costs `git log -S` and buys nothing.
- **No code changes** beyond those renames, except D2b's duration merge.

## Milestones

| Milestone | Status | Model | Depends on |
|---|---|---|---|
| [D2a Library sweep](d2a-library-sweep.md) ([#125](https://github.com/distillerylabs/gage-rs/issues/125)) | `[ ]` | Sonnet | parent branch |
| [D2b CLI sweep](d2b-cli-sweep.md) ([#126](https://github.com/distillerylabs/gage-rs/issues/126)) | `[ ]` | Sonnet | D2a |

**Why Sonnet.** The work is mechanical and pattern-following. The one
place meaning can be lost is rewriting a comment that carries a security
reason: a recovery, enrollment or identity comment that only cited a
`Q-` id. For those, the rule is to keep the reason and add the ADR,
never drop the reason.

**Why the library goes first.** The guard lands with D2a's roots, so D2b
only extends a list. The library has no dependency on CLI comments.

## Acceptance

- Every milestone doc's test list is green.
- `port_anchors` covers all of D2's roots and has been proven to fail by
  hand once.
- `git diff --stat main -- tests/fixtures` is empty at the end of each
  milestone.
- `make lint`, `make test` and `make qa` are clean.
- CI is green on Linux, macOS and Windows.

## Process notes

- **Branches stack:** `main` <- `124-divergence-source-sweep` (these
  docs) <- D2a <- D2b. Each milestone branch is cut from the previous one
  when the user says to start it.
- **Tests come first:**
  - The guard is written red.
  - The renamed tests keep their bodies.
  - The duration module's tests move before its code does.
- **Sweep one file per pass**, and read the comment before changing it.
  A blind regex replace loses reasons. `cargo check --workspace
  --all-targets` after each batch catches broken doc links and
  attributes.
- **If context runs low mid-milestone,** write a `*-sessionhandoff.md`
  here, listing which files are done.
