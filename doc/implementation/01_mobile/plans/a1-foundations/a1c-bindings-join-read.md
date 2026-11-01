# A1c: Bindings for join + read

[A1 plan](index.md) · [#181](https://github.com/distillerylabs/gage-rs/issues/181) · prev: [A1b](a1b-ios-build-ci.md)

> **Recommended model: Opus.** This milestone moves secret values
> (`reveal`) and remote tokens into Swift for the first time, and the
> passphrase that creates a new identity. Each crossing needs its own
> reasoning about what is copied, what is wiped and what the app is
> trusted with.

## Goal

Through `gage-ffi`, an app can:

- create a vault, or clone one, and enroll this device
- store, list and remove a remote token
- sync
- list, search and show entries

Every human decision these reach is a `Prompter` call the app answers,
and every warning and error arrives with its remedy. The bindings are documented
for an app developer.

## Depends on

A1b (the XCFramework build and the simulator CI job, which the
end-to-end test runs in).

## Design references

- [A1 index](index.md), and the spike's
  [findings](../../../00_gage-cli/plans/mobile-spike/findings.md),
  items 5 and 6.
- [ADR-0060](../../../../decisions/0060-mobile-bindings.md) (how types
  cross), [ADR-0054](../../../../decisions/0054-frontend-neutral-errors.md)
  (messages), [ADR-0033](../../../../decisions/0033-recipient-prompt.md)
  and [ADR-0021](../../../../decisions/0021-sync-conflict-resolution.md)
  (default refusals), [ADR-0038](../../../../decisions/0038-enroll-prompter.md)
  (the enrollment code as an argument).
- `crates/gage-ffi/src/lib.rs`: the spike's `Vault`, `Identity`,
  `Bridge`, `GageError`.
- A1a's `Vault::init` and `Vault::clone_remote`.
- `Vault::enroll`, `sync_resolving`, `read_entry`, `search`, `list`;
  `sync::remoteauth::{save, load, remove, hosts}`; `Remedy`
  (`error.rs`); `prompter.rs`.
- [doc/cli/enrollment.md](../../../../cli/enrollment.md): which secret
  is which. An app has to keep the passphrase and the enrollment code as
  distinct as the CLI does.

## Where things are

- **Exposed today:** `Vault.open`, `unlock(prompter)`, `list(identity)`.
- **The bridge** answers `unlock` and `warn`. `confirm`,
  `confirm_default_yes`, `value` and `choose` are refused with
  `NotInteractive`. `resolve_conflict` and `confirm_recipient_change`
  use the library's default refusal.
- **Warnings and errors** cross without their `Remedy`.
- **`Session`** can't cross as an object (it borrows its prompter), so
  everything here is built on `Vault`'s methods.

## Settled decisions

Settled before any code, on the user's instruction to make them. Each
proposal was checked against the code; where the code disagreed with the
proposal, the entry says so. Decisions 2 and 6 constrain later bindings,
so they are also [ADR-0063](../../../../decisions/0063-what-crosses-the-bindings.md).

1. **Blocking calls, not async.** As proposed.
   - Exports stay synchronous, and the docs say to call them off the main
     thread: scrypt alone takes about a second, and sync waits on the
     network.
   - UniFFI's async support needs an executor in the library, which nothing
     else here does.
   - A `Prompter` callback runs on the thread that made the call, so an app
     can block that thread while its main thread shows the dialog.
2. **How a secret crosses.** As proposed, with two corrections.
   - `show(identity, id)` returns the entry without its value or field
     values. `reveal(identity, id, field)` returns one secret, only when the
     app asks.
   - **`Entry` wipes nothing on drop**, and it can't simply gain a `Drop`:
     the library moves fields out of it. So the bindings wipe each entry they
     read, unknown fields included, with a guard of their own.
   - **The bindings can wipe only what stays in Rust.** A returned `String`
     crosses in a `RustBuffer` that Swift reads and UniFFI then frees
     *without wiping*, and no Rust code runs in between to prevent that. So
     anything going out (a revealed value, an enrollment code, a recovery
     key) leaves one unwiped heap copy behind, as well as the Swift string.
     Anything coming in (a passphrase, a token) is lifted without a copy and
     is wiped. The docs say which is which.
3. **Entries are addressed by id.** As proposed. `show` and `reveal` take
   the id `list` and `search` return. A string that isn't a UUID is `Usage`,
   before any path is built from it. `choose` stays refused.
4. **Where remote tokens live.** As proposed: `remoteauth`'s files, under
   the app's `Roots`. A Keychain-backed store would need a new seam in the
   library; that is [#187](https://github.com/distillerylabs/gage-rs/issues/187).
   `remoteauth::load` is not exported: an app stores, lists and removes a
   token, and never reads one back.
5. **No new prompter questions.** As proposed, and checked at every call
   site:
   - `confirm_recipient_change` is reached only through
     `confirm_recipient_trust`, from writes (`entry/crud.rs`, `movecopy`,
     `recovery/swap`, `trust/recipient`)
   - `confirm` only from `trust`'s recipient changes and repair
   - `resolve_conflict` only from `sync_resolving` on a real entry conflict
   - `value` only from `Session` and the CLI's insert; `choose` only from
     `Session`
   - unlock's and sync's trust check only warns
6. **How a remedy crosses.** As proposed: a mirrored `Remedy` with an
   `Other` case, on `Warning` and on every `GageError` case. The app renders
   it in its own words.
7. **Where the docs go.** As proposed: `crates/gage-ffi/README.md`.
8. **The recovery key (added: the plan left this open).** The index says each
   frontend delivers the private half, but an app has no way to make an age
   key, so without this an app-created vault could never have one.
   - `init_vault` takes `with_recovery_key`. The bindings generate the key
     through the library and return the private half only once the vault is
     created and registered, the order `gage init` keeps. A failed init
     returns no key, and the one it made is wiped.
   - It is an explicit argument with no default, so an app chooses to go
     without one; the library's `RecoveryKey` is the same type the CLI uses.
9. **Create and join are functions, not constructors (added).** UniFFI
   constructors return only the object, and `clone` has to report
   `holds_identity` as well. So `init_vault` and `clone_vault` are top-level
   functions returning a record that holds the `Vault`. The vault's directory
   is always `<data>/vaults/<name>`, the CLI's default; an app doesn't choose
   a path.
10. **`enroll` records the registration (added).** `Vault::enroll` leaves
    recording the device, method and key to its caller. The bindings do it
    after a published request, as the CLI does, so that a repeat `enroll`
    recognizes a device that is already a recipient.
11. **Publishing a new vault (added).** `init_vault` with a remote sets
    `origin` and doesn't push, as `Vault::init` documents. The app calls
    `sync`, which publishes it.

**Corrections to the test list below.**

- Task 1 expected a refused token to be `AuthLoginAgain`. In the library a
  token the remote refuses is `AuthLogin` (`syncerr::annotate_auth`), and
  `AuthLoginAgain` is a token file with loose permissions
  (`remoteauth::Error::InsecureToken`). The test covers both, as they are.
- Task 3's fake `RemoteSyncer` needs `Vault::with_remote_syncer`, which was
  private to the library's tests. It moves behind the `test-support`
  feature, as `with_locker` already is. Nothing outside a test can set it.

## Tasks

Write each task's tests first and see them fail, then implement. The
Rust tests drive the foreign side through the callback trait, as the
spike's do, against `gittest` bare remotes.

### 1. Remedies

- [x] Tests:
  - [x] a warning's remedy arrives as the matching `Remedy` case
  - [x] an error's remedy does too (a refused token is `AuthLogin` with
    its host, and a loose token file `AuthLoginAgain`: see the corrections
    above)
  - [x] every remedy the library has today maps to its own case. The
    library's enum is `#[non_exhaustive]`, so the bindings need a
    wildcard arm, which maps a variant added later to `Other`.
- [x] Implement.

### 2. Create and join

- [x] Tests:
  - [x] `init` through the bindings creates and registers a vault that
    `list` then reads; its passphrase request arrives with purpose
    `Create`, and a cancel creates nothing
  - [x] `clone` registers the vault and reports that this device holds
    no identity
  - [x] `enroll` returns the code and the expiry, and publishes the
    request to the remote
  - [x] after the CLI-side library approves the request, `sync` then
    `unlock` reads the vault
  - [x] A1a's rollback cases surface as their typed `GageError`s
- [x] Implement.

### 3. Remote tokens and sync

- [x] Tests:
  - [x] a saved token is used by the next `sync`
  - [x] `remove` deletes it; `hosts` lists the hosts with a stored
    token
  - [x] a token never appears in any `Debug` output or error message
  - [x] an unreachable remote (fake `RemoteSyncer`) is
    `GageError.Unreachable`
- [x] Implement.

### 4. Search, show, reveal

- [x] Tests:
  - [x] `search` returns ids and titles, and says when the match was in
    the body without saying where
  - [x] `show` returns metadata and field names with no secret in it
  - [x] `reveal` returns the value, or one field
  - [x] an unknown id is `NotFound`
  - [x] an identity that isn't a recipient fails typed
- [x] Implement.

### 5. End to end in the simulator

- [x] The Swift smoke from A1b grows a scenario, run in CI:
  1. a fixture creates a vault on a local bare remote
  2. Swift clones it and enrolls
  3. the fixture approves the request
  4. Swift syncs, unlocks, searches, shows and reveals
- [ ] Green in the mobile CI job.

### 6. Docs

- [x] `crates/gage-ffi/README.md`, per decision 7.

## Definition of done

- Every box above is checked.
- `git diff --stat main -- tests/fixtures` is empty.
- `make lint`, `make test`, `make audit` are clean; CI is green on
  Linux, macOS and Windows, and the mobile job is green.
