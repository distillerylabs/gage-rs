# A1a: Library create/join operations

[A1 plan](index.md) · [#179](https://github.com/distillerylabs/gage-rs/issues/179) · next: [A1b](a1b-ios-build-ci.md)

> **Recommended model: Opus.** This milestone moves the code that
> creates a private key and must never orphan one:
>
> - no identity file left behind for a vault that was never created
> - no vault registered without the key that reads it
> - no clone directory left half-written, and a `--dir` the user owned
>   never deleted
>
> The CLI's tests pin messages and exit codes, not the order things
> happen on disk. The new library tests have to pin that order.

## Goal

`crates/gage` has one operation that creates and registers a vault, and
one that clones and registers a vault, each rolling back its own work on
failure. `gage init` and `gage clone` are thin callers of them. No CLI
output, exit code or prompt order changes.

## Depends on

The parent branch `178-a1-mobile-foundations` (these docs).

## Design references

- [A1 index](index.md), and the spike's
  [findings](../../../00_gage-cli/plans/mobile-spike/findings.md), item 1.
- `gage-cli/src/commands/init.rs` (`run`, `publish`,
  `ensure_remote_token`) and `clone.rs` (`run`, `Prior`, `roll_back`,
  `access_lines`).
- `Vault::create` and `CreateSpec::validate_without_device_key`
  (`vault/mod.rs`), `storage::identityfile::{create, remove}`,
  `storage::config::{load_or_default, save_creating_dir}`,
  `Global::register`.
- [ADR-0006](../../../../decisions/0006-vault-id-keying.md): the id is minted before
  the identity, so the key is filed under the id the vault carries.
- [ADR-0011](../../../../decisions/0011-method-scope.md): a device's method is
  local; a clone records the vault's default.
- [ADR-0013](../../../../decisions/0013-orphan-identity-by-pubkey.md): `pubkey` is recorded at
  creation.
- The CLI tests that must stay green unedited:
  - `init_identity_tests.rs`, `init_recoverykey_tests.rs`
  - `clone_enroll_tests.rs`, `lifecycle_tests.rs`
  - `stderr_golden_tests.rs`, `help_snapshot_tests.rs`
  - `make qa`

## Where things are

**`init`** (`init.rs:100-275`), in order:

1. Plan the recovery key; check the device label is free.
2. Load the global config; refuse a registered name.
3. Choose the path: `--dir`, or `vaults_dir/<name>`.
4. Check the recovery-key output path, against the vault path.
5. Mint the id and build the `CreateSpec`.
6. `validate_without_device_key`, so a bad flag fails before any
   passphrase is typed.
7. `identityfile::create`: the passphrase is asked twice, and the key
   is written.
8. Generate the recovery key; add it as an extra recipient.
9. `Vault::create`. If that fails, `identityfile::remove`, and report a
   rollback failure beside the original.
10. Register (`device`, `pubkey`, `method`, `origin`) and save.
11. Deliver the recovery key, print the summary, publish.

**`clone`** (`clone.rs:54-165`), in order:

1. Name from the URL or `--name`; `check_name`.
2. Device from `--device` or the hostname.
3. Refuse a registered name; choose the path.
4. `Prior::of(path)` records whether the directory existed, empty.
5. `gitrepo::clone`; on failure, `roll_back`.
6. `vaultconfig::load`, refusing an unknown `format_version`; on
   failure, `roll_back`.
7. Register (`pubkey: None`, the vault's default `method`, `origin`)
   and save.
8. Print; offer to enroll if this device holds no identity.

## Decisions to make first

1. **Where the operations live, and their names.**
   - Proposal: `vault::lifecycle`, a new module beside `vault/mod.rs`.
   - It holds `Vault::init(dirs, &InitSpec, prompter) -> Result<Initialized, InitError>`
     and `Vault::clone_remote(dirs, &CloneSpec) -> Result<Cloned, CloneError>`.
   - `Initialized` carries the `Vault` and the device's public key.
     `Cloned` carries the `Vault` and whether this device already holds
     an identity for it.
2. **What stays in the frontend.**
   - **Moves to the library:** validation, the registry check, identity
     creation and its rollback, `Vault::create`, clone and its directory
     rollback, the format check, registration.
   - **Stays in the frontend:**
     - device-name defaulting (the CLI reads the hostname; an app has
       its own name)
     - the recovery key: the frontend generates it with
       `new_recovery_key` and passes its public key in, then delivers
       the private half itself
     - the recovery-key output path check, which is about a CLI flag
     - publishing and the token prompt
     - the clone's enroll offer and its text
3. **Rollback failures are typed.**
   - Proposal: `InitError::RollbackFailed { cause, rollback }` and
     `CloneError::RollbackFailed { cause, rollback }`.
   - Each one's `Display` reproduces today's CLI wording exactly, so
     `stderr_golden_tests.rs` passes unedited:
     `{err} (and rolling back the generated identity failed: {rollback})`
     and `{err} (and removing the partial clone at {path} failed: {e})`.
   - The kind stays `Internal`, as the CLI's `fail(ExitCode::Internal, ..)`
     does today.
4. **Concurrent registration.** Neither command locks the global
   config while it registers, and the per-vault `vaultlock` doesn't
   cover that file. Two processes that each load, register and save
   could lose one registration, and with it the only record pointing at
   its identity. Proposal: unchanged in A1a, since moving the code
   shouldn't change its concurrency. Confirm whether the race is real,
   and file it as its own issue if so. A phone app runs one process, so
   it doesn't raise the stakes.
5. **A registered name: one library error, two CLI messages.**
   - Today the two commands refuse a registered name differently:
     - `init` returns `RegistryError::AlreadyRegistered` (`init.rs:138`),
       `"{name:?} is already a registered vault"`
     - `clone` builds its own message with `ExitCode::Conflict`
       (`clone.rs:77`), which adds `; pass --name to clone it under a
       different name`
   - Once the check is in the library, both get the same typed error.
   - Proposal:
     - `InitError` and `CloneError` each carry
       `RegistryError::AlreadyRegistered`, whose kind is already
       `Conflict`.
     - `clone.rs` matches it and renders today's text. The `--name`
       hint names a CLI flag, so it is frontend text and stays out of
       the library's `Display`.
   - No test pins that hint today; `lifecycle_tests.rs` checks only
     `init`'s wording. Task 3 adds one before the move, so the move
     can't drop the hint unseen.
6. **A registry save that fails after the vault exists.**
   - In both commands, registering is the last step, and nothing rolls
     back if `save_creating_dir` fails:
     - `init` leaves the committed vault and this device's identity on
       disk, unregistered and unpublished
     - `clone` leaves the clone on disk, unregistered
   - A retry under the same name is blocked by the non-empty target.
     An `init` retry mints a new id, so the first identity stays
     orphaned under the old one.
   - The opening promise, "no vault registered without the key", still
     holds. Its converse doesn't: a vault and its key can exist with no
     registration pointing at them.
   - Proposal: unchanged in A1a, since rolling back here is a behavior
     change. The library returns the save failure as its own variant,
     never `RollbackFailed`. A test pins what is left behind, so a later
     fix shows up as a visible change. File the rollback as its own
     issue, alongside decision 4's race: both concern the same
     unlocked load-register-save step.

## Settled decisions

(Moved here from above once reviewed.)

## Tasks

Write each task's tests first and see them fail, then implement.

### 1. `Vault::init`

- [ ] Tests (`vault/lifecycle_tests.rs`):
  - [ ] creates the vault and registers it with the id, device,
    `pubkey`, method and origin it was given, and returns the public key
  - [ ] a registered name is refused before the prompter is asked: no
    identity file, no directory
  - [ ] an invalid spec is refused before the prompter is asked
  - [ ] the identity file is under `identities/<vault-id>/`, and the
    vault's config carries the same id
  - [ ] a `Vault::create` failure (target not empty) removes the
    identity file and registers nothing
  - [ ] the prompter cancelling the new passphrase creates nothing
  - [ ] the recovery key's public key, when given, is written after the
    device's key in both recipient files
  - [ ] `#[cfg(unix)]`: a rollback that fails (identity directory made
    read-only) is `RollbackFailed`, and its message carries both errors
  - [ ] a registered name is `AlreadyRegistered`, kind `Conflict`
    (decision 5)
  - [ ] `#[cfg(unix)]`: a registry save that fails (config directory
    made read-only) returns the save error, not `RollbackFailed`, and
    leaves the vault and identity file in place, as today (decision 6)
- [ ] Implement.

### 2. `Vault::clone_remote`

- [ ] Tests (`vault/lifecycle_tests.rs`, against `gittest` bare repos):
  - [ ] clones and registers with the vault's id, its default method,
    the origin, and no `pubkey`
  - [ ] `Cloned` reports whether this device holds an identity, for both
    answers
  - [ ] a failed clone into an absent path leaves no directory
  - [ ] a failed clone into an existing empty directory leaves it, empty
  - [ ] a non-empty target is refused and left untouched
  - [ ] an unknown `format_version` rolls the clone back and registers
    nothing
  - [ ] a registered name is refused before anything is cloned, as
    `AlreadyRegistered` (decision 5)
  - [ ] `#[cfg(unix)]`: a registry save that fails leaves the clone in
    place, unregistered, as today (decision 6)
- [ ] Implement.

### 3. Move `gage init` and `gage clone` onto them

- [ ] The CLI tests above, unedited, are the tests. Run them before
  changing anything, to confirm they're green.
- [ ] Before the move, add one CLI test, green against today's code:
  `clone` into a registered name exits `Conflict`, and its stderr
  carries the `--name` hint (decision 5).
- [ ] `init.rs` and `clone.rs` call `Vault::init` and `Vault::clone_remote`.
  They no longer call `identityfile::create`, `Vault::create`,
  `gitrepo::clone`, `Global::register` or `save_creating_dir` directly.
- [ ] `make qa`.

## Definition of done

- Every box above is checked.
- `git diff --stat main -- tests/fixtures` is empty.
- `make lint`, `make test`, `make qa` are clean; CI is green on Linux,
  macOS and Windows.
