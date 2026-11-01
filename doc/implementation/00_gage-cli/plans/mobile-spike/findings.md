# Mobile bindings spike: findings

Issue [#172](https://github.com/distillerylabs/gage-rs/issues/172). Decision record:
[ADR-0060](../../../../decisions/0060-mobile-bindings.md). Measured 2026-11-01
on Apple silicon, Xcode with the iOS 27.0 SDK, Rust 1.99.0, UniFFI 0.32.2.

## What was built

`crates/gage-ffi` exposes the smallest useful surface:

- `Vault.open(roots, name)`: the vault registered as `name`
- `vault.unlock(prompter)`, where `Prompter` is a foreign callback
- `vault.list(identity)`
- `GageError`, one case per `ErrorKind`, carrying the library's message

Its 9 Rust tests play the foreign side through the same callback trait:

- unlock and list
- attempts counted through the callback
- cancel, too many attempts, and a failing callback
- an unregistered vault, and every error kind mapped
- page lock released only when the last `Identity` handle goes
- a page-lock failure arriving as one warning

`uniffi*` is on the library's dependency denylist.

## Results

| Check | Result |
|---|---|
| `make lint`, `make test` | Clean |
| `cargo +1.95 check -p gage-ffi` (MSRV) | Clean; added to the CI MSRV job |
| `unsafe_code` under `deny` and `forbid` on 0.32.2 | Clean, as on 0.29.5 (ADR-0060 holds) |
| Build for `aarch64-apple-ios-sim` and `aarch64-apple-ios` | Clean, first try, git2 included |
| Swift bindings, generated from the `.a` (library mode) | Generated |
| Swift program on macOS, against a vault `gage init` wrote | Lists both entries; wrong passphrase is `GageError.LockedOrAuth` after attempts `[1, 2]`; unknown vault is `NotFound` |
| The same program inside an iOS 27 simulator | Same output, exit 0 |
| The same program linked for an iOS device | Links; not run (no device) |
| Kotlin bindings | Generated; `Identity` is `Disposable, AutoCloseable` with `close()`, as ADR-0060 says |
| `make audit` | **Fails**: the 8 `uniffi*` crates are MPL-2.0, not on the allowlist |

Release static library: 42.7 MiB per iOS target. The linked Swift smoke
executable: 15.0 MiB.

## The risks from the issue

**git2 cross-compiling.** Not a problem on iOS. HTTPS goes through
Apple's TLS (`GIT_SECURE_TRANSPORT`). Vendored OpenSSL is still compiled
and linked, because `libssh2-sys` needs it (the `ssh` feature, ADR-0015).
iOS has no ssh-agent for it to talk to, so it is dead weight there.
Separately, Apple has deprecated Secure Transport, which libgit2 uses here.
Android was not attempted.

**Platform syscalls.** Unproven on hardware.
- **Page locking:** `mlock` succeeded in the simulator (no warning). The
  simulator runs on the Mac's kernel, so that proves nothing about a
  device. If a device refuses, the library warns and carries on, and the
  tests prove that warning reaches the app.
- **Vault lock:** `vaultlock` ran for real in the simulator; same caveat.
- **Core dumps:** suppression lives in `gage-cli` (`coredump.rs`), so an
  app gets none of it from the library.

**File paths.** No library change needed. `Dirs` is a plain struct, so
the app passes three directories inside its container (`Roots`).

## Found along the way

1. **Creating and joining a vault is CLI-only.** `init` and `clone` do
   their orchestration in `gage-cli/src/commands/`: create the identity,
   create or clone the vault, roll back on failure, register in global
   config. The library has no single operation for either, so an app
   can't create or join a vault without duplicating that sequence. This
   is the main library change mobile needs.
2. **An unlock prompt failure is always `LockedOrAuth`.**
   `UnlockError::Prompter(_)` maps to `LockedOrAuth`
   (`storage/identityfile.rs`), even when the prompt error is `Io`, which
   `PromptError`'s own kind calls `Internal`. On the CLI, a broken
   terminal during unlock exits as an auth failure. The bindings report
   whatever the library says; the test pins that, not the kind.
3. **The deployment target must be set.** Without
   `IPHONEOS_DEPLOYMENT_TARGET`, the vendored C libraries are compiled
   for the SDK version (27.0), and an app linking them claims that
   minimum. The build script must set it.
4. **The link line** needs `-liconv` (libgit2's path handling) plus
   `Security` and `CoreFoundation`.
5. **Passphrases cross as plain strings.** The bindings wipe their copy;
   the Swift or Kotlin string is beyond their reach.
6. **The bridge answers only `unlock` and `warn`.** The library
   `Prompter`'s other questions (confirm, value, choose) are refused with
   `NotInteractive`. Nothing the bindings call asks them yet. Warnings
   cross without their `Remedy`.

## Follow-ups

- License policy for MPL-2.0 (blocks `make audit`, which runs on PRs).
- Library: a create-and-register and a clone-and-register operation, so
  `gage-cli` and the bindings share one.
- `gage-core`: drop the `ssh` feature (and with it OpenSSL) on iOS and
  Android.
- A build script for the XCFramework that sets the deployment target.
- Android: cross-compile with the NDK, then the same smoke in an emulator.
- Hardware: page locking and `vaultlock` on a real iPhone.

## Reproducing

```sh
rustup target add --toolchain 1.99.0 aarch64-apple-ios-sim aarch64-apple-ios
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
cargo build --locked --release --target aarch64-apple-ios-sim -p gage-ffi
cargo run -p gage-ffi --features bindgen --bin uniffi-bindgen -- \
  generate --library target/aarch64-apple-ios-sim/release/libgage_ffi.a \
  --language swift --out-dir out
xcrun --sdk iphonesimulator swiftc -O -target arm64-apple-ios18.0-simulator \
  -o smoke main.swift out/gage_ffi.swift -I out \
  -Xcc -fmodule-map-file=out/gage_ffiFFI.modulemap \
  -L target/aarch64-apple-ios-sim/release -lgage_ffi -liconv \
  -framework Security -framework CoreFoundation
SIMCTL_CHILD_ROOT=<dir with config/ data/ state/> xcrun simctl spawn <device> ./smoke
```

`main.swift` implements `Prompter`, opens `personal` under `$ROOT`,
unlocks and lists, then checks the wrong-passphrase and unknown-vault
errors. The vault was made with `gage init` and `gage insert` against the
same `XDG_*_HOME` directories.
