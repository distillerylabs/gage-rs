# D4 API surface — implementation plan

[← divergence plan](../index.md) · Parent issue
[#140](https://github.com/distillerylabs/gage-rs/issues/140) · Roadmap
[#114](https://github.com/distillerylabs/gage-rs/issues/114)

D4 curates the library's public surface, now that D3 has settled its
types:

- modules are grouped by concern instead of mirroring Go's packages
- frontend types are re-exported at the crate root
- test hooks come off the production `Vault`
- the session clock becomes a trait
- gage-cli dispatches on a typed command instead of a string

D4 changes no on-disk bytes, no exit code, no error text and no help
output. The golden vault and the help snapshots prove it, unedited.

---

## Where things stand at the start of D4

Measured on `main` at `c856051`.

| What | Count | Where |
|---|---|---|
| top-level `pub mod`s | 35 | `crates/gage/src/lib.rs`, no root `pub use` |
| `impl Vault` blocks | 20 in 18 files | 46 pub, 49 `pub(crate)`, 33 private methods |
| pub `Vault` methods with no gage-cli caller | 6 | `pending_dir`, `encrypt_entry`, `require_full_access`, `config_path`, `decrypt_count`, `with_remote_syncer` |
| pub items with no user outside the crate | about 70 | worst in `gitrepo` (19), `crypt` (8), `entry` (7), `vaultconfig` (6) |
| modules used outside the crate only by gage-cli tests | 5 | `agekey`, `crypt`, `locker`, `recipients`, `vaultlock` |
| `Vault` seam fields with an always-true `PartialEq` | 5 | `DecryptCounter` ×2, `ReencryptHook`, `MoveHook`, `EnrollHook` (plus `PullObserver`, which is production) |
| string-keyed command matches in gage-cli | 5 | `commands/mod.rs` `flags`/`long_about`/`run`, `repl.rs` session verbs, the `"help"` cases in `lib.rs` and `cli.rs` |
| "not implemented yet" paths | 2 | `lib.rs`, `repl.rs`; reachable only by an injected fake command |

Regenerate with:

```
grep -c '^pub mod' crates/gage/src/lib.rs
grep -rn --include='*.rs' '^impl Vault' crates/gage/src | grep -v _tests.rs
grep -n 'impl PartialEq for' crates/gage/src/vault.rs crates/gage/src/remotesync.rs
grep -rn --include='*.rs' -E 'match (name|info\.name)|Some\("(exit|use|lock|status|help)"\)|== "help"' crates/gage-cli/src
grep -rn --include='*.rs' 'not implemented yet' crates/gage-cli/src
```

**Three findings that shape the milestones.**

- **Not every seam on `Vault` is a test hook.**
  - `LockerSeam` is injected by gage-cli tests through `test-support`
    (`complete_tests`, `repl_tests`, `entry_tests`).
  - `PullObserver` is production state set by `Session`.
  - Neither can move into a `cfg(test)` bundle, so D4b moves the hooks
    that only `crates/gage`'s own tests touch.
- **The clock is production API.** `SessionConfig.now` is fed by
  gage-cli's `Io.now`, which `main.rs` sets to `Instant::now`, and
  several gage-cli tests drive it. So it becomes a public trait, not a
  test seam.
- **Several tests and docs pin source paths.**
  - `crypt.rs`'s work-factor guards name `crypt.rs`.
  - `recovery/swap_tests.rs` names `src/recovery/swap.rs`.
  - `session_tests.rs` uses `include_str!("session.rs")`.
  - The Makefile's `COVER_IGNORE` matches `gittest`.
  - `doc/format/v2.md` names about 33 source files.

  D4a has to carry these along with the files it moves, without
  loosening them.

## Resolved decisions

See the [divergence plan](../index.md#scope-of-the-later-phases-refined-when-each-is-filed).
Specific to D4:

- **Order D4a → D4b → D4c.**
  - D4a settles where files live and what is public.
  - D4b then moves seams within that layout, so its diff isn't also a
    file move.
  - D4c touches only `gage-cli`.
- **gage-cli is the only consumer.**
  - The library has no other users and promises no semver yet, so D4a
    narrows freely.
  - An item only gage-cli *tests* need goes behind `test-support`, not
    `pub`.
  - `tests/shipped_features.rs` already proves the shipped binary is
    built without that feature.
- **Seams stay seams.**
  - CLAUDE.md's rule still holds: use the real implementation
    everywhere except the test that proves a failure.
  - D4b changes where the hooks *live*, not what they let a test
    prove. Every crash-seam test (re-encryption, `mv`, enrollment) and
    every page-lock-failure test stays green with its assertions
    unedited.
- **The scrypt global stays.**
  - ADR-0024 and ADR-0025 bind it, and its guard tests are source scans
    by design.
  - D4a only updates the file paths those scans name.
- **One ADR per milestone:**
  - ADR-0055: module hierarchy and public surface
  - ADR-0056: test seams and `Clock`
  - ADR-0057: typed command dispatch
- **Error text is unchanged.** Moving a module doesn't change a
  message. D3b already dropped the module-name prefixes.

## Milestones

| Milestone | Status | Model | Depends on |
|---|---|---|---|
| [D4a Module hierarchy + re-exports](d4a-module-hierarchy.md) ([#141](https://github.com/distillerylabs/gage-rs/issues/141), PR [#145](https://github.com/distillerylabs/gage-rs/pull/145)) | `[x]` | Sonnet | parent branch |
| [D4b Test seams off production types](d4b-test-seams.md) ([#142](https://github.com/distillerylabs/gage-rs/issues/142), PR [#146](https://github.com/distillerylabs/gage-rs/pull/146)) | `[x]` | **Opus** | D4a |
| [D4c Typed CLI dispatch](d4c-typed-dispatch.md) ([#143](https://github.com/distillerylabs/gage-rs/issues/143), PR [#147](https://github.com/distillerylabs/gage-rs/pull/147)) | `[x]` | Sonnet | D4b |

**Why Sonnet for D4a.**

- It is file moves, `use` paths and visibility, and the compiler checks
  each one.
- The only judgment calls are the grouping and the re-export list, and
  both are settled in the doc before any code.

**Why Opus for D4b.**

- The hooks it moves are what the crash-safety tests stand on:
  - a panic from `ReencryptHook` simulates a killed process mid
    re-encryption
  - `MoveHook` does the same between the two vaults of a `mv`
  - `EnrollHook` proves the pull and the commit are one locked
    sequence
- A move that compiled but stopped calling a hook would turn those tests
  into tests of nothing. They'd still pass.
- The locker seam is how page-lock failure of the identity key is
  proven.

**Why Sonnet for D4c.**

- It is a CLI-only refactor.
- 54 help snapshots and the registry-consistency tests already pin its
  output.

## Acceptance

- Every milestone doc's test list is green.
- `git diff --stat main -- tests/fixtures` is empty. The help snapshots
  and `format_v2_*` tests are green and unedited.
- `lib.rs` declares grouped modules. gage-cli takes its frontend types
  from the crate root.
- `Vault` derives no `PartialEq`. No test-only hook is a field outside
  `cfg(test)`.
- gage-cli has no string-keyed command match and no "not implemented
  yet" path.
- ADR-0055, ADR-0056 and ADR-0057 are linked from
  `doc/decisions/README.md`. CLAUDE.md's module list matches the new
  layout.
- `make lint`, `make test` and `make qa` are clean.
- CI is green on Linux, macOS and Windows.

## Process notes

- **Branches stack:** `main` <- `140-divergence-api-surface` (these
  docs) <- D4a <- D4b <- D4c. Each milestone branch is cut from the
  previous one when the user says to start it.
- **Decisions first.** Each milestone doc lists proposals under
  "Decisions to make first". Settle them in the doc, and get them
  reviewed, before writing code. Then move them to "Settled decisions",
  as D3c did.
- **Behavior proof.**
  - The help snapshots and the golden vault are not edited. A change
    that seems to need either is a CLI or format change, and needs its
    own ADR first.
  - A test whose *assertions* change is a review item. A test that only
    changes its `use` paths is not.
- **If context runs low mid-milestone,** write a `*-sessionhandoff.md`
  here.
