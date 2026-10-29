# D2b — CLI source sweep

[← D2a](d2a-library-sweep.md) · [D2 plan](index.md) · [#126](https://github.com/distillerylabs/gage-rs/issues/126)

> **Recommended model: Sonnet.** This is a mechanical sweep plus one
> small module move whose output is pinned by tests.

## Goal

`crates/gage-cli` and `scripts/qa` carry no port provenance, and
gage-cli has one duration module in place of two Go-style formatters.
Its output is unchanged. The guard covers all of D2's roots.

## Depends on

D2a, for the guard and the library sweep.

## Design references

- [D2 index](index.md): the anchor → replacement tables and the sweep
  rules.
- [ADR-0037](../../../../../decisions/0037-exit-code-taxonomy.md) and
  `doc/cli/`. The duration grammar and spelling are part of the CLI
  contract (see the [divergence plan](../index.md#resolved-decisions)).
- Archived [open-questions.md](../../../../../archive/rust-port/open-questions.md):
  Q-QR-CRATE, Q-LINE-EDITOR, Q-RECOVERY-DATA-DIR and Q-RECOVERY-EMPTY-OUT,
  the sources for the new ADRs.

## Decisions to make first

1. **Duration module shape.** Proposed: `crates/gage-cli/src/duration.rs`
   with three items:
   - `parse(&str) -> Result<Duration, DurationError>`, moved from
     `shell.rs:201-211`
   - `format(Duration)`, which is today's `shell::go_duration_string`
   - `format_rounded(Duration)`, which rounds to the nearest second
     (half up) and then calls `format`. It replaces `repl::go_duration`.

   On whole seconds the two old formatters already agree (`5s`, `2m5s`,
   `1h0m0s`), so `format_rounded` is exactly `go_duration`. It stays in
   `gage-cli` because the grammar is the frontend's. The library takes
   `Duration`.
2. **The allowlist** after the guard's roots are extended, as in D2a.

## Tasks

### 1. Extend the guard, red

- [ ] Add `crates/gage-cli` and `scripts/qa` to `port_anchors.rs`'s
      roots. Confirm it fails.

### 2. Duration module, tests first

- [ ] Move `go_duration_string_matches_gos_spelling` to
      `duration::tests::format_spells_durations`.
- [ ] Move `durations_follow_gos_grammar` to
      `duration::tests::parse_accepts_the_grammar`.
- [ ] Add `format_rounded_rounds_half_up`. It pins `1.499s` → `1s`,
      `1.5s` → `2s`, `59.5s` → `1m0s` and `3599.5s` → `1h0m0s`, which are
      today's `go_duration` outputs.
- [ ] Implement `duration.rs` to green. Delete both old functions.
- [ ] Update the call sites:
  - `shell.rs:101`
  - `commands/secretout.rs:101`
  - `commands/enroll.rs:23,48-49`
  - `repl.rs:540`

  `commands/enroll.rs::human_duration` is prose and stays.

### 3. New ADRs

- [ ] ADR-0049 The QR crate: encode-only, no decoder
      (Origin: Q-QR-CRATE).
- [ ] ADR-0050 The line editor and the history file
      (Origin: Q-LINE-EDITOR).
- [ ] ADR-0051 What `--recovery-key-out` refuses
      (Origin: Q-RECOVERY-DATA-DIR, Q-RECOVERY-EMPTY-OUT).
- [ ] Link them from `doc/decisions/README.md`. Fix any D2a comment that
      reworded around Q-LINE-EDITOR.

### 4. Sweep `crates/gage-cli`

Work one file at a time, following the [sweep rules](index.md#resolved-decisions).

- [ ] `src/*_tests.rs` and the `tests/` directory: about 600 `// Go:`
      lines, plus "from P2", "M1's half" and "Ported late, in P9 · S1".
- [ ] Production code: `Q-` ids become ADRs, and `D2`, `D3` and `D5` in
      the recovery code are inlined.
- [ ] `Cargo.toml`: Q-LINE-EDITOR, Q-EDITOR, Q-CLIPBOARD, Q-QR-CRATE and
      Q-RECOVERY-LOCAL-IDENTITY-ERROR are replaced.
- [ ] Renames:
  - `scriptprompt::for_script_picks_the_source_in_go_s_order` becomes
    `for_script_picks_the_source_in_order`.
  - `smoke::prompts_go_to_the_error_stream_not_stdout` uses "go" as a
    verb. Keep it unless it reads badly.

### 5. Sweep `scripts/qa`

- [ ] `scenarios/03-share-by-key.sh:67`: Q-RECIPIENT-PROMPT becomes
      ADR-0033.
- [ ] Sweep any other hits the guard reports.

### 6. Green

- [ ] `no_port_anchors` is green, and the allowlist is recorded.
- [ ] Prove it fails by hand in a gage-cli file. Record that here.
- [ ] `git diff --stat main -- tests/fixtures` is empty.
- [ ] `make qa` is green.

## Definition of done

- Every box above is ticked.
- `make lint`, `make test` and `make qa` are clean. The help snapshots
  and `format_v2_*` tests are green and unedited.
- CI is green on Linux, macOS and Windows.
