# D2a — Library source sweep

[D2 plan](index.md) · [#125](https://github.com/distillerylabs/gage-rs/issues/125) · next: [D2b](d2b-cli-sweep.md)

> **Recommended model: Sonnet.** This is a mechanical, comment-only sweep,
> plus one std-only scan test that follows `doc_links.rs`.

## Goal

`crates/gage` and the root config files carry no port provenance. Each
comment that explained *why* still does, now citing an ADR where the
decision binds. A guard test keeps it that way.

## Depends on

The parent branch `124-divergence-source-sweep` (these docs).

## Design references

- [D2 index](index.md): the anchor → replacement tables and the sweep
  rules.
- [doc/decisions/](../../../../../decisions/README.md): ADR format and
  `Origin:` lines.
- `crates/gage-cli/tests/doc_links.rs`: the repo-root and walk pattern
  the guard reuses.
- Archived [open-questions.md](../../../../../archive/rust-port/open-questions.md):
  `Q-TWO-VAULT-LOCK` and `Q-YAML-CRATE`, the sources for the two new
  ADRs.

## Decisions to make first

1. **The guard's allowlist.** It is proposed empty. It is settled once
   the guard first runs against the swept tree. Any surviving hit is
   either rewritten or listed with a reason. Record the final list here.

   **Settled: empty.** Every hit on the swept tree was a real anchor and
   was rewritten. The guard also flags `E<n>` ids (enrollment task ids such
   as `E4`), which the index's grep does not list; they were the same kind
   of anchor.

## Tasks

### 1. The guard, red

- [x] `crates/gage-cli/tests/port_anchors.rs`. `no_port_anchors` scans
      `.rs`, `.toml` and `.sh` files under its roots for the patterns in
      the [index](index.md#where-things-stand-at-the-start-of-d2):
  - `// Go:` and the word `Go`
  - `.go` names
  - `Test[A-Z]…` names
  - `Q-` ids
  - milestone ids
  - `A<n>` and `D-ENROLL-*` labels

  It is std-only, and it reports every hit as `path:line: text`.
- [x] Roots: `crates/gage`, `Cargo.toml`, `deny.toml`,
      `rust-toolchain.toml`.
- [x] `allowlisted_entries_still_match`: an allowlist entry whose
      substring no longer occurs fails, so the list can't go stale.
- [x] Run it and confirm it fails on the current tree.

### 2. New ADRs

- [x] ADR-0047 Two vaults are locked in vault-id order
      (Origin: Q-TWO-VAULT-LOCK).
- [x] ADR-0048 The YAML crate (Origin: Q-YAML-CRATE).
- [x] Link both from `doc/decisions/README.md`.

### 3. Sweep `crates/gage`

Work one file at a time, following the [sweep rules](index.md#resolved-decisions).

- [x] `src/*_tests.rs` and the `tests/` directory: about 650 `// Go:`
      lines and the "Ports Go's `foo_test.go`" headers.
- [x] `src/**/*.rs` production code: `Q-` ids become ADRs using the
      table, and milestone ids and A/D labels are rewritten.
- [x] `Cargo.toml`: Q-GIT-CRATE, Q-SSH and Q-TRUST-HASH become ADRs.
- [x] Renames:
  - `devicename::normalize_lowercases_like_go` becomes
    `normalize_lowercases`. Done.
  - `vault::inferred_names_still_go_through_check_vault_name` and
    `gitrepo::fetch_and_push_go_anonymous_with_no_stored_token` use "go"
    as a verb. Keep them unless they read badly. Kept: they read fine.
- [x] `cargo check --workspace --all-targets` after each batch.

### 4. Sweep the root config files

- [x] `Cargo.toml:28`: state the measured reason instead of "P2
      planning".
- [x] `deny.toml`: Q-GIT-CRATE, Q-AGE-CRATE, Q-YAML-CRATE and
      Q-LINE-EDITOR become ADRs. Q-LINE-EDITOR's ADR comes in D2b, so
      either cite it by its planned number or reword the comment so it
      cites nothing.
- [x] `rust-toolchain.toml`: Q-FILE-LOCK becomes ADR-0003.

### 5. Green

- [x] `no_port_anchors` is green, and the allowlist is recorded under
      decision 1.
- [x] Prove it fails by hand: add `// Go: TestX` to a library file, watch
      it go red, revert. Record that here.

      Done: appended `// Go: TestX` to `xdgpaths.rs`; `no_port_anchors`
      failed with `xdgpaths.rs:241: Go comment`, and passed again once
      reverted.
- [x] `git diff --stat main -- tests/fixtures` is empty.

## Definition of done

- Every box above is ticked.
- `make lint` and `make test` are clean. The help snapshots and
  `format_v2_*` tests are green and unedited.
- CI is green on Linux, macOS and Windows.
