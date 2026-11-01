# ADR-0060: Mobile bindings live in this repo; the apps don't

- Status: Accepted
- Origin: mobile bindings spike ([#172](https://github.com/distillerylabs/gage-rs/issues/172))

## Context

An iOS app, and later an Android one, needs `gage-core` in process, called
from Swift and Kotlin. The library was shaped for a second frontend
([ADR-0054](0054-frontend-neutral-errors.md),
[ADR-0055](0055-module-hierarchy.md)), but nothing yet says where the
bindings live, what generates them, or where the apps live.

Generated FFI code is `unsafe` by nature, and the workspace denies
`unsafe_code`. Whether the bindings crate needs an exception had to be
settled before it exists.

## Decision

**The bindings are a workspace crate, `crates/gage-ffi`.** It is a
frontend over the library, as `gage-cli` is: it adapts types and holds no
behavior. A library change and the binding change it forces land in one
PR, under the same toolchain pin, lints, CI and release versioning
([ADR-0058](0058-release-and-versioning.md)).

**UniFFI generates the bindings, in proc-macro mode only.** One Rust
crate yields both Swift and Kotlin. The interface is declared with
`#[uniffi::export]` and the derives; there are no UDL files and no
`include_scaffolding!`.

**`gage-ffi` takes no `unsafe` exception.** It inherits the workspace
lints unchanged. rustc does not apply `unsafe_code` to code expanded from
another crate's procedural macros, so UniFFI's generated `#[unsafe(no_mangle)]`
functions and raw-pointer blocks never reach the lint: a probe crate on
UniFFI 0.29.5 and 0.32.2 built clean under both `deny` and `forbid`. That generated
layer is trusted dependency code, like `libgit2-sys`, not code the lint
vouches for. UDL mode is excluded partly because its `include!`d
scaffolding was not tested against the lint. Hand-written `unsafe` in
`gage-ffi`, if it is ever needed, follows the existing pattern: an
item-level `#[allow(unsafe_code)]`, a `// SAFETY:` comment, and an entry in
[lint-rationale.md](../design/lint-rationale.md).

**The library never depends on UniFFI, not even behind a feature.**
`uniffi` joins the denylist in `crates/gage/tests/no_frontend_deps.rs`, and
no library type carries a UniFFI attribute. `gage-ffi` wraps or mirrors
the library types it exposes.

**Human decisions cross the boundary the way they do for the CLI.**
`Prompter` becomes a UniFFI callback interface that Swift or Kotlin
implements. Typed return values (candidate lists, warnings, unlock
requests) become UniFFI records and enums. Errors become a UniFFI error
enum keyed by `ErrorKind`, carrying the library's message unchanged
(ADR-0054).

**`Identity` crosses as an opaque object.** It is a UniFFI object held
behind an `Arc`; foreign code holds a handle and never sees key bytes.
Zeroizing and releasing the page lock still happen in `Drop`, when the
last reference is released.

**The apps live in their own repos** (`gage-ios`, later `gage-android`).
Their toolchains, signing credentials and store release cadence don't
belong in this repo's build, lint or release pipeline. An app pins a
released version of the bindings, and uses a local path override against
a checkout of this repo while both are changing.

## Consequences

- Exposing a library feature to mobile means writing its wrapper types in
  `gage-ffi`. That boilerplate is the cost of keeping UniFFI out of the
  library.
- On Kotlin, an object is released by the garbage collector unless the
  app calls `close()` (UniFFI objects are `AutoCloseable`). The Android
  app must close every `Identity` explicitly, or an unlocked key outlives
  its use. Swift's reference counting releases deterministically.
- Bumping UniFFI changes generated `unsafe` code that no lint reads.
  Review its changelog as for any FFI dependency; cargo-deny covers
  advisories.
- The platform invariants that need real syscalls (`vaultlock`, `memlock`,
  core-dump suppression) are unproven on iOS and Android until they run
  there. Mobile targets aren't in CI yet.
- UniFFI's crates are MPL-2.0. `deny.toml` allows that licence for those
  crates by name, not workspace-wide. It is file-level copyleft: shipping
  them obliges only that changes to UniFFI's own files stay MPL.
- How the Swift package and the Android library are distributed, and
  whether `gage-ffi` is published, is decided when there is something to
  distribute.
