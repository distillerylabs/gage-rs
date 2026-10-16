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

*M8a1:* the isolation landed in P4 · M8a1 (`gitrepo::isolate`), not in
M1; see [Q-GIT-ISOLATION](#q-git-isolation).

*P4 planning amendments:* the "No `ssh`" decision is reversed by
[Q-SSH](#q-ssh): git2's `ssh` feature is added for best-effort ssh-agent.
The isolation step below was assigned to M1 but did not land there. It
lands in P4 · M8a1 ([Q-GIT-ISOLATION](#q-git-isolation)). The
merge-commit row above proves only the commit shape. Merges stay
file-level and never use `merge_trees` ([Q-FILE-MERGE](#q-file-merge)).

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

*Resolved in P2 planning* ([#12](https://github.com/distillerylabs/gage-rs/issues/12)):
**`age` 0.12.1, default features only.** Checked with a throwaway spike
against the crate, not from its docs:

- **Scrypt work factor**: `scrypt::Recipient::set_work_factor(u8)` writes
  the factor into the file's `-> scrypt <salt> <logN>` stanza, so the
  shipped 19, the test hook and the guard tests all work as in Go
  ([Q-SCRYPT-HOOK](#q-scrypt-hook)). The factor is per recipient, so E2's
  separate enrollment factor needs nothing more. `scrypt::Identity` caps
  the factor it accepts at a value *calibrated per machine*
  (measured + 4), so gage sets 22 explicitly, as Go does.
- **Scrypt in debug builds** is about 50x slower than release (factor 19:
  36 s against 0.73 s), and every scrypt identity or recipient runs an
  87 ms calibration pass. `[profile.dev.package.*]` `opt-level = 3` for
  `scrypt`, `salsa20`, `pbkdf2`, `sha2` and `hmac` brings debug builds to
  release speed.
- **Errors differ from Go's.** A wrong passphrase is
  `DecryptError::DecryptionFailed` on a header where
  `Decryptor::is_scrypt()` is true; Go's age reports it as "no identity
  matched" and gage inspects the stanza types. An X25519 file offered to a
  scrypt identity is `NoMatchingKeys` with `is_scrypt()` false, so it is a
  corrupt identity file, not a wrong passphrase. For entries,
  `NoMatchingKeys` is "not a recipient" and everything else (header
  errors, `InvalidMac`, a payload read error) is corrupt ciphertext. A
  damaged scrypt stanza body is indistinguishable from a wrong passphrase,
  as in Go.
- **Mixing**: `Encryptor::with_recipients` refuses a scrypt recipient
  combined with anything, including a second scrypt recipient
  (`MixedRecipientAndPassphrase`); gage still refuses first, as Go.
- **Formats**: binary only; gage never writes armor, so the `armor`
  feature stays off. The identity file is age-keygen format and parses
  with `age::IdentityFile::from_buffer`.
- **`.age-recipients` parsing**: age's `cli_common::read_recipients`
  needs the `cli-common` feature, which pulls `rpassword` and `pinentry`
  into the library; even as a dev-dependency it would unify into the
  graph the purity check reads. The round trip test parses each
  non-blank, non-comment line with `x25519::Recipient`, the parser stock
  `age -R` uses for those lines.
- **Licences**: the dalek crates need **BSD-3-Clause** in `deny.toml`;
  everything else matches the allow list through an MIT/Apache option.

Decrypting Go-written files is not required ([Q-COMPAT](#q-compat)).

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

*P2 · M2:* the `--recipient` half is done. `init` generates this device's
identity, `--recipient` is optional and additive (this device's key first),
and the help says so. The `--remote` half still waits for P4 · M8a.

*P4 · M8a2:* the `--remote` half is done. `--remote` sets `origin`,
records it in the registration, and publishes the new vault at once,
asking for the host's token first if none is stored (#38). A failed
publish leaves the vault created and registered. Offline is a warning and
exit 0; anything else fails, saying to create the repository and run
`gage push`.

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

## Resolved in P2 planning

Settled in [#12](https://github.com/distillerylabs/gage-rs/issues/12) and
the [P2 plan](p2-crypto-entries/index.md), with throwaway spikes against
the candidate crates.

### `[x]` Q-YAML-CRATE — Which YAML crate for entries? {#q-yaml-crate}

**Resolved: `saphyr` 0.2 to parse, a gage-owned emitter to write, `jiff`
for timestamps.**

Go doesn't hand the entry to yaml.v3 to style: it builds the document as
a node tree with a chosen style per scalar, re-parses its own output, and
falls back to quoting every string if anything changed, because yaml.v3's
styling loses a leading newline and writes tabs into blocks it can't read
back. That property is gage's to own whatever the library, so the
question was only which crate *reads* entries.

- **`saphyr`** (pure Rust, YAML 1.2, MIT/Apache) loaded with
  `early_parse(false)` keeps every scalar's original text and style: an
  unknown `x: 0x1F` stays `0x1F` and is re-emitted as written. It rejects
  duplicate keys and records spans, which P6 · M12's `cat` alignment
  needs (Go's walks the node tree for line positions). Used with
  `default-features = false`; that adds **Zlib** (`foldhash`) to
  `deny.toml`. It doesn't strip a BOM and accepts several documents, so
  gage strips the one and refuses the other.
- **Rejected:** `serde_yaml_ng` reads unknown `0x1F` back as `31`, and
  offers no per-scalar style control, so Go's quoted fallback can't be
  expressed; it is also a translation of libyaml to unsafe Rust.
  `serde_norway` is the same lineage. `serde_yml` is unsound
  (RUSTSEC-2025-0068).
- **Timestamps**: `jiff` parses RFC 3339 with any offset and normalizes
  to UTC seconds, as Go's `time.Parse`. `humantime`, already a
  dependency, accepts only `Z`.

The rules for reading gage's own fields (raw text, `null` as empty, what
is a parse error) and for emitting are in
[M3](p2-crypto-entries/m3-entry-format.md).

### `[x]` Q-PLATFORM-CRATES — memlock, masked input, pty {#q-platform-crates}

**Resolved: `libc` + `windows-sys` directly, `zeroize`, `rpassword`,
`portable-pty` (dev-only).**

- **memlock**: `mlock`/`munlock` and `VirtualLock`/`VirtualUnlock` called
  directly from one `#[allow(unsafe_code)]` module, as Go uses `x/sys`.
  `region` would cover only part of it: gage also needs page-aligned
  allocation (Go's `memlock.Alloc`, so two keys never share a page lock)
  and core-dump calls. Allocation is `std::alloc` with a page-size
  `Layout`, rounded to whole pages. `zeroize` wipes, with writes the
  compiler can't remove (Go's loop has no such guarantee).
- **Core dumps** (in `gage-cli`, a process policy): `setrlimit(RLIMIT_CORE, 0)`
  on Unix and `SetErrorMode` on Windows, as Go. No
  `prctl(PR_SET_DUMPABLE)`: Go doesn't, and it also blocks debuggers.
- **Masked input**: `rpassword` 7.5 (Apache-2.0). It does its own raw-mode
  handling on the controlling terminal, and its input can be pointed at a
  path, which lets the real-terminal test hand it a PTY's slave in-process,
  as Go hands `x/term` the slave fd. `crossterm` is a TUI toolkit; nothing
  in P2 needs more than no-echo line input.
- **PTY tests**: `portable-pty` 0.9 as a `gage-cli` dev-dependency, for
  `openpty`, the slave's path and its termios. The amendment: these tests
  are **Unix only**, as Go's are (`//go:build linux || darwin`); the
  earlier note that they must run on Windows (ConPTY) went beyond Go.

### `[x]` Q-IDENTITY-SECRET — How does `Identity` hold its key? {#q-identity-secret}

**Resolved: the secret key line in a locked page-aligned buffer; the
X25519 key parsed per operation; `Drop` wipes, unlocks and frees.**

Go keeps the `AGE-SECRET-KEY-1…` line in a page-aligned, locked buffer and
records as a gap that age's parsed key sits in ordinary garbage-collected
memory. In Rust, `age::x25519::Identity` zeroizes itself on drop, but it
lives wherever its owner puts it, and a global-allocator `Box` shares
pages. So gage keeps only the key line, in a `memlock::LockedBuf`, and
parses the X25519 key for each decrypt (microseconds against the
Diffie-Hellman it precedes), dropping it straight after. The remaining
gap, recorded as Go records its own: `age`'s bech32 decoding makes
transient copies that are neither locked nor wiped.

`Drop` order is zeroize, then unlock (only if the lock took), then
deallocate. `Identity` is not `Clone`. Go's runtime checks for using an
identity after `Close` and for copies sharing one `Close` become
`compile_fail` doctests; its nil and zero-value cases are excused, since
neither is constructible.

### `[x]` Q-SCRYPT-HOOK — How do tests lower the scrypt work factor? {#q-scrypt-hook}

**Resolved: an atomic live copy, a setter that exists only in test
builds, and an idempotent fixture helper.**

Go has `const shippedScryptWorkFactor = 19`, a package variable the crypto
path reads, a `SetScryptWorkFactorForTests` forbidden outside tests by
lint, and a `TestMain` per package that lowers it. In Rust: a `const` for
the shipped value; a `static AtomicU8` live copy; the setter under
`#[cfg(any(test, feature = "test-support"))]`, so it doesn't exist in
the shipped binary at all (`gage-cli` enables `test-support` only as a
dev-dependency, which resolver 3 keeps out of normal builds). Rust has no
`TestMain`, so `testsupport::fast_scrypt()` (a `std::sync::Once`) is
called by the fixtures that create or unlock vaults; a test that forgets
it is slow, not wrong. Go's three guard tests are ported: the constant is
19, a source scan finds no other store to the atomic, and a real file
written with the shipped value carries 19 in its header.

*P2 · M2:* the last of those runs in a child process (the test binary
re-executed) that never calls the hook, rather than lowering and restoring
the live value as Go does: Rust's tests share one process and run in
parallel, so a temporary store would race every other scrypt test. The
child proves the stronger thing anyway, that the live value *starts* at the
shipped one. The source scan is a whitelist (the one declaration, `.load`
reads, the setter's body), so an alias of the static is refused too.

## Resolved in P2 implementation

### `[x]` Q-INIT-ORDER — When does `init` ask for the new passphrase? {#q-init-order}

**Resolved: after every check the flags allow, before the disk checks.**
Go's `runInit` generates the identity (and asks for the passphrase twice)
before `Create` validates `--type`, `--method` and the `--recipient` keys,
although its own comment says knowable failures should come first. gage-rs
validates those first (`CreateSpec::validate_without_device_key`), so a bad
flag never costs two blind passphrase entries. The target-directory check
stays where Go has it, inside `Vault::create` after the key exists, with
the rollback that removes the key; Go's rollback tests depend on that
ordering. Observable difference: with no passphrase available (stdin at end
of input), `gage init x --type s3` exits `2` (usage) here and `5`
(locked-or-auth) in Go (given `--no-recovery-key`, without which Go's
recovery-key planning refuses a non-terminal run first). With a passphrase
available the two agree.

## Resolved in P3 planning

Settled in [#21](https://github.com/distillerylabs/gage-rs/issues/21) and
the [P3 plan](p3-query-session/index.md). The line editor was checked
against `rustyline`'s source, not its documentation.

### `[x]` Q-LINE-EDITOR — Which line editor for the REPL? {#q-line-editor}

**Resolved: `rustyline` 18.0.1, in `gage-cli` only, behind a
`LineReader` trait.**

Go uses `chzyer/readline`; M6 needs history that records only chosen
lines, a history file nobody else can read, a completer fed by live
session data (M13), and Windows support.

- History is never added automatically (`auto_add_history` defaults to
  false), so the REPL adds each submitted, non-blank line itself and
  nothing else: no passphrase, no value.
- ~~`FileHistory` sets the file to 0600 on Unix (`fix_perm`); gage also
  creates it 0600 before the first save.~~ Amended in M6: gage owns the
  history file as Go's `history.go` does (created or tightened to 0600,
  append-only), and `rustyline` is built without default features, so
  its history is the in-memory recall list only, preloaded from the
  file. `FileHistory::save` rewrites the whole file, and a session must
  start even when the file can't be read.
- `Completer::complete(&self, line, pos, ctx)` returns candidates
  computed on demand, which M13 needs for vault names and entry titles.
- MIT. Its dependencies are expected to fall within `deny.toml`'s
  existing license list (MIT/Apache-2.0); `make audit` confirms when M6
  adds it, and any new license is recorded here. **Recorded in M6:** on
  Windows it depends on `clipboard-win` and `error-code`, both BSL-1.0
  (Boost: permissive, OSI-approved, Apache-2.0 compatible), now in
  `deny.toml`'s allow list. Neither reaches `crates/gage`.
- **The difference from Go that matters**: `rustyline` puts the terminal
  in raw mode only for the duration of a `readline()` call. Go's library
  holds the terminal and reads stdin from a goroutine for the life of an
  instance, which forced Go to send passphrase prompts through the line
  editor. gage-rs's terminal prompter keeps reading stdin directly
  between lines.

The REPL reads through a `LineReader` trait: a plain implementation over
`Io.stdin` for in-process tests and piped input, and a `rustyline` one
when stdin is a terminal. `reedline` was not taken up: it is built around
a full-screen-ish painter and has a larger dependency tree, and nothing
here needs more than line editing, history and completion.

### `[x]` Q-SESSION-SHAPE — How does `Session` hold keys, time and choices? {#q-session-shape}

**Resolved: `Option<Identity>` per vault, borrowed by commands; an
injected `Instant` clock; `Prompter::choose` for ambiguity.**

- Each vault the session has touched is a `HeldVault` whose identity is
  `Option<Identity>`; `None` is locked, and the vault stays listed so
  `status` and the prompt still show it. `lock` and the idle timeout drop
  the `Identity` through one function, so `Drop` (wipe, unlock pages) is
  the only teardown.
- Commands borrow `&Identity` from `&mut Session`. Go holds `*Identity`
  pointers and needs `ErrIdentityClosed` plus a call-depth counter so
  that a re-lock can't land under a command still holding a key; in
  Rust that overlap doesn't compile. The depth rule survives only for
  *when* the idle timeout is evaluated: once, on entering the outermost
  call, and once per REPL line through `begin_command`.
- The clock is `Box<dyn Fn() -> Instant>`, defaulting to `Instant::now`
  (monotonic, so a wall-clock change can't unlock or re-lock anything).
  A plain function suffices because the timeout is a "now against last
  use" check at the start of a call, never a background timer. The CLI
  injects it too, as Go's `App.Now`, so the tests don't depend on
  Windows' coarse clock.
- An ambiguous query in a session goes to `Prompter::choose(&CandidateList)`,
  the existing seam, which returns an id the session checks was offered.

### `[x]` Q-EDITOR — How does `edit` find `$EDITOR` and where does the plaintext go? {#q-editor}

**Resolved: Go's rules, with the `which` crate for lookup and `statfs`
for the tmpfs check.**

- `$EDITOR` is trimmed and split on whitespace; the first word is
  resolved on `PATH` with `which` (in `gage-cli` only; it handles Windows
  `PATHEXT`, which a hand-written lookup would get wrong) **before** any
  scratch file is created. Unset, blank or unresolvable is an error with
  no file written. std has no `LookPath`, and spawning would only find
  out after the plaintext is on disk.
- Scratch directory: on Linux the first of `$XDG_RUNTIME_DIR` and
  `/dev/shm` that `statfs` (via `libc`, already a dependency) reports as
  tmpfs; elsewhere `std::env::temp_dir()`. The file is created
  exclusively with an unpredictable name (`tempfile`), 0600 on Unix, and
  on every exit path overwritten with zeros at its length and unlinked,
  both best-effort, as Go. Beyond Go, on Unix the handle the file was
  written through stays open across the editor and is zeroed as well, so
  an editor that saves by renaming a new file over the path can't leave
  the original inode un-zeroed; Windows closes it before the editor runs,
  since editors there may refuse a file another process holds (P3 · M5,
  second review).
- Tests fake the editor by re-executing the test binary, the pattern the
  vault-lock, core-dump and commit-author tests already use, rather than
  adding an editor seam. The action is in a config file named on the
  editor's command line, not an environment variable, and `$EDITOR`
  itself reaches the CLI through `Io::editor` (set by `main`): in-process
  tests share one environment, and `set_var` is `unsafe` (P3 · M5).

### `[x]` Q-INDEX-ARENA — How does the metadata index hold plaintext? {#q-index-arena}

**Resolved: one `LockedBuf` arena per vault index, page-locked through
the vault's `Locker`.**

Go packs titles and descriptions into one `memlock.Alloc` arena per vault
with entries referencing offsets, because Go strings can't be zeroed and
per-entry allocation wastes up to two pages each. The Rust port keeps the
shape: a `LockedBuf` (page-aligned, zeroized on drop) holds the packed
bytes, entries hold spans, and growth allocates a larger buffer, locks
it, copies the live spans and drops the old one. Locking goes through the
vault's `Arc<dyn Locker>`, the same as its `Identity`, so the existing
counting and failing lockers prove the lock, warn-once and
release-on-discard rules with no new seam. Timestamps and `updated_by`
(a device name, already public in the vault config) stay in ordinary
memory, as in Go. The index holds no `value` or `fields` and is never
written to disk.

## Resolved in P4 planning

Settled in [#32](https://github.com/distillerylabs/gage-rs/issues/32) and
the [P4 plan](p4-sync/index.md). Where libgit2's behavior matters, each
entry names what the milestone has to pin with a test. These entries are
not a guess at libgit2's error values.

### `[x]` Q-SSH — Does gage-rs support SSH remotes? {#q-ssh}

**Resolved: yes, at Go's best-effort level, using git2's `ssh` feature
(vendored libssh2) and ssh-agent only.** This amends
[Q-GIT-CRATE](#q-git-crate)'s "No `ssh`".

Go authenticates SSH remotes through ssh-agent and never reads
`~/.ssh/config`. Two of Go's `remoteauth` tests pin that behavior. The
user chose parity over refusing SSH outright. What follows from that:

- **Credentials.** They come only from `Cred::ssh_key_from_agent`. gage
  reads no key files and asks for no passphrase.
- **No exec backend.** libgit2's exec-based SSH backend (`GIT_SSH_EXEC`)
  is never selected. It would shell out to `ssh`, which CLAUDE.md rules
  out.
- **Host aliases.** Go's heuristic is ported: an SSH host with no dot is
  treated as a `~/.ssh/config` alias. Such a remote is refused with the
  real reason, pointing at the HTTPS spelling and `gage auth login`.
  libssh2 doesn't read `~/.ssh/config` either, so the alias would
  otherwise fail as a confusing connection error.
- **Host keys.** Settled in M8a1 from libgit2 1.9.7's
  `transports/ssh_libssh2.c` (`check_certificate`): libgit2 loads
  `~/.ssh/known_hosts` itself (through its home directory, which the
  config isolation doesn't touch) and checks the host key against it.
  With no `certificate_check` callback, a host that is unknown or whose
  key doesn't match fails closed with `ErrorCode::Certificate`. gage
  therefore installs no `certificate_check`, which matches go-git's
  known_hosts default. libgit2-sys compiles only the libssh2 backend
  (`GIT_SSH_LIBSSH2`), so the exec backend is never built.
- **Build cost.** libssh2-sys is a second vendored C library on three
  operating systems. It is always built from its bundled source (unless
  `LIBSSH2_SYS_USE_PKG_CONFIG` is set, or vcpkg finds one on MSVC), links
  OpenSSL on Unix and WinCNG on Windows. `crates/gage` names it directly
  on Unix with `vendored-openssl`, so its OpenSSL is vendored without
  relying on libgit2-sys's feature wiring. M8a1: the crate is
  `MIT OR Apache-2.0`, the bundled C libssh2 BSD-3-Clause, both already
  allowed; `make audit` passed with no advisories. A local incremental
  build took 8s; the cold three-OS CI build time is checked on the
  milestone's first CI run.
- **Tests.** No network and no agent can be assumed. As in Go, the test
  pins *which path* resolving the method took: an agent credential, or an
  error naming ssh-agent. It never pins the outcome.

### `[x]` Q-GIT-ISOLATION — Where does libgit2's config isolation land? {#q-git-isolation}

**Resolved: in P4 · M8a1, before the first network operation.**
[Q-GIT-CRATE](#q-git-crate) assigned the isolation to M1. M1 shipped
explicit signatures, an explicit initial branch and the git-defaults
test, but not the `set_search_path` call. That gap was harmless while
gage-rs never contacted a remote. From the first fetch or push onward, a
user's `url.<base>.insteadOf` would redirect a vault.

- One `#[allow(unsafe_code)]` `std::sync::Once` in `gitrepo`. It points
  System, XDG, Global and ProgramData at an empty directory, and it also
  sets the server timeouts ([Q-REMOTE-TIMEOUT](#q-remote-timeout)). It
  runs before every repository open, including the opens in `gittest`.
- A child-process test runs with a hostile global config:
  `url.<x>.insteadOf` pointing at a decoy bare repository, and
  `core.autocrlf=true`.
  - It first asserts that libgit2 can see that config, so the test is not
    vacuous.
  - It then asserts that neither setting has any effect.

*M8a1:* the search path is set to an empty string rather than an empty
directory. libgit2's `git_sysdir_set` stores it as-is, which leaves each
level with no directory to search
(`isolation_empties_every_config_search_path`). gage opens repositories
with `Repository::open`, never "from env", so `GIT_CONFIG_GLOBAL`,
`GIT_CONFIG_SYSTEM` and `GIT_CONFIG_NOSYSTEM` are never read either. The
child-process test (`hostile_git_config_is_ignored`) first shows an
unisolated libgit2 seeing the hostile config and rewriting the URL; then
clone, push, fetch and fast-forward go through gage and land in the real
remote with LF line endings intact. Removing the isolation turns it red.

### `[x]` Q-SYNC-CLASSIFY — How are fetch and push failures classified? {#q-sync-classify}

**Resolved: Go's three buckets and exit codes, decided from
`git2::Error` class and code and never from the message text.
Divergence is detected positively, with elimination as the fallback.**

| Bucket | Exit code | Detected by |
|---|---|---|
| Unreachable | `Unreachable` (7) | Network, OS, TLS-connect and SSH-connect failures; gage's own deadline abort |
| Auth | `LockedOrAuth` (5) | `ErrorCode::Auth`; HTTP 401, 403 and 404 (a missing repository is treated as an access problem, as in Go) |
| Diverged | `Conflict` (1) | Push: `ErrorCode::NotFastForward`, or a status reported through `push_update_reference`. Any other push failure lands here by elimination, as in Go |
| Internal | `Internal` (6) | Fetch only: a failure in none of the buckets above passes through. A fetch is never rejected for divergence |

Go classifies divergence by elimination only because go-git's `Push`
doesn't wrap its sentinel error. libgit2 types the rejection, as the
P1 spike observed. One libgit2 behavior matters here: a ref the server
refuses arrives through the `push_update_reference` callback, while
`push()` itself can return `Ok`. A push therefore treats any status
reported there as diverged.

M8a1 triggers every row for real and records the class and code libgit2
1.9 actually produced:

- a loopback port that refuses connections
- a `.invalid` host
- a recorder answering 401, 403 and 404
- a real non-fast-forward push

The classified error carries a message with the credentials stripped
out. It never carries a raw `git2::Error` whose text might contain a URL
with a token in it.

*What M8a1 observed (libgit2 1.9.7), and where it departs from the table
above.* The full table is the doc comment on `syncerr`; each row is
triggered for real by `syncerr_tests` and `gitrepo::remote_tests`.

- A refused loopback port and a `.invalid` host are class `Net`
  (expected `Os` under WinHTTP), `GenericError`: unreachable.
- A 401 is the only typed HTTP status (`Http`, `ErrorCode::Auth`). Both
  libgit2 HTTP backends report every other non-200 status as class `Http`,
  `GenericError`, with the number only in the message. Since the message
  is never read, the whole `Http` class is auth. That puts 403 and 404
  where the table wants them, and also a 5xx or a URL that isn't a git
  repository; the message gage shows still names the real status.
- A token the remote rejects, and a credentials request from a host the
  token wasn't stored for, are refused by gage's own `credentials`
  callback; gage records the refusal and classifies it as auth without
  reading libgit2's error. Its deadline abort works the same way, as
  unreachable.
- A local path that doesn't exist is unreachable, not auth as in Go:
  libgit2 takes a remote for a local path only when the path exists, and
  otherwise reports "unsupported URL protocol" (`Net`). For a vault on an
  unmounted drive, "retry later" is the honest answer; gage names the
  path in the message. A path that exists but isn't a repository is class
  `Repository`, and is auth, as in Go.
- A non-fast-forward push is `Reference`, `ErrorCode::NotFastForward`,
  raised client-side before anything is sent. A ref the server refuses
  (proven with a locked ref on the remote) arrives as a
  `push_update_reference` status while `push()` returns `Ok`; `push`
  reports it as diverged.

### `[x]` Q-REMOTE-TIMEOUT — How is a network operation bounded without `context`? {#q-remote-timeout}

**Resolved: `REMOTE_OP_TIMEOUT` = 30s, public, enforced in two layers.**

- **libgit2's server timeouts.** The connect and read timeouts
  (`set_server_connect_timeout_in_milliseconds`,
  `set_server_timeout_in_milliseconds`) are set once inside the isolation
  `Once`. They bound a blackholed connect and a stalled read.
- **A per-operation deadline.** It is checked in the transfer-progress,
  pack-progress and sideband callbacks. Returning `false` aborts the
  operation, and the abort classifies as unreachable. Tests use a
  deadline of zero, never real time.

*M8a1:* git2's pack-builder and push-transfer progress callbacks return
nothing, so they can't abort. The deadline is checked in transfer progress
and sideband progress (fetch), in push negotiation (push, before the pack
is sent), and in the credentials callback. Tests prove a zero deadline
aborts a fetch with something to transfer and a push with something to
send, each as unreachable with nothing fetched or sent.

There is no cancellation token. Go's `context.Canceled` exists because Go
already has a `context`. No gage-rs frontend cancels a sync today, and
one can be added if the GUI needs it.

As in Go, every automatic operation and every manual verb starts its own
deadline. The push after an M8b resolution gets a fresh one, because a
human deciding between two versions of a password is not network time.

### `[x]` Q-FILE-MERGE — How does gage merge two histories? {#q-file-merge}

**Resolved: gage's own file-level three-way merge, as in Go. Never
`git2::merge_trees`.**

The P1 spike showed that `merge_trees` can produce the merge commit M8b
needs. It also merges text files line by line. `.age-recipients` is text,
so two devices that each add a recipient would merge into a union that
neither device wrote. Only the committed `.gitattributes` (`-merge`)
would stop that. Go's guarantee does not depend on that file, and the
design keeps it that way: the file binds a human running stock `git`
inside the vault, not gage.

- The merge is computed from the merge base and the tree diffs base→ours
  and base→theirs.
  - A path changed on only one side takes that side's version.
  - A path changed on both sides is a conflict, unless both sides hold
    the same blob.
  - A deletion on both sides is taken.
  - A deletion on one side with a modification on the other is a
    delete/modify conflict.
- Nothing is written to the working tree until commit. A conflict writes
  nothing at all.
- A partial failure rolls back: it removes the files the merge created,
  skips paths that aren't there, and always reaches the hard reset.
- Tree paths come from whoever can push to the remote. They go through
  `safe_join` before any filesystem write.
- `merge_remote` is `prepare_merge` followed by `PendingMerge::commit`
  with no choices, from M8a1 onward. M8b exposes the choices, `content`
  and the files to add. It doesn't split a function that M8a1 shipped
  whole, so the two paths can't drift apart
  (`TestMergeRemoteAndPrepareMergeCannotDrift`).

### `[x]` Q-REMOTE-AUTH — How does a stored token reach libgit2, and how is that tested? {#q-remote-auth}

**Resolved: through the `credentials` callback, tested on the wire
against a loopback HTTP recorder.**

- **Storage.** Tokens live at `$GAGE_STATE/tokens/<host>`, mode 0600,
  written through `atomicfile`.
  - `load` refuses a file others can read, on Unix.
  - The host is validated as a single path component before any path is
    built from it.
  - Two vaults on the same host share one token.
- **The callback, not `custom_headers`.** A header set with
  `custom_headers` is sent on every request, including one that a
  redirect sends to another host. The `credentials` callback is given the
  URL, so it offers the token only to the host it was stored for.
  `follow_redirects(Initial)` is also set explicitly.
- **No retry loop.** libgit2 calls the callback again after a 401. A
  second call within one operation fails, which then classifies as auth
  with the host-naming hint.
- *M8a1, redirects observed:* with `follow_redirects(Initial)`, libgit2
  follows a 302 on the first request to another host name and sends no
  `Authorization` there. If that host then challenges, the callback
  refuses: the token is offered only to the host it was stored for. That
  holds both when the redirect comes before the token is offered and when
  the origin accepted the token first and then redirected
  (`a_token_is_not_offered_to_a_host_it_was_not_stored_for`,
  `a_redirect_after_the_token_does_not_carry_it_to_another_host`).
- *Amended after M8a1's first Windows CI run, WinHTTP:* on Windows,
  libgit2 does HTTP(S) through WinHTTP, not its own client, and two
  differences break the guarantees above (read in libgit2 1.9.7's
  `transports/winhttp.c`):
  - It reads any `follow_redirects` mode except `None` as "follow to
    another host, on every request". It never treats `Initial` as
    initial-only.
  - Once a credential is accepted, it re-applies it to every later
    request (`apply_credentials` ignores the URL), without asking the
    `credentials` callback again.

  An origin that accepted the token and then redirected to another
  `https://` host would therefore receive the token there. gage sets
  `RemoteRedirect::None` on Windows (`gitrepo::REDIRECTS`). Unix keeps
  `Initial`. A same-host redirect is still followed. `clone`'s branch
  listing goes through `git_remote_connect`, which takes no redirect
  option, and a detached remote always gets `Initial`. So the listing
  connects through an anonymous remote on a scratch repository whose
  `http.followRedirects` carries the same policy, on every platform.
  - WinHTTP follows a cross-host redirect only to `https://`, and the
    loopback recorder speaks plain HTTP. On Windows the redirect tests
    therefore pin that the redirect is replayed to the same host and
    never reaches the other one, with no leak; they can't exercise the
    `https://` case. That case rests on `REDIRECTS` (pinned by
    `the_redirect_policy_matches_the_http_backend`) and libgit2's
    `git_net_url_apply_redirect` refusing a host change when following
    is off.
  - Also from that run: with no credentials callback installed,
    WinHTTP reports a 401 as class `Http`, `GenericError`, rather than
    `ErrorCode::Auth`. The class puts it in the auth bucket all the same
    (see Q-SYNC-CLASSIFY and the `syncerr` table).
- **No token stored** means an anonymous attempt, not an error, as in Go.
- **The test hook.** libgit2 hands credentials only to its built-in HTTP
  transport, so Go's `gagetest://` recording transport has no equivalent.
  `gittest` gets a loopback HTTP recorder built on `std::net`, with no
  new dependency. It answers 401 with `WWW-Authenticate: Basic`, records
  the `Authorization` header of the retry, and then fails. It can also
  answer 403 or 404 for the classification tests. This observes the token
  on the wire, which is a stronger check than Go's recorder.

### `[x]` Q-SYNC-PUSHED — Does a push with nothing to send report a push? {#q-sync-pushed}

**Resolved in P4 · M8a2: no. A deliberate deviation from Go, fixing Go's
known bug F2.** Go's `RemoteSyncer.Push` returns only an error and
discards `gitrepo.Push`'s "was anything sent", so `gage sync` on a vault
already in sync says it pushed. Go pins the bug in
`TestSyncReportsAPushEvenWhenNothingWasSent`, written to go red once it
is fixed.

gage-rs's `RemoteSyncer::push` returns `Result<bool, _>`, and
`SyncReport::pushed` is what it returned. So an in-sync `sync`, `push`
or `pull` says the vault "is already in sync with origin". The Go row is
ported with its assertion flipped
(`sync_reports_no_push_when_nothing_was_sent`), and
`TestSyncLineSaysAlreadyInSyncWhenThereWasNothingToDo` runs end to end,
as Go's comment asks once F2 is fixed.

### `[x]` Q-CONFLICT-PROMPTER — How does a frontend opt in to resolving conflicts? {#q-conflict-prompter}

**Resolved: a default method,
`Prompter::resolve_conflict(&EntryConflict) -> Result<Resolution, PromptError>`,
that returns a new `PromptError::NotInteractive`.**

Go makes `ConflictPrompter` a separate optional interface. A frontend with
nobody to ask simply doesn't implement it, so the refusal is structural,
and Go's existing prompters didn't have to change. A default trait method
gives Rust the same two properties:

- an implementation that says nothing refuses
- existing `Prompter` implementations compile unchanged

That is simpler than an `as_conflict_prompter()` downcast.
`TerminalPrompter` overrides the method, and still refuses when it is
not interactive. That is the path `--script`, `--stdin` and CI take.

Go's `TestNilPrompterRefusesRatherThanPanicking` is excused: a
`&dyn Prompter` cannot be null.

---

## Resolved in P5 planning

Settled in [#41](https://github.com/distillerylabs/gage-rs/issues/41) and
the [P5 plan](p5-recipients-trust/index.md).

### `[x]` Q-E1B-RETAG — Where do `recipient_batch_test.go`'s rows belong? {#q-e1b-retag}

**Resolved: in P7 · E1b, not P5.**

The phase map tagged the file P5 by its name. Its 15 tests cover E1b's
N-recipient form of `AddRecipient`:

- a slice of recipients under one lock, one trust question, one
  re-encryption pass and one commit
- extra paths deleted in that commit
- a caller-supplied failed-push clause

E1b is a pure refactor with no observable change, and its only consumer
is E4's batch `recipient approve`. Porting it in P5 would build machinery
for a caller two phases away. M9b builds the single-recipient write, and
E1b widens it, as in Go. `phase-map.awk` now tags `recipient_batch` P7,
which leaves P5 with 121 rows and P7 with 201.

### `[x]` Q-P5-SPLIT — How is Go's M9 split? {#q-p5-split}

**Resolved: M9a (identities and the read side of recipients, on Sonnet)
and M9b (recipient writes, re-encryption atomicity and the dirty-tree
reset, on Opus).**

Go's M9 is about 63 rows here. Two kinds of risk share it:

- the read side, which is additive and needs no unlock for `verify`
- the write side, whose correctness shows only after a crash

Splitting by risk follows P4's split of M8a, and keeps the crash-safety
work in a milestone of its own.

### `[x]` Q-CRASH-SEAM — How do tests interrupt a re-encryption or a move? {#q-crash-seam}

**Resolved: `#[cfg(test)]` hook fields on `Vault`, matching Go's
unexported `onReencryptEntry` and `onMoveDestCommitted`.**

Every test that needs one is a library test inside the crate (the
`*_tests.rs` modules), so `cfg(test)` is enough. No `test-support` feature
is needed, and the hooks don't exist in any non-test build.

Go's M9 review found that the hook can't reach the window after the
recipient files are written. The test for that window calls the real
`write_recipient_files` rather than hand-writing lookalike files.

~~A hook stands in for a crash by returning an error.~~ **Amended at the
start of M9b:** a hook stands in for a crash by *panicking*, as Go's
does, and the test catches the unwind (`catch_unwind`) and re-raises any
other panic. A returned error would run the error-handling path, so a
cleanup-on-failure added later would make every crash test prove that
cleanup rather than that an interruption is invisible. Unwinding runs only
`Drop`s: the lock's releases it, as the OS does for a killed process, and
the files on disk stay as the crash left them. Nothing in the library
catches an unwind. The hook is a zero-sized field outside `cfg(test)`
(the `DecryptCounter` pattern), so `Vault`'s constructors and derives are
unchanged.

### `[x]` Q-TRUST-HASH — How is `.age-recipients` hashed, and who computes the diff? {#q-trust-hash}

**Resolved: SHA-256 from `sha2`, and gage's own LCS unified diff.**

`sha2` is already in the tree through `age`. M10 adds it as a direct
dependency at that version, and `cargo tree -d` must show no second copy.

The diff is ported from Go's `unifiedDiff` and
`longestCommonSubsequence`, with Go's test. It diffs a few short TOML
files, so a diff crate would add a dependency for about a hundred
lines.

### `[x]` Q-RECIPIENT-PROMPT — How is a recipient change confirmed, and what is `--yes`? {#q-recipient-prompt}

**Resolved:
`Prompter::confirm_recipient_change(&RecipientChangeWarning) -> Result<bool, PromptError>`,
with a default that returns `PromptError::NotInteractive`, as
[Q-CONFLICT-PROMPTER](#q-conflict-prompter) does. `--yes` is a
`gage-cli` root flag that wraps the prompter and answers only this
method.**

Go asks through `ident.frontend()`, the prompter the identity was unlocked
with. The Rust `Identity` carries no prompter ([Q-IDENTITY-SECRET](#q-identity-secret)).
Every write already takes `prompter: &dyn Prompter`, and the questions go
through that parameter: M9b's self-removal `confirm`, M10's trust
question and `--repair`'s `confirm`.

The library decides cache regeneration from `verify_recipients`, so
*who* answered can't change whether the cache is regenerated. `--yes`
never answers `confirm`, so "remove this device's own key?" stays a real
question in a scripted run.

`--yes` is refused in a session, both on a session line and on the way
in. A session's writes ask through the prompter the session holds, so a
`--yes` on one line would do nothing. A `--yes` on the way in would wrap
that prompter for the session's whole life and silently approve every
change.

Settled at the start of M10 implementation:

- **The terminal prompter refuses when nobody is at it** (a deliberate
  deviation). Go's `ConfirmRecipientChange` renders and reads a line
  regardless, so on piped input it spends whatever line comes next as the
  answer: `printf 'pass\nvalue' | gage insert x --value-stdin` would
  answer with the value. gage-rs refuses with `NotInteractive` before
  drawing or reading anything, as its `resolve_conflict` does. The outcome
  is Go's bare-Enter outcome (no write, exit Conflict) without consuming
  input; `--yes` is how a script answers yes.
- **No script-mode exception yet.** Go lets `--yes` through in a
  `--script`/`--stdin` session, which is the CI case the flag exists for.
  gage-rs has no script mode, so every session is a terminal one and both
  refusals apply unconditionally. Script mode, when it is ported, brings
  Go's exception with it.
  **Amended in P6 planning:** P6 · M12d ports script mode with Go's
  exception. `--yes` is accepted in a `--script`/`--stdin` session and
  still refused in an interactive one.
- **`--yes` is hidden from help.** Go's captured help lists no inherited
  flag anywhere (`tests/fixtures/go/help/`), so the clap flag is global
  and hidden to match. It is accepted before or after the command.
- **The opportunistic sync hook is `sync_resolving`'s**, after its pull,
  as in Go's code (the M10 plan said `Vault::sync`, which takes no
  prompter). Re-selecting a held vault in a session does not warn, as
  Go's `Session.Use` doesn't.

### `[x]` Q-TWO-VAULT-LOCK — How does `mv`/`cp` lock two vaults? {#q-two-vault-lock}

**Resolved: both locks are held for the whole operation, acquired in
order of vault id. A source and destination with the same id are refused
before any lock is taken.**

Go's M11 doc orders the locks by name. Go's code after A20 orders them by
id, because the lock file is keyed by id. Ordering by something that no
longer names the lock would allow the opposite-order deadlock that the
fixed order exists to prevent. gage-rs ports the code.

The same-id refusal also matters for a Rust-specific reason. Each
`vaultlock` lock is an `flock` on its own open file description, so a
process that locks one vault twice waits out the full timeout and then
fails. Two registrations of one repository share an id, so they must be
refused up front, which is Go's
`TestMoveRejectsTwoRegistrationsOfOneVaultAsSourceAndDestination`.

### `[x]` Q-TRUST-CACHE-PATH — Where does the trust cache live? {#q-trust-cache-path}

**Resolved: `$GAGE_STATE/<vault-id>/`, holding the verbatim
`known-config.toml` and the hash of `.age-recipients`, written with
`atomicfile` and never committed. `vault remove` deletes the
directory.**

Go's M10 doc keys the cache by local name. E0 re-keyed it by id, and the
port plan keys every per-vault local path by id from the start.
`paths::trust_cache_dir` has existed since P1, but nothing calls it yet.
A half-written pair reads as "no record", so it is bootstrapped again
rather than trusted.

`vault remove` deletes the files the cache owns by name, then the
directory if it is empty, as Go's `RemoveTrustCache` does: a narrow
delete needs no argument about what else might live there.

---

## Resolved in P6 planning

Settled in [#52](https://github.com/distillerylabs/gage-rs/issues/52) and
the [P6 plan](p6-polish/index.md). The crate decisions name what each
milestone confirms against the crate before writing code; anything that
confirmation changes is recorded here.

### `[x]` Q-P6-SPLIT — How is Go's M12 split? {#q-p6-split}

**Resolved: four milestones by layer. M12a (`log`/`history --decrypt`),
M12b (`cat` display, `ls -H`, `vault remove` orphan cleanup), M12c
(`-c/--clip`, `-q/--qr`) and M12d (`--script`/`--stdin`,
`GAGE_PASSPHRASE`, the phase's docs).**

Go's M12 is 87 rows here, plus 23 that earlier phases left to it. Go
calls its items independent of each other, so the split follows what
each one needs:

- M12a is library work (a `gitrepo` history walk and two `Vault`
  methods), so it goes first.
- M12b holds the one irreversible operation in the phase (deleting an
  identity file).
- M12c brings the only new frontend crates and the only platform-specific
  behavior, so a failure on one CI platform holds up nothing else.
- M12d drives commands from every earlier milestone, including M12c's
  `-c` in a scripted session, and writes the docs once everything has
  shipped.

### `[x]` Q-CLIPBOARD — Which clipboard crate, and how is the clear driven? {#q-clipboard}

**Resolved: `arboard` in `gage-cli` only, with `default-features = false`
and `wayland-data-control`, behind a `ClipboardPort` trait. One-shot
Ctrl-C is caught with `ctrlc`.**

Go uses `atotto/clipboard`, which drives `pbcopy`/`xclip`/`wl-copy` with
the value on the child's stdin, and the user32 API on Windows. Its M12
plan flagged that putting plaintext through a child process is a real
trade-off.

- `arboard` writes in-process on every platform (X11 and Wayland through
  their protocols, AppKit, user32), so plaintext never passes through a
  child process. Default features are off to drop `image-data`;
  `wayland-data-control` keeps Go's Wayland support.
- On Linux, contents last only while a `Clipboard` lives. That matches
  Go's decision that the clear happens in the process that wrote the
  value: a one-shot `-c` blocks holding it, and a session holds it for
  its life.
- `ctrlc` sets a process-wide handler once. It is installed only on the
  one-shot `-c` path, just before the wait, and the process clears and
  exits right after, so the handler never outlives its use. Tests replace
  the wait and never install it.
- The port, the timer, the wait and the report sink for a scheduled
  clear's failure are seams in `gage-cli`, as Go's `clipboardPort`,
  `newClipboardTimer` and `ClipboardWait` are. No test touches a real
  clipboard; headless CI has none.

M12c confirms the dependency tree, licenses and `Send`-ness before
writing code, and records any new license here.

### `[x]` Q-QR-CRATE — Which QR crate, and how are the codes tested? {#q-qr-crate}

**Resolved: `qrcode` with `default-features = false`, encoding only, at
error correction level M with a 4-module quiet zone. The half-block
renderer is gage's own.**

Go uses `rsc.io/qr` and renders two modules per character cell. The
`qrcode` crate's default features pull in `image` for rendering that
gage doesn't use. Its module grid (`to_colors`) is all the renderer
needs.

Go's round-trip tests decode the rendered characters back to a grid and
compare it with the encoder's own grid for the same value, leaving "does
the encoder encode correctly" to the encoder. gage-rs does the same
against `qrcode`, so no decoder dependency is added. P8's recovery-key
display reuses this renderer.

### `[x]` Q-SCRIPT-PASSPHRASE — How does `GAGE_PASSPHRASE` reach an unlock? {#q-script-passphrase}

**Resolved: `main` reads the variable into an `Io` field, and three
`gage-cli` wrapper prompters carry Go's policy.**

- **`Io.passphrase_env`**, read once by `main`, as `editor` already is.
  In-process tests share one process environment, and Go's tests use
  `t.Setenv`. Setting the field instead keeps tests independent. Nothing
  in `crates/gage` learns the variable exists, as in Go ("a `cmd/gage`
  decorator, not a library concept").
- **`EnvPrompter`, `RefusingPrompter`, `NoCreatePrompter`**, ports of
  Go's `envPrompter`, `refusingPrompter` and `noCreatePrompter`, shaped
  like `AssumeYes`. Each answers one thing and delegates every other
  method. The source order, the unlock-purpose-only rule and the single
  attempt are Go's M12 decision, unchanged.
- **`TestTerminalPrompterOfWalksDecoratorChains` is excused.** Go needs
  `terminalPrompterOf` so a session can hand its line editor to the
  terminal prompter behind a decorator, because Go's line editor holds
  the terminal for its whole life. gage-rs's terminal prompter reads
  stdin directly between lines
  ([Q-LINE-EDITOR](#q-line-editor)), so nothing ever unwraps a prompter.
  M12d tests instead that every wrapper passes through each method it
  doesn't answer.

### `[x]` Q-ORPHAN-CLEANUP — Who decides that an identity file is orphaned? {#q-orphan-cleanup}

**Resolved: the library assesses and returns a typed value; the CLI asks
and renders.**

Go's `removeOrphanedIdentity` is entirely in `cmd/gage`: it reads the
recipient list, compares public keys, checks the registration against
the vault on disk, and asks `Confirm`. The library/CLI rule puts the
decision in `crates/gage`: a function over the registration returns
`Nothing`, `Keep { path, reason }` or `Offer { path, device }`. `gage-cli`
renders `Keep` on stderr and asks `confirm` on `Offer`. Deleting goes
through the library's identity removal.

A future GUI gets the same answer and asks its own way, and each keep
reason is testable without the CLI. The behavior is Go's after E0:
compare public keys, never names; always confirm, default no; keep and
say why in every other case.

### `[x]` Q-LOG-WALK — What do `log` and `history` walk, and how do they print? {#q-log-walk}

**Resolved while implementing M12a: Go's behavior, with these choices where
Go's code leaves room.**

- **The walk** is `Revwalk` from HEAD sorted `TIME | TOPOLOGICAL`. Go uses
  committer-time order alone; adding topological order keeps a child ahead
  of its parent when a vault commits several times in one second. A commit
  is a change when its blob at the path differs from *every* parent's, and
  absent is a different state from empty.
- **A vault-wide `log`** (no query) covers the entries the vault holds now.
  A removed entry's history is reached by naming its full id; listing every
  removed entry would mean diffing the whole history of the repository.
- **`Vault::history` returns oldest first**, the order it prints in. `log`
  returns newest first.
- **`history` prints** `== <time>  <hash>[ (deleted)]` per revision, then
  one line per compared part of the entry, marked `+`, `-` or a space. A
  deleted revision prints its header and no body.
- **Local time** needs a time zone database, so `gage-cli` takes `jiff`
  with `tz-system` (and `tzdb-bundle-platform` on Windows, which has none).
  The library keeps its instants zone-free.

---

---

## Withdrawn

Kept as a record; both existed only to serve byte parity.

### `[x]` Q-SESSION-HELP-FIXTURES — How are session-mode help surfaces captured? {#q-session-help-fixtures}

**Withdrawn** with the help byte gate ([Q-HELP-RENDER](#q-help-render)).
P3's session help is tested structurally against the registry.

### `[x]` Q-DIFF-NORMALIZE — What does the differential harness normalize? {#q-diff-normalize}

**Withdrawn** with the differential harness ([Q-COMPAT](#q-compat)).
