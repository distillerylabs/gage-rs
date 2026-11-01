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

None open. All six are settled below.

## Settled decisions

Settled 2026-11-01, from the proposals this section held. Where a
proposal changed, the change is said.

1. **Where the operations live, and their names.** As proposed, plus
   what the proposal left open:
   - `vault::lifecycle` (`vault/lifecycle.rs`), tests in
     `vault/lifecycle_tests.rs`. `Vault::init(dirs, &InitSpec, prompter)
     -> Result<Initialized, InitError>` and `Vault::clone_remote(dirs,
     &CloneSpec) -> Result<Cloned, CloneError>`.
   - **`Vault::init` mints the id; `InitSpec` has no id field.** The id
     and the identity file are then made by one function, so ADR-0006's
     ordering can't be got wrong by a caller. It also means the identity
     file is always new: `identityfile::create` reuses a file already at
     the path, and rolling back a reused file would delete a key that
     existed before the call. A fresh id makes that impossible.
   - **The path is the caller's.** `InitSpec.path` and `CloneSpec.path`
     are required. The CLI's default stays `paths::vaults_dir(dirs)
     .join(name)`, and the CLI needs the path before the call anyway, to
     check `--recovery-key-out` against it.
   - **The recovery key is `InitSpec.recovery_key: Option<String>`**, its
     public key. The library applies `RECOVERY_DEVICE_LABEL`, so no
     frontend can file it under the wrong label. The other recipients are
     `InitSpec.recipients`, written after the device's key, as
     `--recipient` is today.
   - `Initialized { vault, pubkey }`. `Cloned { vault, holds_identity }`.
     `holds_identity` is true when this device has an identity file for
     the vault, and also when that can't be checked: the CLI leaves both
     cases alone today.
2. **What stays in the frontend.** As proposed. Also in the frontend:
   inferring a clone's name from its URL (`vault::name_from_url`, a
   library function the CLI calls). The library still runs `check_name`
   on whatever name it is given, and checks the device name, since a GUI
   may call it directly.
   - One consequence for `init`: the CLI generates the recovery key
     before calling `Vault::init`, so before the passphrase prompt rather
     than after it. Nothing is shown or written any earlier, and prompt
     order is unchanged. Only a failing system RNG could tell the
     difference.
   - One duplicated check: `init.rs` keeps its read-only registry check
     ahead of `check_recovery_key_out_path`. Without it, a registered
     name with a bad `--recovery-key-out` would report the path instead
     of the name. `Vault::init` checks again, and its check is the one
     that holds.
3. **Rollback failures are typed.** As proposed.
   - `InitError::RollbackFailed { cause: CreateError, rollback:
     Box<UnlockError> }`: only a `Vault::create` failure rolls back. The
     box is clippy's `result_large_err`, nothing more.
   - `CloneError::RollbackFailed { cause: Box<CloneError>, path,
     rollback: std::io::Error }`: a git failure or a config failure.
   - Display is today's wording; kind `Internal`. No CLI test pins
     either message today, so the library tests pin them.
   - **Test seam.** Making the identity rollback fail needs the identity
     directory made read-only after it is created and before it is
     removed, and the id is only known inside `Vault::init`. A
     `#[cfg(test)]` thread-local hook in `lifecycle.rs`, called right
     after the identity is written with the new id, does it. There is no
     `Vault` yet at that point, so ADR-0056's `TestSeams` can't hold it.
     Its non-test body is empty, and a test counts its calls.
4. **Concurrent registration.** Unchanged in A1a. The race is real: both
   commands load the global config before their slow step (a passphrase
   typed twice, or a network clone), then save the whole file. A second
   process that registers in between is overwritten. Filed as
   [#183](https://github.com/distillerylabs/gage-rs/issues/183).
5. **A registered name: one library error, two CLI messages.** As
   proposed. `InitError::Registry` and `CloneError::Registry` carry
   `RegistryError::AlreadyRegistered`; `clone.rs` matches it and adds the
   `--name` hint.
   - Correction: a test does pin the hint already.
     `sync_tests.rs::clone_refuses_an_already_registered_name` checks
     exit `Conflict` and `--name` in stderr. Task 3 tightens it to the
     whole message rather than adding a second test.
6. **A registry save that fails after the vault exists.** As proposed:
   unchanged in A1a, its own variant (`InitError::SaveRegistry`,
   `CloneError::SaveRegistry`), never `RollbackFailed`, and a test pins
   what is left behind. Filed as
   [#184](https://github.com/distillerylabs/gage-rs/issues/184).

**Recorded as [ADR-0061](../../../../decisions/0061-library-creates-and-registers.md):**
creating or joining a vault is one library operation that writes the
registration too. Until now the library never wrote global config
(`identity add` still doesn't).

## Tasks

Write each task's tests first and see them fail, then implement.

### 1. `Vault::init`

- [x] Tests (`vault/lifecycle_tests.rs`):
  - [x] creates the vault and registers it with the id, device,
    `pubkey`, method and origin it was given, and returns the public key
  - [x] a registered name is refused before the prompter is asked: no
    identity file, no directory
  - [x] an invalid spec is refused before the prompter is asked
  - [x] the identity file is under `identities/<vault-id>/`, and the
    vault's config carries the same id
  - [x] a `Vault::create` failure (target not empty) removes the
    identity file and registers nothing
  - [x] the prompter cancelling the new passphrase creates nothing
  - [x] the recovery key's public key, when given, is written after the
    device's key in both recipient files
  - [x] `#[cfg(unix)]`: a rollback that fails (identity directory made
    read-only) is `RollbackFailed`, kind `Internal`, and its message is
    today's wording with both errors (decision 3)
  - [x] the test hook runs once per identity created, with the id the
    vault carries (decision 3)
  - [x] a registered name is `AlreadyRegistered`, kind `Conflict`
    (decision 5)
  - [x] `#[cfg(unix)]`: a registry save that fails (config directory
    made read-only) returns the save error, not `RollbackFailed`, and
    leaves the vault and identity file in place, as today (decision 6)
- [x] Implement.

### 2. `Vault::clone_remote`

- [x] Tests (`vault/lifecycle_tests.rs`, against `gittest` bare repos):
  - [x] clones and registers with the vault's id, its default method,
    the origin, and no `pubkey`
  - [x] `Cloned` reports whether this device holds an identity, for both
    answers
  - [x] a failed clone into an absent path leaves no directory
  - [x] a failed clone into an existing empty directory leaves it, empty
  - [x] a non-empty target is refused and left untouched
  - [x] an unknown `format_version` rolls the clone back and registers
    nothing
  - [x] a registered name is refused before anything is cloned, as
    `AlreadyRegistered` (decision 5)
  - [x] `#[cfg(unix)]`: a registry save that fails leaves the clone in
    place, unregistered, as today (decision 6)
  - [x] `RollbackFailed`'s message is today's wording and its kind
    `Internal`, built directly: a failing directory removal can't be
    produced honestly in a test (decision 3)
- [x] Implement.

### 3. Move `gage init` and `gage clone` onto them

- [x] The CLI tests above, unedited, are the tests. Run them before
  changing anything, to confirm they're green.
- [x] Before the move, tighten `clone_refuses_an_already_registered_name`
  to today's whole message, green against today's code (decision 5).
- [x] File the registration race (decision 4) and the unrolled-back
  registry save (decision 6) as their own issues.
- [x] ADR-0061, linked from the decisions index.
- [x] `init.rs` and `clone.rs` call `Vault::init` and `Vault::clone_remote`.
  They no longer call `identityfile::create`, `Vault::create`,
  `gitrepo::clone`, `Global::register` or `save_creating_dir` directly.
- [x] `make qa`.

## Definition of done

- Every box above is checked.
- `git diff --stat main -- tests/fixtures` is empty.
- `make lint`, `make test`, `make qa` are clean; CI is green on Linux,
  macOS and Windows.
