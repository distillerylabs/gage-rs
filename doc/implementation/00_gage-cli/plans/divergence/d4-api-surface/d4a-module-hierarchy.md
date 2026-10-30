# D4a — Module hierarchy and re-exports

[D4 plan](index.md) · [#141](https://github.com/distillerylabs/gage-rs/issues/141) · next: [D4b](d4b-test-seams.md)

> **Recommended model: Sonnet.** This milestone is file moves, `use`
> paths and visibility. The compiler checks every one, and no behavior
> changes. The judgment calls are the grouping and the re-export list,
> and both are settled below before any code. The one thing to watch is
> the source-scanning guard tests: a guard that stops finding its file
> must fail, never pass vacuously.

## Goal

`crates/gage` reads as a library organised by concern, not as a port of
Go's packages:

- modules are grouped into a few top-level areas
- the types a frontend uses are re-exported at the crate root
- anything no frontend uses is `pub(crate)`
- `recipient` and `recipients` are one module

Nothing user-visible changes. The help snapshots and `format_v2_*` tests
stay green and unedited.

## Depends on

The parent branch `140-divergence-api-surface` (these docs).

## Design references

- [D4 index](index.md).
- CLAUDE.md, "Architecture: the library/CLI split is load-bearing". The
  purity lint lives in `crates/gage/clippy.toml` and the denies in
  `lib.rs`. `scripts/lint-selftest.sh` appends probes to `lib.rs`, so
  those denies must stay in `lib.rs`.
- [ADR-0024](../../../../../decisions/0024-scrypt-hook.md) and
  [ADR-0025](../../../../../decisions/0025-enroll-scrypt-hook.md): what
  the `crypt.rs` source-scan guards protect.
- [doc/design/lint-rationale.md](../../../../../design/lint-rationale.md).

## Where things are

**`lib.rs`.**

- 35 flat `pub mod`s, plus private `index` and `serde_opt`.
- `gittest` and `testsupport` are gated `cfg(any(test, feature =
  "test-support"))`.
- 15 `#[cfg(test)] mod *_tests` declarations.
- There is no `pub use`. The subdirectory modules (`entry/`,
  `enrollment/`, `recovery/`, `vaultconfig/`, `gitrepo/`, `gittest/`)
  already keep their files private and re-export from `mod.rs`.

**What gage-cli imports**, grouped by module (production code unless
marked *tests*):

| Module | Items |
|---|---|
| `vault` | `Vault`, `CreateSpec`, `LabelledRecipient`, `UnlockError`, `check_name`, `name_from_url` |
| `session` | `Session`, `SessionConfig`, `SessionError`, `OpenError` |
| `entry` | `Entry`, `NewEntry`, `ListEntry`, `Candidate`, `CandidateList`, `ResolveError`, `WriteError`, `Timestamp`, `KeyLine`, `key_layout`, `resolve_query` |
| `error` | `ErrorKind`, `HasKind`, `Remedy` |
| `prompter` | `Prompter`, `PromptError`, `Purpose`, `Kind`, `Warning`, `UnlockRequest`, `UnlockResponse`; *tests*: `FakePrompter` |
| `vaultconfig` | `VaultId`, `VaultType`, `UnlockMethod`, `Recipient`, `load`; *tests*: `CURRENT_FORMAT_VERSION`, `File`, `save` |
| `config` | `Global`, `VaultEntry`, `GitMeta`, `Shell`, `RegistryError`, `load_or_default`, `save`, `save_creating_dir` |
| `enrollment` | `EnrollmentRequest`, `Approval`, `ApprovalResult`, `EnrollmentError`, `OpenedRequest`, `PendingRequest`, `RequestCandidates`, the TTL constants |
| `recovery` | `RecoveryKey`, `RecoverResult`, `RecoverSpec`, `RecoveryError`, `RotateResult`, `RECOVERY_DEVICE_LABEL`, `check_device_label_free`, `new_recovery_key` |
| `recipient` | `RecipientChange`, `RecipientError`, `RecipientVerification` |
| `remotesync` | `ConflictKind`, `SyncError`, `SyncReport` |
| `syncresolve` | `ConflictSide`, `EntryConflict`, `Resolution`, `entry_count` |
| `syncerr` | `Kind`; *tests*: `Error::new` |
| `gitrepo` | `RemoteOptions`, `clone`, `is_clean`, `remote_url`, `set_remote`; *tests*: `commit_all`, `commit_count`, `head_commit`, `ahead_count`, `push`, `AUTHOR_*` |
| `remoteauth` | `Error`, `host`, `hosts`, `load`, `remove`, `save`, `token_host` |
| `trustcache` | `RecipientChangeWarning`, `difference_lines`, `remove` |
| `identityfile` | `IdentityBackup`, `backup`, `create`, `remove`, `MARKER_FILE_NAME` |
| `paths` | `Error`, `global_config_file`, `identity_file_path`, `vaults_dir`; *tests*: `identities_dir`, `lock_file_path` |
| `xdgpaths` | `Dirs`, `Error`, `Os` |
| `history` | `LogEntry`, `Revision` |
| `generate` | `GenerateOptions`, `DEFAULT_LENGTH`, `MIN_LENGTH`, `generate_value` |
| `orphan` | `KeepReason`, `OrphanCheck`, `assess` |
| `movecopy` | `MoveError`, `MoveResult` |
| `identity` | `Identity` |
| `devicename` | `normalize`, `valid` |
| `atomicfile` | `write_new` |
| *tests only* | `agekey::validate_recipient`, `crypt::{Recipient, encrypt, decrypt}`, `locker::{Locker, MemLocker}`, `recipients::{load, save}`, `vaultlock::Lock`, `gittest::*`, `testsupport::*` |

Nothing outside the crate uses `memlock`, `search`, `identity_add`,
`index` or `serde_opt` by name. They're reached only through `Vault`
methods.

**Recipients.**

- `recipients.rs` (170 lines) is the `.age-recipients` file: `Error`,
  `load`, `save`.
- `recipient.rs` (675 lines) is verify, add and remove. It has two
  `impl Vault` blocks, plus `RecipientWrite` and `exists`, both
  `pub(crate)`.

**`impl Vault`.** 20 blocks in 18 files, one per concern. The blocks
already follow modules, so they move with them.

**Things that pin a source path:**

- `crypt.rs:693` `library_sources()` names files by their path relative
  to `src/`, and asserts that `"crypt.rs"` was found (lines 716, 781,
  809, 904).
  - After a move, that name is `crypto/crypt.rs` on Unix and
    `crypto\crypt.rs` on Windows.
- `recovery/swap_tests.rs:817-891` walks `src/recovery` and looks for
  `swap.rs`.
- `session_tests.rs:475` uses `include_str!("session.rs")`, and its scan
  at :450-495 looks for `impl Session` and `impl Command`.
- `generate.rs:268` uses `include_str!("generate.rs")`. This works as
  long as the test stays in that file.
- Makefile:65 has `COVER_IGNORE := (^|/)gittest(/|\.rs$$)`.
- `crates/gage/tests/no_cli_text.rs` walks `src/` recursively and
  asserts more than 20 files. Directory moves are fine.
- `doc/format/v2.md` has about 33 backticked `crates/gage/src/<file>.rs`
  mentions. Others are in `doc/format/README.md:22`,
  `doc/design/lint-rationale.md:26,29` and CLAUDE.md's module list. None
  of them is a Markdown link, so `doc_links` won't catch them.

## Decisions to make first

1. **The grouping.** Proposed:

   | Area | Modules |
   |---|---|
   | `storage` | `atomicfile`, `config`, `paths`, `xdgpaths`, `vaultconfig`, `vaultlock`, `identityfile`, `serde_opt` |
   | `crypto` | `agekey`, `crypt`, `identity`, `locker`, `memlock`, `generate` |
   | `sync` | `gitrepo`, `remotesync`, `syncerr`, `syncresolve`, `remoteauth` |
   | `trust` | `recipient` (merged with `recipients`), `trustcache`, `orphan` |
   | `entry` | today's `entry/` plus `history`, `search`, `movecopy`, `index` |
   | `enrollment`, `recovery` | unchanged |
   | root | `vault`, `session`, `prompter`, `error`, `devicename`; test-gated `gittest`, `testsupport` |

   Open points:

   - **`identity_add`.** It is `Vault::add_identity`/`list_identities`,
     which register a device. It is only `impl Vault` methods plus
     their error types. Proposed: make it a private submodule
     (`vault/identities.rs`, with `vault.rs` becoming `vault/mod.rs`)
     that re-exports its error types from `vault`. Alternatively, keep
     it as its own root module.
   - **`devicename`.** It validates a path component, a security
     boundary. Proposed: keep it at the root. Moving it into `storage`
     is the alternative.
   - **`entry` growing.** Today `entry/` is 4.2k lines. Adding `index`
     keeps the session cache private to `entry`, but `Session` builds
     one, so it needs `pub(crate)`.
   - **`*_tests.rs` files** move with the module they test, declared
     from that module's `mod.rs` instead of `lib.rs`.

2. **Root re-exports.** Proposed: re-export what a frontend names in a
   signature or a match, and nothing else:
   - `Vault`, `CreateSpec`
   - `Session`, `SessionConfig`
   - `Entry`
   - `Identity`
   - `VaultId`, `VaultType`, `UnlockMethod`
   - `Prompter`, `Warning`
   - `ErrorKind`, `HasKind`, `Remedy`

   Everything else is reached through its area (`gage::sync::SyncReport`,
   `gage::entry::ResolveError`). This keeps the root short enough to
   read as the library's front page. The alternative is a flat
   re-export of every type gage-cli uses, which is about 80 names and
   recreates the flat namespace.

3. **The `pub(crate)` default.**
   - Area modules are `pub`. Their submodules are private, re-exported
     from the area's `mod.rs`.
   - An item is `pub` only if gage-cli's production code uses it, or a
     `pub` signature exposes it.
   - Items that only gage-cli tests use go behind
     `cfg(any(test, feature = "test-support"))`:
     - `crypt::{encrypt, decrypt, Recipient}`
     - `agekey::validate_recipient`
     - `locker::{Locker, MemLocker}`
     - the `.age-recipients` `load`/`save`
     - `vaultlock::Lock`
     - the gitrepo test helpers
     - `paths::{identities_dir, lock_file_path}`
     - `vaultconfig::{File, save, CURRENT_FORMAT_VERSION}`
   - `Locker`/`MemLocker` are an exception to settle: `Vault::with_locker`
     is a `test-support` method, but `Locker` appears in its signature.
     Gating the trait the same way is fine, *if* `Identity` and `Index`
     still compile without the feature. They do: they take
     `Arc<dyn Locker>` crate-internally.
   - The six pub `Vault` methods with no gage-cli caller go
     `pub(crate)`: `pending_dir`, `encrypt_entry`, `require_full_access`
     and `config_path`. `decrypt_count` and `with_remote_syncer` are
     left as they are for D4b.

4. **The recipient merge.** Proposed: `trust::recipient`, with:
   - a private `file` submodule for today's `recipients.rs`:
     `load`/`save`/`Error`, with the `test-support` re-export per
     decision 3
   - verify, add and remove from today's `recipient.rs` in the module
     body, or in an `ops` submodule if one file exceeds about 800 lines

   `recipients::Error` is renamed `RecipientsFileError` where it becomes
   visible next to `RecipientError`.

5. **No handle types for `Vault`.** The roadmap allowed splitting
   `Vault`'s impl into focused handles (`vault.entries()`,
   `vault.recipients()`). Proposed: don't, in D4.
   - Each `impl Vault` block already sits in the module for its concern,
     and keeps doing so after the move.
   - Handles would change every call site in gage-cli for no behavior
     gain.
   - `session_tests.rs` binds `Vault::insert`, `read_entry` and others
     as function pointers.

   Record this as a non-goal in ADR-0055.

6. **Path-pinned guards.** Proposed:
   - `crypt.rs`'s `library_sources()` compares a normalised relative
     path (joined with `/`) against a constant `CRYPT_SOURCE =
     "crypto/crypt.rs"`.
   - `swap_tests.rs` walks the new directory.
   - Each keeps its "found nothing" assertion. No guard is loosened to
     a `contains` or an `ends_with` that would match some other file.

## Settled decisions

Decisions 1-6 were built as proposed, with these refinements
(recorded in [ADR-0055](../../../../../decisions/0055-module-hierarchy.md)):

- **Decision 1.** `identity_add` became the private `vault::identities`
  submodule; `devicename` stays at the root; `*_tests.rs` files are
  declared by the module they test. The cross-cutting `unlock_tests`,
  `reencrypt_tests`, `twodevice_tests` and `sync_index_tests` live under
  `vault/`; `session_tests` and `format_v2_*` stay at the root.
- **Decision 3.** Area submodules are `pub mod`, not private with
  re-exports: several share function names (`load`, `save`, `remove`), so
  flattening would collide. Visibility was narrowed at module level:
  `memlock`, `search`, `index` are crate-private; `agekey`,
  `locker`, `vaultlock` and the `.age-recipients` `load`/`save` are `pub`
  only under `test-support`. `encrypt_entry`, `pending_dir`,
  `require_full_access` and `config_path` are `pub(crate)`.
- **Decision 4.** `trust::recipient` is one file module plus a private
  `file` submodule (`RecipientsFileError`); no `ops` split was needed
  (675 lines).
- **Decision 6.** `CRYPT_SOURCE = "crypto/crypt.rs"`; `swap_tests` needed
  no change since `recovery/` did not move.

## Tasks

### 1. Guards first

- [x] Before moving anything, make the path-pinned guards
      layout-independent where possible:
  - normalise `library_sources()` names to `/`
  - name the files they look for through one constant each

  Green on the current layout.
- [x] Add a test in `crates/gage/tests/` that lists `lib.rs`'s top-level
      `pub mod`s and asserts they equal the decision-1 set. It is red
      until task 3 is done, and it pins the layout afterwards.

### 2. Merge `recipient` and `recipients`

- [x] Per decision 4. Update the callers in `vault.rs`, `entry/mod.rs`,
      `entry/store.rs`, `trustcache.rs`, the tests, and the six gage-cli
      test files.

### 3. Group the modules

- [x] Move one area at a time, with `cargo check --workspace
      --all-targets` green after each: `storage`, then `crypto`, then
      `sync`, then `trust`, then `entry`.
- [x] Use `git mv` so history follows the files.
- [x] Move `*_tests.rs` declarations to their area's `mod.rs`.
- [x] Update `COVER_IGNORE` if `gittest`'s path changes, and the
      guards' constants from task 1.

### 4. Re-exports and visibility

- [x] Add the decision-2 root re-exports. Move gage-cli's imports of
      those types to the root.
- [x] Apply decision 3. Narrow item by item until `cargo check
      --workspace --all-targets` and `cargo check -p gage-cli
      --all-targets` are green. Confirm with `cargo build -p gage-cli`
      (no `test-support`) that the shipped binary still builds.
- [x] `cargo doc -p gage --no-deps` builds with no broken intra-doc
      links.

### 5. Docs and ADR

- [x] ADR-0055: module hierarchy and public surface, including
      decision 5 as a non-goal. Link it from `doc/decisions/README.md`.
- [x] Update the source paths in `doc/format/v2.md`,
      `doc/format/README.md` and `doc/design/lint-rationale.md`. This is
      path text only, not a format change.
- [x] Update CLAUDE.md's "Supporting modules" list.

### 6. Green

- [x] `make lint`, `make lint-selftest` and `make test` are clean.
- [x] `make cover` and `make cover-check` still pass, so the
      `COVER_IGNORE` change didn't widen or narrow coverage by accident.
- [x] `git diff --stat main -- tests/fixtures` is empty.

## Definition of done

- Every box above is ticked, and decisions 1–6 are recorded as settled.
- `lib.rs` declares only the decision-1 modules. The layout test from
  task 1 pins them.
- No `recipients` module remains.
- Every source-scan guard still asserts it found its file.
- `make lint`, `make test`, `make lint-selftest` and `make qa` are
  clean. The help snapshots and `format_v2_*` tests are green and
  unedited.
- CI is green on Linux, macOS and Windows. Windows matters here because
  the guards normalise path separators.
