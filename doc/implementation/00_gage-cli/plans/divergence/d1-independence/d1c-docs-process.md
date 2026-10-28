# D1c — Docs archive, re-home, process

[← D1b](d1b-retire-parity.md) · [D1 plan](index.md) · [#118](https://github.com/distillerylabs/gage-rs/issues/118) · next: D2 Source sweep (filed when D1 closes)

> **Recommended model: Sonnet**, with an ⚑ Opus review of the ADRs that
> carry forward security decisions (Q-IDENTITY-SECRET,
> Q-RECOVERY-SECRET-MEMORY, Q-SCRYPT-HOOK, Q-ENROLL-SCRYPT-CAP,
> Q-DEVICE-NAME). Restating a security decision is where meaning gets
> lost.

## Goal

The docs describe gage-rs as its own project, and nothing in the tree
depends on the Go repo continuing to exist:

- The port plans are archived and frozen.
- The TDDs become gage-rs's design docs.
- A decision log replaces the `Q-` register.
- What existed only in Go's repo is imported.
- The process docs stop pointing at Go.

## Depends on

D1b. The deleted guards were the last readers of the
`plans/rust-port/` refs this milestone moves.

## Design references

- The [divergence plan](../index.md#resolved-decisions).
- The archived `open-questions.md`, for which decisions still bind.

## Sources

Go gage at `64b6cf0`, checked out at `../gage` for reading and never
vendored:

- `README.md`: "what gage is", and "Security model and limitations"
- `doc/implementation/00_gage-cli/plans/gage-cli-design/open-questions.md`:
  the Go-only decisions cited from gage-rs code
- the TDDs' A- and D- decision labels

## Decisions to make first

1. **ADR format.**
   - Proposed: `doc/decisions/NNNN-slug.md`, each with Status, Context,
     Decision, Consequences, and "Supersedes / origin" (the archived `Q-`
     anchor or Go register entry).
   - An index in `doc/decisions/README.md`.
   - One ADR per decision. Short; the history stays in the archive.
2. **Which decisions are carried forward.** Proposed: those that
   constrain future code. Specifically:
   - format and storage: Q-COMPAT's surviving invariants, the v2 format
     freeze (→ `doc/format/`), Q-FILE-LOCK, Q-TRUST-HASH,
     Q-TRUST-CACHE-PATH
   - git and sync: Q-GIT-CRATE, Q-SSH, Q-GIT-ISOLATION, Q-SYNC-CLASSIFY,
     Q-REMOTE-TIMEOUT, Q-FILE-MERGE, Q-REMOTE-AUTH
   - crypto and secret memory: Q-AGE-CRATE, Q-IDENTITY-SECRET,
     Q-SCRYPT-HOOK, Q-ENROLL-SCRYPT-HOOK, Q-ENROLL-SCRYPT-CAP,
     Q-RECOVERY-SECRET-MEMORY, Q-RECOVERY-NOCLOBBER, Q-INDEX-ARENA
   - CLI behavior: Q-HELP-RENDER, Q-RECIPIENT-PROMPT, Q-SCRIPT-PASSPHRASE,
     Q-CLIPBOARD, Q-EDITOR, the exit-code taxonomy
   - the Go-only decisions: Q-DEVICE-NAME, Q-METHOD-SCOPE,
     Q-ORPHAN-BY-NAME, Q-CMD-AVAILABILITY, Q-HELP-SURFACES,
     Q-SYNC-CONFLICT, Q-METHOD-FLAG, A19–A22, D-ENROLL-*

   The phase-split and process entries stay archived only (Q-P*-SPLIT,
   Q-E1B-RETAG, Q-INVENTORY-GUARD, Q-README-STATUS, Q-GO-ISSUE-REFS,
   Q-ISSUE-CADENCE).
3. **Code references.**
   - D1c does not rewrite the roughly 130 `Q-` references in source;
     that is D2.
   - Each ADR lists its origin anchors, so D2 can map a `Q-` reference
     to its ADR mechanically.
4. **The TDDs.**
   - Proposed: move them with `git mv` to
     `doc/design/gage-cli-design.md` and `doc/design/gage-cli-init-design.md`.
   - Replace the "gage-rs note" header.
   - Rewrite Go code paths (`internal/gage/*.go`, `cmd/gage`, Cobra,
     go-git) as gage-rs modules or neutral wording, and keep the
     rationale.
   - Turn links to Go-only plan directories into ADR links, or drop them.

## Tasks

### 1. Link guard, red first

- [x] `doc_links_resolve` (`crates/gage-cli/tests/doc_links.rs`, `std::fs`
      only, no new dependency): every relative Markdown link, and every
      `#anchor` within a file, outside `doc/archive/` resolves. Make it
      red once by moving the register, and record that here.
      *Done.* Written first, it already failed on 9 real links (the
      Go-only plan links in the enrollment TDD, two `GO_GAGE_SHA` links, and
      a `session-mode.md#yes-in-a-script` anchor whose heading slug is
      `--yes-in-a-script`). Moving `rust-port/` to `doc/archive/` took it to
      27, which is the red-once. It is green at the end.

### 2. Archive

- [x] `git mv doc/implementation/00_gage-cli/plans/rust-port doc/archive/rust-port`.
- [x] Add `doc/archive/README.md`: frozen, kept as history, not
      maintained, with links that may point at a Go repo that has been
      archived.
- [x] Fix the divergence plan's links into `../rust-port/`.

### 3. Decision log

- [x] `doc/decisions/README.md`: the format, the index, and "every new
      decision is an ADR".
- [x] `0001-divergence-from-go-gage.md`: the resolved decisions in the
      divergence plan. This is the one document outside the archive
      that names Go gage.
- [x] One ADR per decision-2 entry, each with its origin anchors.
- [ ] Opus review of the security ADRs listed in the model note. *Not done:
      the ADRs were written in a Sonnet session. Review ADR-0023, 0024,
      0025, 0026, 0027 and 0010 before merging.*
- [x] `doc/design/lint-rationale.md`: the gosec/golangci → clippy and
      cargo-deny mapping table, moved out of
      `p9-parity-signoff/s2-ledger-and-help.md`.

### 4. Design docs and imported content

- [x] Move the TDDs to `doc/design/` per decision 4.
- [x] `doc/design/security-model.md`, from the Go README's "Security
      model and limitations", rewritten for gage-rs (for example, SSH
      per Q-SSH).
- [x] README: say what gage is (from the Go README) and drop "use Go
      gage for real secrets" and "a Rust port of". Keep "not released,
      not independently audited". Link `doc/cli/`, `doc/design/`,
      `doc/format/` and `doc/decisions/`.
- [x] `SECURITY.md`: point at `doc/design/security-model.md` and drop
      the port wording.
- [x] `CODE_OF_CONDUCT.md:64`: report through this repo.
- [x] `doc/cli/README.md`, `command-reference.md` and `exit-codes.md`:
      drop the Go framing and point the `Q-` links at ADRs.

### 5. Process docs

- [x] CLAUDE.md:
  - Remove: "Compatibility and parity", the ledger/inventory phase
    planning, "Port the current Go behavior", the "Go sources" item in
    milestone docs, the parity item in "Before considering any change
    done", and "mirror Go's `internal/gage/*`".
  - Add: "Format stability" (D1a's rule) and "Decisions" (an ADR in
    `doc/decisions/`).
  - Point "Full design rationale" at `doc/design/`.
- [x] CONTRIBUTING.md: remove "parity gaps" and "Go is the reference".
- [x] `.claude/agents/gage-qa.md`: the spec order becomes
      `doc/cli/` → `doc/format/` → ADRs. Coverage is computed from the
      help snapshots.

### 6. Check

- [ ] `grep -rn 'Go gage\|GO_GAGE_SHA\|parity'` outside `doc/archive/`
      hits only ADR-0001 and the "Retired" history line in
      `doc/decisions/README.md`, if one is kept.

      *Status.* Hits remain in places this milestone does not own: four
      source-comment mentions in `crates/gage-cli/src/` (D2's sweep), the
      divergence and D1 plan documents (they describe this change), and
      one unrelated use of "parity" in `doc/design/gage-cli-design.md`
      ("parity with POSIX").

## Definition of done

- Every box above is ticked.
- `doc_links_resolve` is green.
- `make lint`, `make test` and `make qa` are clean.
- CI is green on all three OSes.
- The D1b help snapshots and the D1a fixtures are unchanged.
- The D1 index and the divergence plan's status columns are `[x]`.
