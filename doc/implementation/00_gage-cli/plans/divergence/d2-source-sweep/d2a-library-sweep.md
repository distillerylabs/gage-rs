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

## Tasks

### 1. The guard, red

- [ ] `crates/gage-cli/tests/port_anchors.rs`. `no_port_anchors` scans
      `.rs`, `.toml` and `.sh` files under its roots for the patterns in
      the [index](index.md#where-things-stand-at-the-start-of-d2):
  - `// Go:` and the word `Go`
  - `.go` names
  - `Test[A-Z]…` names
  - `Q-` ids
  - milestone ids
  - `A<n>` and `D-ENROLL-*` labels

  It is std-only, and it reports every hit as `path:line: text`.
- [ ] Roots: `crates/gage`, `Cargo.toml`, `deny.toml`,
      `rust-toolchain.toml`.
- [ ] `allowlisted_entries_still_match`: an allowlist entry whose
      substring no longer occurs fails, so the list can't go stale.
- [ ] Run it and confirm it fails on the current tree.

### 2. New ADRs

- [ ] ADR-0047 Two vaults are locked in vault-id order
      (Origin: Q-TWO-VAULT-LOCK).
- [ ] ADR-0048 The YAML crate (Origin: Q-YAML-CRATE).
- [ ] Link both from `doc/decisions/README.md`.

### 3. Sweep `crates/gage`

Work one file at a time, following the [sweep rules](index.md#resolved-decisions).

- [ ] `src/*_tests.rs` and the `tests/` directory: about 650 `// Go:`
      lines and the "Ports Go's `foo_test.go`" headers.
- [ ] `src/**/*.rs` production code: `Q-` ids become ADRs using the
      table, and milestone ids and A/D labels are rewritten.
- [ ] `Cargo.toml`: Q-GIT-CRATE, Q-SSH and Q-TRUST-HASH become ADRs.
- [ ] Renames:
  - `devicename::normalize_lowercases_like_go` becomes
    `normalize_lowercases`.
  - `vault::inferred_names_still_go_through_check_vault_name` and
    `gitrepo::fetch_and_push_go_anonymous_with_no_stored_token` use "go"
    as a verb. Keep them unless they read badly.
- [ ] `cargo check --workspace --all-targets` after each batch.

### 4. Sweep the root config files

- [ ] `Cargo.toml:28`: state the measured reason instead of "P2
      planning".
- [ ] `deny.toml`: Q-GIT-CRATE, Q-AGE-CRATE, Q-YAML-CRATE and
      Q-LINE-EDITOR become ADRs. Q-LINE-EDITOR's ADR comes in D2b, so
      either cite it by its planned number or reword the comment so it
      cites nothing.
- [ ] `rust-toolchain.toml`: Q-FILE-LOCK becomes ADR-0003.

### 5. Green

- [ ] `no_port_anchors` is green, and the allowlist is recorded under
      decision 1.
- [ ] Prove it fails by hand: add `// Go: TestX` to a library file, watch
      it go red, revert. Record that here.
- [ ] `git diff --stat main -- tests/fixtures` is empty.

## Definition of done

- Every box above is ticked.
- `make lint` and `make test` are clean. The help snapshots and
  `format_v2_*` tests are green and unedited.
- CI is green on Linux, macOS and Windows.
