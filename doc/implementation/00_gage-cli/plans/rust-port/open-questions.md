# Rust port — open questions

[← port plan](index.md)

The decision register for questions the **port** raises. Every design
decision inherited from Go gage stays recorded in the
[Go register](https://github.com/distillerylabs/gage/blob/main/doc/implementation/00_gage-cli/plans/gage-cli-design/open-questions.md)
and is not restated here; an entry belongs here only if it is Rust-specific
or records a deliberate deviation from the Go binary.

Status: `[ ]` open, `[x]` resolved. A resolved entry is kept as a record.

---

## Resolved at bootstrap

### `[x]` Q-COMPAT — How compatible is gage-rs with Go gage? {#q-compat}

**Resolved: fully on-disk compatible.** A vault written by either binary is
readable and writable by the other. The Go binary at
`tests/fixtures/go/GO_GAGE_SHA` is the reference; any observable difference
(on-disk bytes, exit codes, help text, error wording that tests pin) is a
bug in gage-rs unless an entry here records it as a deliberate deviation.
This constrains crate choices (YAML output, age header, scrypt work factor)
and is why the differential harness exists.

### `[x]` Q-PORT-STATE — Port Go's milestones as built, or Go's current behavior? {#q-port-state}

**Resolved: current behavior.** A19/E1a and A20/E0, and the post-plan
changes, are folded into the Rust milestone that owns the surface. No Rust
milestone builds behavior Go later replaced (v1 config, name-keyed
identities, `recipient add --reencrypt`). See the
[mapping](index.md#go--rust-milestone-mapping).

### `[x]` Q-ISSUE-CADENCE — When are GitHub issues filed? {#q-issue-cadence}

**Resolved: roadmap plus rolling phases.** One roadmap tracking issue lists
P1–P9. A phase's parent issue and milestone sub-issues are filed when the
phase starts, so later issues reflect what earlier phases learned.

### `[x]` Q-TOTP — Is TOTP in scope? {#q-totp}

**Resolved: no.** Go has a settled design (`gage-cli-totp.md`) and no
implementation, so there is nothing to be compatible with. The TDD is not
copied. If Go builds it, it enters the ledger when the pin moves.

### `[x]` Q-RECOVERY-INTERIM — What does `init` do before the recovery key lands? {#q-recovery-interim}

**Resolved: behave like Go `init` before Go #7.** The recovery key (P8)
depends on M9's `commit_recipient_list` and the enrollment phase's
pending-prune, so it cannot land with M1/M2. Until P8, Rust `gage init`
creates no recovery key and accepts neither `--no-recovery-key` nor
`--recovery-key-out`. This is a known, temporary help-surface and
behavior difference from the pinned Go binary: the `init` help fixture and
any differential scenario involving `init` are marked expected-to-differ
until P8, and P8's definition of done removes those marks. Acceptable only
because there is no installed base.

### `[x]` Q-GO-ISSUE-REFS — Which issue numbers do the Go plan docs cite? {#q-go-issue-refs}

**Resolved: an earlier repository's.** The Go plan docs and commit messages
cite #25 (M13), #27, #38, #39, #54, #55, #58 and #59–#63 for features, but in
`distillerylabs/gage` those numbers are unrelated (Codecov, coverage,
dependabot PRs, …) or don't exist.
They come from the repository Go gage lived in before it moved. The parity
ledger therefore identifies pre-move work by **commit SHA** in the Go repo,
with the old issue number only as a label, and treats Go's git history as a
source in its own right rather than trusting the GitHub issue list to be
complete.

---

## Open — decide in P1, before M1

Each is decided by a short spike, recorded here with the evidence, and
reflected in the [crates table](index.md#crates). **Model: Opus.**

### `[ ]` Q-GIT-CRATE — `gix` or `git2`? {#q-git-crate}

**Blocks:** M1 (`init`, `set-remote`), and the whole of P4.

- `gix`: pure Rust, closest in spirit to go-git (no C, no system git).
  Risk: push and merge support has historically trailed fetch/clone.
- `git2`: libgit2, statically vendored so the binary stays self-contained
  and never shells out. Mature push/merge; a C dependency and its CVE stream.

The spike must show, against a `gittest`-style local bare repo and offline:
init + commit, fetch, fast-forward, **push over HTTPS with a token** (to a
local smart-HTTP fixture), divergence detection, a merge commit with chosen
per-file resolutions (M8b), history walk of one path (`log`/`history`), and
that the local transport works on Windows CI. Go's M8a/M8b docs list what
go-git's thin merge surface forced; check the candidate against that list.

### `[ ]` Q-YAML-CRATE — Which YAML crate reproduces Go's entry bytes? {#q-yaml-crate}

**Blocks:** M3.

`serde_yaml` is deprecated. Candidates: `serde_yaml_ng`, `serde_norway`,
`serde_yml`. The entry payload is YAML inside age, and `cat`/`edit` show it,
so the choice is pinned with golden-byte tests against Go `yaml.v3` output:
block scalars for multi-line values, key order, quoting of strings that look
like numbers/bools/dates, timestamp format, empty `fields`. Go's `cat`
post-processes through the YAML node tree (#60/#61), so node-level access
matters too. A hand-written emitter for the fixed entry shape is an
acceptable answer if no crate matches.

### `[ ]` Q-AGE-CRATE — Does the `age` crate cover everything Go gage uses? {#q-age-crate}

**Blocks:** M2.

Check: X25519 recipients/identities, scrypt (passphrase) recipients with a
settable work factor (shipped 2^19, plus the test-only hook and its guard
tests), armored and binary formats as Go writes them, `.age-recipients`
parsing identical to Go's (comments, blank lines, labels), and decryption of
files Go wrote. Also E2's separate enrollment scrypt factor.

### `[ ]` Q-FILE-LOCK — Which advisory-lock primitive? {#q-file-lock}

**Blocks:** M0b.

`std::fs::File::lock`/`try_lock` (stable since Rust 1.89) vs `fs4`. Must
match Go's `flock`/`LockFileEx` semantics so a Go process and a Rust process
exclude each other on the same lock file — a differential test, not only a
unit test. Also settles Go M0's contended-lock behavior (block with a
message naming the holder, with a timeout) for Rust.

### `[ ]` Q-PLATFORM-CRATES — memlock, masked input, pty {#q-platform-crates}

**Blocks:** M2.

- memlock: `region` vs `libc` + `windows-sys` directly, behind the `Locker`
  seam; plus core-dump suppression (`setrlimit`/`prctl` on Unix, Windows
  equivalent).
- Masked terminal input: `rpassword` vs `crossterm`.
- PTY for the few real-terminal tests: `expectrl` vs `portable-pty`, which
  must run on Windows CI (ConPTY).

---

## Open — later phases

### `[ ]` Q-SESSION-HELP-FIXTURES — How are session-mode help surfaces captured? {#q-session-help-fixtures}

**Blocks:** P3 (M6).

`tests/fixtures/go/help/` holds one-shot help only. The in-session `help`
(and session-only commands) needs an unlocked session to render, so it
can't be captured by `capture.sh` as written. Decide in P3's planning:
extend the capture script with a scripted session against a throwaway
vault, or compare through the differential harness instead.

### `[ ]` Q-DIFF-NORMALIZE — What does the differential harness normalize? {#q-diff-normalize}

**Blocks:** P2 (first differential scenarios).

UUIDs, timestamps, commit SHAs and ciphertext necessarily differ between
runs. Decide the normalization rules (placeholder substitution, comparing
decrypted content instead of ciphertext, comparing the git tree shape
instead of SHAs) and, as important, what must **not** be normalized: exit
codes, file names' shape, file modes, YAML bytes of decrypted entries.
