# ADR-0056: Test seams live off production types; `Clock` is the session's injected time

- Status: Accepted
- Origin: divergence plan D4b ([#142](https://github.com/distillerylabs/gage-rs/issues/142))

## Context

`Vault` carried eight seam fields: five test-only hooks and counters
(`DecryptCounter` ×2, `ReencryptHook`, `MoveHook`, `EnrollHook`), the
locker and syncer seams, and the production `PullObserver`. Each test-only
type was zero-sized outside tests, but each needed an always-true
`PartialEq` so that `Vault` could derive `PartialEq`, and those impls hid
what the two whole-`Vault` comparisons actually checked. The decrypt
counter and `with_remote_syncer` were `pub` under `test-support`, which no
gage-cli test used.

The session's clock was `Option<Box<dyn Fn() -> Instant>>` on
`SessionConfig`, defaulted inside `Session::new`. `Locker` required `Any`
for one test's downcast.

## Decision

**One `cfg(test)` bundle.** `Vault`'s test-only hooks and counters live in
one `#[cfg(test)] seams: TestSeams` field, in `vault/mod.rs`. Production
code calls `Vault` methods (`count_decrypt`, `after_reencrypted_entry`,
`after_deleted_path`, `after_dest_committed`, `after_enroll_pull`, ...)
whose non-test bodies are empty, so the `#[cfg]`s stay in that one file.
A shipped build has no hook or counter field. The setters
(`with_reencrypt_hook`, ...) and readers (`decrypt_count`,
`enrollment_decrypt_count`), and `with_remote_syncer`, are `cfg(test)`
and `pub(crate)`.

**What stays a field.** `LockerSeam` (gage-cli's tests inject lockers
through `test-support`), `SyncerSeam` (the `RemoteSyncer` seam, `None`
being the real `GitSyncer`) and `PullObserver` (production state). None
of them implements `PartialEq`.

**No `Vault` equality.** `Vault` derives `Debug` and `Clone`, not
`PartialEq`/`Eq`. A test that checks a constructed vault compares its
`name`, `path`, `id` and `dirs` and asserts the locker and syncer seams are
the defaults. Code that needs to know whether two vaults are the same
vault compares `id`. No `PartialEq` impl that returns `true` is allowed.

**`Clock`.** `session::Clock { fn now(&self) -> Instant }`, with
`SystemClock` as the real one, is what the idle timeout and `status`
read. `SessionConfig.clock` is `&'a dyn Clock`, not optional. Like the
prompter, it is borrowed for the session's lifetime, and it has no
`Send + Sync` bound: a session is single-threaded, and test clocks are
`Rc<Cell<_>>`. gage-cli's `Io.clock` carries it, and `main` passes
`&SystemClock`. Both types are re-exported at the root, since a frontend
names them (this adds to ADR-0055's re-export list).

**Not routed through `Clock`:**

- the `Instant::now` deadlines in `remotesync`, `vaultlock`, `gitrepo`
  and `enrollment`
- jiff `Timestamp::now` for entry and enrollment times
- `SystemTime::now`, and git signatures

They are deadlines and stamps, not session state. Their tests need no time
control, or already take `now` as a parameter (`pending_enrollments_at`,
`open_enrollment_at`).

**`Locker` without `Any`.** `trait Locker: Send + Sync`.
`default_locker_is_the_real_memlock_backed_one` asserts the constructed
vaults' locker seam is the default. It then locks and unlocks a key-sized
page-locked buffer through `LockerSeam::default()`, skipping only on a
resource refusal, as memlock's own round trip does. A `dyn Locker` can't
name its concrete type without `Any` (`type_name_of_val` names the trait
object), so the default's type is no longer checked at runtime. What
guards it is the one-line `LockerSeam::get`.

## Non-goals

- The scrypt work-factor global and its guards
  ([ADR-0024](0024-scrypt-hook.md), [ADR-0025](0025-enroll-scrypt-hook.md))
  are untouched.
- No behavior, format or command-line change.

## Consequences

- Each hook has a test that counts its calls on a known path, so a crash
  test that panics from a hook can't pass because the hook stopped being
  called.
- A new test-only hook on `Vault` goes into `TestSeams`, with a `Vault`
  method that production code calls. It is never a new field.
