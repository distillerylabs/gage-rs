# A1 Mobile foundations: implementation plan

[← mobile plan](../index.md) · Parent issue
[#178](https://github.com/distillerylabs/gage-rs/issues/178) · Roadmap
[#177](https://github.com/distillerylabs/gage-rs/issues/177)

At the end of A1, a `gage-ios` app can be built on `gage-ffi` and can:

- create a vault, or clone one and enroll this device
- unlock, sync, list, search and show entries

The build that produces it is a script, and CI runs it.

A1 changes no on-disk bytes and no user-visible CLI behavior. The golden
vault and the help snapshots prove it, unedited.

---

## Where things stand at the start of A1

Measured on `main` at `dcb40b2`, 2026-11-01.

| What | State | Where |
|---|---|---|
| bindings crate | `gage-ffi`: `Vault.open`, `unlock(prompter)`, `list`; `Prompter` answers `unlock` and `warn` only | `crates/gage-ffi/src/lib.rs` |
| create a vault | CLI orchestration: identity, `Vault::create`, identity rollback, registration | `gage-cli/src/commands/init.rs` |
| join a vault | CLI orchestration: `gitrepo::clone`, directory rollback (`Prior`), config read, registration | `gage-cli/src/commands/clone.rs` |
| enroll, sync, push, pull, remote tokens | already library calls | `Vault::enroll`, `sync_resolving`, `push`, `pull`; `sync::remoteauth` |
| show, search | already library calls on `Vault` | `Vault::resolve`, `Vault::search` |
| iOS build | by hand, recipe in the spike findings | [findings](../../../00_gage-cli/plans/mobile-spike/findings.md) |
| OpenSSL on iOS | compiled and linked: git2's `openssl-sys` dependency excludes macOS only, and libssh2 needs it | `cargo tree --target aarch64-apple-ios -i openssl-sys` |
| SSH remotes | best-effort through ssh-agent ([ADR-0015](../../../../decisions/0015-ssh.md)); no agent exists on iOS | `sync/remoteauth.rs`, `sync/gitrepo/remote.rs` |
| mobile CI | none | — |

**Three findings that shape the milestones.**

- **`Session` can't cross the boundary as it is.** It borrows its
  `Prompter` for a lifetime (`Session<'a>`), and a UniFFI object can't
  hold a borrow. The bindings therefore build on `Vault`'s own methods,
  which `Session` calls too.
- **Recovery-key delivery is a frontend concern.** `init` generates a
  recovery key in the library (`new_recovery_key`) but delivers it in
  the CLI: shown on the terminal or written to a file. The library
  operation takes the recovery key's public key as a recipient. Each
  frontend delivers the private half.
- **Dropping `ssh` may not drop OpenSSL on iOS.** git2 itself depends on
  `openssl-sys` on every Unix but macOS, iOS included. A1b measures what
  is still linked once `ssh` is gone, rather than promising it all goes.

## Resolved decisions

From the planning Q&A (recorded on #177 and #178):

- **Scope.** iOS only, join + read. Writes are A2; Android is A3.
- **Minimum iOS 18.**
- **One ADR (A1b):** no SSH remotes on iOS or Android.
- **The CLI moves onto the new library operations, unchanged.** No exit
  code, message or prompt order changes. The `UnlockError::Prompter`
  kind finding from the spike is a separate issue, not part of A1,
  because fixing it changes a CLI exit code.

## Milestones

| Milestone | Status | Model | Depends on |
|---|---|---|---|
| [A1a Library create/join operations](a1a-library-create-join.md) ([#179](https://github.com/distillerylabs/gage-rs/issues/179)) | `[x]` | **Opus** | parent branch |
| [A1b iOS build + simulator CI](a1b-ios-build-ci.md) ([#180](https://github.com/distillerylabs/gage-rs/issues/180)) | `[x]` | Sonnet | A1a |
| [A1c Bindings for join + read](a1c-bindings-join-read.md) ([#181](https://github.com/distillerylabs/gage-rs/issues/181)) | `[x]` | **Opus** | A1b |

**Why Opus for A1a.**

- It moves the code that creates a private key and must never orphan
  one: an identity written for a vault that was never created, or a
  vault registered without the key that reads it.
- Rollback order is the whole point, and the CLI's tests prove the
  messages, not the order on disk.

**Why Sonnet for A1b.**

- Cargo target tables, a build script and a CI workflow, each checkable
  by running it.
- The one judgment call, refusing SSH on mobile, is settled before the
  milestone starts and recorded in an ADR.

**Why Opus for A1c.**

- Secret values (`reveal`), remote tokens and a new identity's
  passphrase cross into Swift for the first time.
- Each crossing needs its own reasoning about what is copied, what is
  wiped, and what the app is trusted with.

## Acceptance

- Every milestone doc's test list is green.
- `git diff --stat main -- tests/fixtures` is empty. The help snapshots
  and `format_v2_*` tests are green and unedited.
- The macOS mobile CI job builds the XCFramework, and the Swift smoke
  passes in a simulator.
- From Swift, in a simulator, against a local bare remote:
  1. clone a vault an existing device created through the library
  2. enroll, and get approved by that device
  3. sync, then list, search and show

  The other device runs in the smoke fixture, not the `gage` binary: an
  iOS app doesn't require the CLI, and the CLI calls the same library
  operations.
- `make lint`, `make test`, `make qa` and `make audit` are clean.
- CI is green on Linux, macOS and Windows.

## After A1

Not part of A1's acceptance.

- **iOS container paths
  ([#188](https://github.com/distillerylabs/gage-rs/issues/188)).** A
  registration records absolute paths, and iOS can change an app's
  container path across updates and restores, so a vault might stop
  opening. Not yet reproduced: confirm it first. Settle it before a real
  app ships.
- **Keychain token store
  ([#187](https://github.com/distillerylabs/gage-rs/issues/187)).** Remote
  tokens are files under the app's `Roots` today. Not urgent.
- **Real hardware.** Page locking and the vault lock have only run in the
  simulator, which uses the Mac's kernel. Test them on an iPhone.
- **`UnlockError::Prompter`'s kind
  ([#190](https://github.com/distillerylabs/gage-rs/issues/190)).** An unlock
  prompt failure always reports as `LockedOrAuth`, even when the prompt
  itself broke. It stayed out of A1 because fixing it changes a CLI exit
  code; see "Resolved decisions" above.
- **Next phases.** A2 (writes) follows once the `gage-ios` app has used
  A1's prompter and error shapes, and A3 is Android. Both are on the
  [mobile plan](../index.md), and each files its issues when it starts.

## Process notes

- **Branches stack:** `main` <- `178-a1-mobile-foundations` (these docs)
  <- A1a <- A1b <- A1c. Each milestone branch is cut from the previous
  one when the user says to start it.
- **Decisions first.** Each milestone doc lists proposals under
  "Decisions to make first". Settle them in the doc, and get them
  reviewed, before writing code. Then move them to "Settled decisions".
- **Tests first, per task.** Each task's tests are written and seen to
  fail before its implementation.
- **CLAUDE.md** now points at this track: the "current roadmap" link,
  and plan paths that name `doc/implementation/<track>/plans/` rather
  than only `00_gage-cli`.
