# D4b — Test seams off production types

[D4 plan](index.md) · [#142](https://github.com/distillerylabs/gage-rs/issues/142) · previous: [D4a](d4a-module-hierarchy.md) · next: [D4c](d4c-typed-dispatch.md)

> **Recommended model: Opus.** The hooks this milestone moves are what
> the crash-safety tests stand on. A panic from `ReencryptHook` is a
> process killed mid re-encryption. `MoveHook` is the same, between a
> `mv`'s two vaults. `EnrollHook` proves enroll's pull and commit are
> one locked sequence. A refactor that compiles but stops *calling* a
> hook leaves those tests green and proving nothing. The locker seam is
> how page-lock failure of the identity key is proven.

## Goal

Production types carry production state:

- Test-only hooks and counters on `Vault` live in one `cfg(test)` bundle
  that doesn't exist in a shipped build.
- `Vault` stops deriving `PartialEq`/`Eq`, and with it the always-true
  `PartialEq` impls its seams needed.
- The session clock is a `Clock` trait, not a boxed closure.
- `Locker` stops requiring `Any`.

Nothing user-visible changes. Every crash-seam and page-lock test keeps
its assertions.

## Depends on

D4a. The seams move within the layout D4a settles, so this diff isn't
also a file move.

## Design references

- [D4 index](index.md), "Seams stay seams".
- CLAUDE.md, "Injectable seams for otherwise-untestable failures" and
  "Testing conventions". A fake exists only for the failure it proves.
- [ADR-0023](../../../../../decisions/0023-identity-secret.md): the
  identity secret, page-locked and zeroized in `Drop`.
- [ADR-0029](../../../../../decisions/0029-index-arena.md): the index
  arena locks pages through the vault's locker.
- [ADR-0024](../../../../../decisions/0024-scrypt-hook.md),
  [ADR-0025](../../../../../decisions/0025-enroll-scrypt-hook.md): the
  scrypt global, which this milestone does **not** touch.
- `doc/design/gage-cli-design.md`, "Why an idle timeout still matters
  despite process-scoped keys".

## Where things are

File names are as of `c856051`. D4a moves them, so re-check paths
before starting.

**`Vault`** (`vault.rs:45-81`) derives `Debug, Clone, PartialEq, Eq`. Its
seam fields:

| Field | Type | Set by | Read by | gage-cli use |
|---|---|---|---|---|
| `locker` | `LockerSeam(Option<Arc<dyn Locker>>)`, pointer `PartialEq` | `with_locker` (`test`/`test-support`) | `unlock`; `locker()` → `Index`, `recovery/swap.rs` | **yes**: `complete_tests`, `repl_tests`, `entry_tests` |
| `decrypts` | `DecryptCounter`, always-true `PartialEq` | — | `entry/store.rs` `bump()`; `decrypt_count` (`test`/`test-support`) | no |
| `enrollment_decrypts` | `DecryptCounter` | — | `enrollment/seal.rs` `bump()`; `enrollment_decrypt_count` (`cfg(test)`) | no |
| `reencrypt_hook` | `ReencryptHook { entry, delete }`, always-true | `with_reencrypt_hook`, `with_delete_hook` (`cfg(test)`) | `recipient.rs` `after_entry`, `after_delete` | no |
| `move_hook` | `MoveHook`, always-true | `with_move_hook` (`cfg(test)`) | `movecopy.rs` | no |
| `enroll_hook` | `EnrollHook`, always-true | `with_enroll_hook` (`cfg(test)`) | `enrollment/enroll.rs` | no |
| `syncer` | `SyncerSeam(Option<Arc<dyn RemoteSyncer>>)`, pointer `PartialEq` | `with_remote_syncer` (`test`/`test-support`) | `remotesync.rs` `syncer()` | no |
| `pulled` | `PullObserver`, always-true | `with_pull_observer` (production, from `Session`) | `remotesync`, `syncresolve`, `entry/crud.rs` | — |

- The full field list is written out in `create`, `from_registration`
  and `for_tests`, and in two test expectations (`vault.rs` around 1176
  and 1620). Those two `assert_eq!`s are the only whole-`Vault`
  comparisons.
- `Debug` is still needed, because about 10 `unwrap_err()` calls take a
  `Result<Vault, _>`. `Clone` is used widely.

**The clock.**

- `SessionConfig.now: Option<Box<dyn Fn() -> Instant + 'a>>`
  (`session.rs:201`) is stored as a `Box<dyn Fn>` defaulting to
  `Instant::now`, and read at four places (idle check, `status`,
  `enter`).
- gage-cli has `Io.now: &'a dyn Fn() -> Instant` (`lib.rs:85`).
  `main.rs:70` passes `&Instant::now`, and `repl.rs:65` boxes it into
  `SessionConfig`.
- Test clocks:
  - `session_tests.rs:176` `Clock(Rc<Cell<Instant>>)`. This is not
    `Send`, and **already named `Clock`**.
  - `repl_tests.rs` `step_clock`
  - `clip_tests.rs:466`, which fires a fake timer from the clock
- Out of scope: every other time read (`Instant::now` deadlines in
  `remotesync`, `vaultlock`, `gitrepo`, `enrollment`; jiff
  `Timestamp::now` for entry and enrollment times; `SystemTime::now`;
  git signatures). Enrollment already takes `now` as a parameter where
  tests need it (`pending_enrollments_at`, `open_enrollment_at`).

**`Locker: Any + Send + Sync`** (`locker.rs:14`).

- The only use of `Any` is `unlock_tests.rs:370`,
  `(&*locker as &dyn Any).is::<MemLocker>()`, in
  `default_locker_is_the_real_memlock_backed_one`. There is no
  production downcast.

## Decisions to make first

1. **The seam bundle.** Proposed:

   ```rust
   pub struct Vault { name, path, id, dirs, locker: LockerSeam,
                      syncer: SyncerSeam, pulled: PullObserver,
                      #[cfg(test)] seams: TestSeams }

   #[cfg(test)]
   #[derive(Clone, Default)]
   pub(crate) struct TestSeams {
       pub(crate) decrypts: Arc<AtomicUsize>,
       pub(crate) enrollment_decrypts: Arc<AtomicUsize>,
       pub(crate) after_reencrypted_entry: Option<Arc<dyn Fn(usize) + Send + Sync>>,
       pub(crate) after_deleted_path: Option<Arc<dyn Fn(usize) + Send + Sync>>,
       pub(crate) after_dest_committed: Option<Arc<dyn Fn() + Send + Sync>>,
       pub(crate) after_enroll_pull: Option<Arc<dyn Fn() + Send + Sync>>,
   }
   ```

   - Production call sites call `Vault` methods (`self.after_entry(n)`,
     `self.count_decrypt()`), and those methods have a `cfg(test)` body
     and an empty one otherwise. That keeps the `#[cfg]`s in one file,
     not at every call site.
   - **What `test-support` loses.** `DecryptCounter` is
     `cfg(any(test, feature = "test-support"))` today, and
     `decrypt_count` is `test-support` public. No gage-cli test calls
     either, so both become `cfg(test)`. The same goes for
     `with_remote_syncer` as a method, while `SyncerSeam` itself stays a
     field (decision 2).
   - Alternative: keep each hook type but make every one a `cfg(test)`
     field. That gives more `#[cfg]` lines and the same result.

2. **What stays a field.**
   - `LockerSeam` stays: gage-cli tests inject lockers through
     `test-support`.
   - `SyncerSeam` stays: it is the `RemoteSyncer` seam CLAUDE.md names,
     with the real `GitSyncer` as its `None`.
   - `PullObserver` is production state.
   - Proposed: all three keep their pointer-or-flag semantics, but lose
     `PartialEq`, because nothing compares them once `Vault` doesn't.

3. **`Vault` equality.** Proposed: drop `PartialEq, Eq` from `Vault`.
   - The two tests compare `name`, `path`, `id` and `dirs` explicitly,
     and assert the seams are the defaults: `locker.is_default()` and
     `syncer.is_default()`.
   - What that guards is "a constructed vault uses the real locker and
     syncer", which the always-true impls hid.
   - If anything else needs to compare two vaults, it compares `id`.
     Record that in ADR-0056.

4. **The `Clock` trait.** Proposed:

   ```rust
   pub trait Clock { fn now(&self) -> Instant; }
   pub struct SystemClock;           // Instant::now
   SessionConfig { …, clock: &'a dyn Clock }
   ```

   - **No `Send + Sync` bound.** `Session` is single-threaded and
     borrows the clock for `'a`, like `prompter`. This keeps the
     `Rc<Cell<…>>` test clocks working.
   - **Not `Option`.** gage-cli always has one (`Io.clock`, set to
     `&SystemClock` in `main.rs`), and the library tests that passed
     `None` pass `&SystemClock`. That removes the default-filling
     branch.
   - gage-cli's `Io.now` becomes `Io.clock: &'a dyn Clock`.
   - The test clock in `session_tests` is renamed `StepClock` and
     implements `Clock`. gage-cli's `step_clock` and the `clip_tests`
     clock become `impl Clock` types.
   - The trait lives beside `Session` (in `session`), and is re-exported
     at the root only if D4a's re-export rule (named in a frontend
     signature) says so. It does, because `main.rs` names `SystemClock`.
   - Scope: the session clock only. The other time reads above are
     listed in ADR-0056 as deliberately not routed through `Clock`.
     Their tests either don't need time control, or already take `now`
     as a parameter.

5. **`Locker` without `Any`.** Proposed:
   - `trait Locker: Send + Sync`, with no `Any`.
   - `default_locker_is_the_real_memlock_backed_one` asserts
     `LockerSeam::is_default()` on a constructed vault.
   - It also keeps proving that the *default path* resolves to
     `memlock`: by checking that `LockerSeam::default().get()` locks a
     page through `memlock` (a page-lock round trip on a real buffer,
     which unlock tests already do on supported platforms), or by
     comparing `type_name`.
   - Pick the one that stays meaningful on all three OSes, and record
     it here.

## Settled decisions

*(Move each decision here once it has been reviewed, as D3c did.)*

## Tasks

### 1. Prove the hooks still fire

- [ ] Tests first, against the current layout. For each hook, a test
      asserts the hook was *called* a known number of times on a known
      path. Today a panicking hook is what proves it ran, and a moved
      hook that never runs would make "the crash left state X" tests
      pass for the wrong reason. Add these counts where missing:
  - re-encrypting n entries calls `after_entry` n times
  - a recipient removal calls `after_delete` once per deleted path
  - `mv` calls `after_dest_committed` once
  - `enroll` calls `after_pull` once

  Green on `main`'s code before anything moves.

### 2. Seam bundle

- [ ] Implement decision 1. Move the setters (`with_reencrypt_hook`,
      `with_delete_hook`, `with_move_hook`, `with_enroll_hook`) onto
      `TestSeams`, or keep them as `cfg(test)` `Vault` builders that
      write into it. Production call sites are unchanged in shape.
- [ ] Narrow `decrypt_count`, `enrollment_decrypt_count` and
      `with_remote_syncer` to `cfg(test)`, per decision 1.
- [ ] `cargo build -p gage` without `test-support`: `Vault` has no hook
      or counter field. A `size_of::<Vault>()` comparison between test
      and non-test isn't possible in one build, so this check is by
      inspection, recorded in the PR.

### 3. `Vault` equality

- [ ] Tests first: rewrite the two whole-`Vault` `assert_eq!`s per
      decision 3, including the default-seam assertions. They must be
      green before the derive goes.
- [ ] Drop `PartialEq, Eq` from `Vault`. Delete every always-true
      `PartialEq`/`Eq` impl, and the pointer ones, unless something
      else still needs them.

### 4. `Clock`

- [ ] Tests first: the idle-timeout and `status` tests in
      `session_tests`, `index_tests`, `repl_tests` and `clip_tests` are
      ported to `impl Clock` clocks, with only the clock's construction
      changing.
- [ ] Implement decision 4 in the library, then in gage-cli (`Io`,
      `main.rs`, `repl.rs`, and every test `Io` literal).

### 5. `Locker`

- [ ] Rewrite `default_locker_is_the_real_memlock_backed_one` per
      decision 5, then drop `Any` and the `use std::any::Any`.

### 6. ADR and green

- [ ] ADR-0056: test seams live off production types; `Clock` is the
      session's injected time; which time reads aren't routed through
      it, and why. Link it from `doc/decisions/README.md`.
- [ ] `make lint` and `make test` are clean. `git diff --stat main --
      tests/fixtures` is empty.
- [ ] The scrypt hook and its guard tests are unchanged:
      `git diff main -- '*crypt.rs'` touches nothing in the work-factor
      section.

## Definition of done

- Every box above is ticked, and decisions 1–5 are recorded as settled.
- `Vault` derives no `PartialEq`. No `impl PartialEq … { true }` remains
  in the crate.
- No test-only hook or counter is a `Vault` field outside `cfg(test)`.
- `SessionConfig` has no closure-typed clock, and `Locker` has no `Any`
  supertrait.
- The crash-seam and page-lock-failure tests are green, with their
  assertions unedited. The new call-count tests from task 1 are green.
- `make lint`, `make test` and `make qa` are clean. The help snapshots
  and `format_v2_*` tests are green and unedited.
- CI is green on Linux, macOS and Windows.
