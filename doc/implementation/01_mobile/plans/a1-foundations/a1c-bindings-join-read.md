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

## Decisions to make first

1. **Blocking calls, not async.** Proposal: exports stay synchronous.
   - The docs say to call them off the main thread: scrypt alone takes
     about a second, and sync waits on the network.
   - UniFFI's async support needs an executor in the library, which
     nothing else here does.
2. **How a secret value crosses.**
   - Proposal: `show(identity, id)` returns the entry *without* its
     value or field values.
   - A separate `reveal(identity, id, field: Option<String>) -> String`
     returns one secret, only when the app asks for it.
   - The bindings wipe their own copy. The Swift string is outside
     their reach; the docs say so and tell the app not to keep it.
3. **Entries are addressed by id.** Proposal: `show` and `reveal` take
   the id `list` and `search` return. Free-text title resolution, and
   with it `choose`, waits until an app needs it. The CLI's query order
   is for typed queries.
4. **Where remote tokens live.** Proposal: unchanged. `remoteauth`
   writes its token files under the app's `Roots`, and the app protects
   those directories with iOS data protection. A Keychain-backed store
   would need a new seam in the library; file it as an issue for later.
5. **No new prompter questions.** The library asks nothing beyond
   `unlock` and `warn` on the join + read path:
   - `confirm_recipient_change` and `confirm` are asked only by writes
     and recipient changes (`entry/crud.rs`, `movecopy`,
     `trust/recipient`)
   - `resolve_conflict` only once this device writes
   - `value` only by insert
   - `choose` only by `Session`'s query resolution (decision 3)

   The clone's enroll offer is CLI text: an app decides from
   `Cloned`'s has-an-identity flag. A new identity's passphrase already
   reaches the app through `unlock`, with purpose `Create`. Proposal:
   the bridge keeps refusing the rest, and A2 adds them with the writes
   that ask them.
6. **How a remedy crosses.**
   - Proposal: a `Remedy` enum mirroring the library's, plus an `Other`
     case, because the library's is `#[non_exhaustive]`.
   - It rides on `Warning` and on every `GageError` case (from the
     library error's `remedy()`), so a refused token can say "log in
     again".
   - The app renders it in its own words; no CLI command text crosses.
7. **Where the docs go.** Proposal: `crates/gage-ffi/README.md`, which
   A1b started:
   - the API
   - threading
   - which secret is which
   - closing `Identity` on Kotlin
   - what the app must protect

## Settled decisions

(Moved here from above once reviewed.)

## Tasks

Write each task's tests first and see them fail, then implement. The
Rust tests drive the foreign side through the callback trait, as the
spike's do, against `gittest` bare remotes.

### 1. Remedies

- [ ] Tests:
  - [ ] a warning's remedy arrives as the matching `Remedy` case
  - [ ] an error's remedy does too (a refused token is
    `AuthLoginAgain` with its host)
  - [ ] every remedy the library has today maps to its own case. The
    library's enum is `#[non_exhaustive]`, so the bindings need a
    wildcard arm, which maps a variant added later to `Other`.
- [ ] Implement.

### 2. Create and join

- [ ] Tests:
  - [ ] `init` through the bindings creates and registers a vault that
    `list` then reads; its passphrase request arrives with purpose
    `Create`, and a cancel creates nothing
  - [ ] `clone` registers the vault and reports that this device holds
    no identity
  - [ ] `enroll` returns the code and the expiry, and publishes the
    request to the remote
  - [ ] after the CLI-side library approves the request, `sync` then
    `unlock` reads the vault
  - [ ] A1a's rollback cases surface as their typed `GageError`s
- [ ] Implement.

### 3. Remote tokens and sync

- [ ] Tests:
  - [ ] a saved token is used by the next `sync`
  - [ ] `remove` deletes it; `hosts` lists the hosts with a stored
    token
  - [ ] a token never appears in any `Debug` output or error message
  - [ ] an unreachable remote (fake `RemoteSyncer`) is
    `GageError.Unreachable`
- [ ] Implement.

### 4. Search, show, reveal

- [ ] Tests:
  - [ ] `search` returns ids and titles, and says when the match was in
    the body without saying where
  - [ ] `show` returns metadata and field names with no secret in it
  - [ ] `reveal` returns the value, or one field
  - [ ] an unknown id is `NotFound`
  - [ ] an identity that isn't a recipient fails typed
- [ ] Implement.

### 5. End to end in the simulator

- [ ] The Swift smoke from A1b grows a scenario, run in CI:
  1. a fixture creates a vault on a local bare remote
  2. Swift clones it and enrolls
  3. the fixture approves the request
  4. Swift syncs, unlocks, searches, shows and reveals
- [ ] Green in the mobile CI job.

### 6. Docs

- [ ] `crates/gage-ffi/README.md`, per decision 7.

## Definition of done

- Every box above is checked.
- `git diff --stat main -- tests/fixtures` is empty.
- `make lint`, `make test`, `make audit` are clean; CI is green on
  Linux, macOS and Windows, and the mobile job is green.
