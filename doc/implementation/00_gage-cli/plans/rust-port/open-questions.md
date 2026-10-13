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

**Resolved: behavioral parity, not byte parity** (amended in P1 planning,
[#4](https://github.com/distillerylabs/gage-rs/issues/4)).

Go gage is the **spec**: its TDDs, its behavior, its test inventory, and its
on-disk *schema* (file names, field names, directory layout, what is
committed and what stays local). It is not a **peer**: gage-rs does not
promise that a vault written by one binary is readable or writable by the
other, and does not reproduce Go's bytes (TOML quoting, YAML style, Cobra's
help layout, usage-error wording).

*Why the amendment.* The bootstrap decision was "fully on-disk compatible":
either binary reads and writes the other's vaults, Go's bytes and help text
are authoritative, and a differential harness proves it. There are no
Go-written vaults to protect, so that cost had no beneficiary. Planning P1
found it was a large share of the phase: a go-toml-style emitter, a
Cobra-format help renderer and usage-error fixtures, a Windows lock byte
range matching Go's, Go-written fixture vaults, and a separate spike
milestone. Across the port it also drove the differential harness and
golden-byte YAML.

*What does not change.* Everything that comes from design or security
rather than interop: the library/CLI split and its lint, process-level
`vaultlock` tests, untrusted-input validation of vault ids and device
names, vault-id keying, the exit-code taxonomy (kept at Go's numbers since
there is no reason to differ), `.age-recipients` usable by stock `age`,
standard age files and the scrypt guard tests, 3-OS CI, the coverage gate,
and test-first milestones. Because the schema is the same, gage-rs will
likely read a Go vault in practice; that is neither promised nor tested.

*The risk accepted.* If Go gage gains users before the port finishes,
compatibility is expensive to retrofit. This decision assumes Go is frozen
as the reference or will be retired.

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
`--recovery-key-out`. Under behavioral parity ([Q-COMPAT](#q-compat)) this
is ordinary "not built yet", not a tracked deviation; P8 adds the flags.
Acceptable because there is no installed base.

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

## Resolved in P1 planning

Settled in [#4](https://github.com/distillerylabs/gage-rs/issues/4) and the
[P1 plan](p1-foundation/index.md).

### `[x]` Q-FILE-LOCK — Which advisory-lock primitive? {#q-file-lock}

**Resolved: `std::fs::File::lock`/`try_lock` on every platform** (stable
since Rust 1.89, which the toolchain pin satisfies). The open question was
whether a Go process and a Rust process exclude each other: Go's Windows
implementation locks one reserved byte at offset `0x7fffffff_00000000`
(so holder info at offset 0 stays readable), while std's `LockFileEx` call
covers the whole range from offset 0. Under behavioral parity
([Q-COMPAT](#q-compat)) cross-binary exclusion is not a requirement, so no
spike is needed and `fs4` is not used.

Consequence for holder info on Windows: a contender may be unable to read
the holder's `pid=`/`since=` record from a range the holder has locked.
That record is best-effort in Go too (a zero record renders as "another
process"), so M0b keeps the record but its tests assert holder detail on
Unix only, and on Windows assert only that a contended error is returned.

Contended-lock behavior (Go M0's open decision): poll every 20 ms up to a
caller-given timeout, then return a typed `Contended { path, holder }`
error. A zero timeout tries once. The OS releases the lock when the holder
exits, including on kill.

### `[x]` Q-GIT-CRATE — `gix` or `git2`? {#q-git-crate}

**Resolved: `git2` 0.21 (libgit2 1.9.7), statically vendored**, with
`default-features = false` and features `https`, `vendored-libgit2`,
`vendored-openssl`, plus `libz-sys` with `static`. No `ssh`: gage's
supported remote auth is HTTPS + token. Decided by M0b's spike
([#6](https://github.com/distillerylabs/gage-rs/issues/6)), driven by P4's
needs rather than Go interop. It is needed in P1 because
`gittest::new_bare_remote` (M0b) and `init`'s first commit (M1) both use it,
and switching after P4 would be expensive.

**Why not `gix`.** `gix` 0.89 has no push. Nothing in `gix` or
`gix-protocol` 0.67 implements the receive-pack (send-pack) client side;
the only `receive_pack` module is the fetch side receiving a pack. gage
pushes after every write (P4 · M8a), so `gix` would mean hand-writing the
push protocol, or running a second git stack beside it. Settled by source
inspection, so no `gix` code was built. Its pure-Rust build and
closeness to go-git don't outweigh a missing core operation. Revisit only if
the crate grows a push client before P4 starts.

**Evidence for `git2`.** A throwaway workspace member ran on the
`6-m0b-git-spike` branch,
[CI run 38325834646](https://github.com/distillerylabs/gage-rs/actions/runs/38325834646):
green on Linux, macOS and Windows, offline after `cargo fetch`, about
1.5–2 minutes for a cold build including vendored OpenSSL. The branch and
its code were then deleted. Every scenario ran in a child process whose
HOME, USERPROFILE, XDG_CONFIG_HOME and APPDATA pointed at a hostile global
config (`user.name`, `init.defaultBranch=hostile`, `commit.gpgSign=true`,
`core.autocrlf=true`). Each test first asserted libgit2 could see that
config, so none of them is vacuous.

| Requirement | Result |
|---|---|
| init + commit with explicit branch and signature | `RepositoryInitOptions::initial_head("main")` (bare and non-bare) and explicit `Signature`s win over the hostile config. libgit2 never signs commits on its own. |
| fetch + fast-forward from a local bare repo | Named `origin` remote, explicit refspec, `graph_ahead_behind` = (0, 1), then `reference` + `checkout_head(force)`. |
| push to a local bare repo | Works. Rejections arrive through the `push_update_reference` callback as well as the return value. |
| divergence detection | A non-fast-forward push fails with the typed `ErrorCode::NotFastForward`, so no string matching is needed (cf. Go M8a's "don't pattern-match go-git's rejection string"). After fetch, `graph_ahead_behind` = (1, 1) and `merge_base` work. |
| merge commit with chosen per-file resolutions | `merge_trees(base, ours, theirs)` gives an in-memory index. The conflicted path (opaque binary `.age`) is resolved by taking a chosen side's `IndexEntry` (clearing its stage bits), disjoint additions merge cleanly, `write_tree_to` + `commit` with parents `[local, remote]` (local first), then checkout. The result is a clean tree, and the push fast-forwards. That is the `PrepareMerge`/`PendingMerge.Commit` shape M8b needs, with no working-tree writes until commit. |
| single-path history walk | `revwalk` (topological + time), keeping commits whose blob at the path differs from any parent's. Traverses merge commits; the logic is gage's own, as in Go. |
| HTTPS token reaches the transport | libgit2 passes credentials to its *built-in* HTTP transport, not to a custom registered one (`transport::register` sees no credentials). So the hook is a **loopback HTTP server that records the `Authorization` header**. That is stronger than Go's `gagetest://` recorder, because it observes the token on the wire. The challenge path is a 401, then the `credentials` callback, then `Basic base64(user:token)` for both fetch (upload-pack) and push (receive-pack). Proactive auth like go-git's `BasicAuth` uses `FetchOptions/PushOptions::custom_headers` and sends the header on the first request, with no 401 round trip. |
| HTTPS compiled in, no system libraries | `Version::https()` true, `ssh()` false, `vendored()` true. An `https://` remote fails as a connection error, not an unsupported scheme. Backends: OpenSSL (vendored, static) on Linux, Security.framework on macOS, WinHTTP on Windows. Linux `ldd` shows only libc, libgcc_s and **libz**, so P4 adds `libz-sys/static`. |
| local transport on Windows | Works with plain `C:\...` paths for the remote URL. |

**Consequences carried into the milestones that use it:**

- **Isolate libgit2 from the user's and system git config.** libgit2
  *honors* `url.<base>.insteadOf` from the user's global config: a hostile
  rewrite redirected a fetch to a decoy repo on all three OSes. go-git
  doesn't, and the design says gage doesn't (CLAUDE.md: "`insteadOf`
  rewrites are not honored"). `git2::opts::set_search_path` set to an empty
  directory for `System`, `XDG`, `Global` and `ProgramData` before any other
  libgit2 call hides all of them (0 entries visible afterwards). It is an
  `unsafe fn` (a process-global libgit2 option), so `gitrepo` gets one
  `#[allow(unsafe_code)]` call behind a `std::sync::Once`, run before every
  repository open. That lands with `gitrepo` in M1, with a test that a
  hostile `insteadOf` is ignored. The same isolation covers `core.autocrlf`
  from Git for Windows' system config, which the spike didn't trigger, but
  gage shouldn't depend on that.
- **Pin repo-local config at init** anyway (`core.autocrlf=false`), as
  defense in depth for a vault opened by stock git or an older gage.
- **`deny.toml` must allow `Unicode-3.0`**: `git2`'s `url` → `idna` → ICU
  crates carry it. libgit2's own C code is GPLv2 with the linking
  exception, which permits static linking into an Apache-2.0 binary.
- A C dependency means following libgit2 and OpenSSL advisories:
  `make audit` (cargo-deny) covers the `-sys` crates' RustSec entries.

### `[x]` Q-AGE-CRATE — Does the `age` crate cover what gage needs? {#q-age-crate}

**Resolved for P1; the rest moves to P2 planning.** M1 needs only recipient
validation, so a malformed `--recipient` fails at the CLI boundary.

*Amended in M1:* that validation is Go's shape-only Bech32 check
(`agekey`), ported as is, not the `age` crate's `x25519::Recipient`
parsing this entry first chose. Go deliberately accepts every recipient
`.age-recipients` may legally hold, plugin recipients (`age1yubikey1…`)
included, so a vault shared with a device that uses one still round-trips
its config; X25519 parsing would refuse those, a behavior deviation. So M1
adds no `age` dependency: P2 · M2 adds it, together with the
`.age-recipients` round-trip test through the crate's reader and the
`deny.toml` license decisions its dependencies need (`curve25519-dalek` is
BSD-3-Clause).

Still to check before M2: scrypt (passphrase) recipients with a settable
work factor (shipped 2^19, plus the test-only hook and its guard tests),
armored and binary formats, `.age-recipients` parsing (comments, blank
lines), and E2's separate enrollment scrypt factor. Decrypting Go-written
files is no longer required ([Q-COMPAT](#q-compat)).

### `[x]` Q-INIT-INTERIM — What does `init` do before identities and sync exist? {#q-init-interim}

**Resolved: `init` requires `--recipient` until M2, and has no CLI
`--remote` until P4 · M8a.** M1 has no crypto, so there is no device key
to generate; as in Go's own M1, `init` fails with an error naming
`--recipient` when none is given, and the first `--recipient` key is
labelled with the device name (`--device` or the normalized hostname),
later ones `recipient-<n>`. M2 makes `--recipient` additive, as Go's
current `init` is. The library's `Vault::create` accepts a remote and sets
`origin`, but the CLI flag waits for M8a, because Go's `init --remote`
publishes immediately and that needs push. `init`'s help text describes
what M1 actually does.

### `[x]` Q-HELP-RENDER — Who renders help and usage errors? {#q-help-render}

**Resolved: clap, with grouping and availability from the registry.** The
command registry (name, aliases, short description, group, one-shot or
session availability) stays the single source, as Go's does, because P3's
in-session `help` renders from it too. clap renders the one-shot help and
usage errors; the root listing is grouped in registry order (via
`help_heading` or a help template). Go's help fixtures are a reference for
wording and grouping, not a byte gate, and Cobra's quirks (a parent with an
unknown child exiting 0, `[flags]` added on usage errors) are not
reproduced. Tests assert structure: grouping, aliases shown with the
canonical name, the one-shot set equal to the registry, and usage errors
showing the command's own usage and exiting with the usage code.

---

## Open — decide in P2 planning

Moved from P1: they block M2/M3, and P2 planning is better informed.
**Model: Opus.**

### `[ ]` Q-YAML-CRATE — Which YAML crate for entries? {#q-yaml-crate}

**Blocks:** M3.

`serde_yaml` is deprecated. Candidates: `serde_yaml_ng`, `serde_norway`,
`serde_yml`. Byte parity with Go `yaml.v3` is not required
([Q-COMPAT](#q-compat)); the requirement is a maintained crate that
round-trips the entry shape faithfully and renders readably for
`cat`/`edit`: block scalars for multi-line values, stable key order,
quoting of strings that look like numbers/bools/dates, RFC 3339
timestamps, unknown fields preserved. Go's `cat` post-processes through the
YAML node tree (#60/#61), so node-level access matters.

### `[ ]` Q-PLATFORM-CRATES — memlock, masked input, pty {#q-platform-crates}

**Blocks:** M2.

- memlock: `region` vs `libc` + `windows-sys` directly, behind the `Locker`
  seam; plus core-dump suppression (`setrlimit`/`prctl` on Unix, Windows
  equivalent).
- Masked terminal input: `rpassword` vs `crossterm`.
- PTY for the few real-terminal tests: `expectrl` vs `portable-pty`, which
  must run on Windows CI (ConPTY).

---

## Withdrawn

Kept as a record; both existed only to serve byte parity.

### `[x]` Q-SESSION-HELP-FIXTURES — How are session-mode help surfaces captured? {#q-session-help-fixtures}

**Withdrawn** with the help byte gate ([Q-HELP-RENDER](#q-help-render)).
P3's session help is tested structurally against the registry.

### `[x]` Q-DIFF-NORMALIZE — What does the differential harness normalize? {#q-diff-normalize}

**Withdrawn** with the differential harness ([Q-COMPAT](#q-compat)).
