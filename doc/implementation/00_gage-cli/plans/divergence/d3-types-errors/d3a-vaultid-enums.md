# D3a — `VaultId`, enums, `Option` config

[D3 plan](index.md) · [#132](https://github.com/distillerylabs/gage-rs/issues/132) · next: [D3b](d3b-error-boundary.md)

> **Recommended model: Opus.** A vault id becomes a filesystem path
> component (ADR-0006), so moving its validation from the path builders
> to a type is a security change. Every serde change risks a byte change
> in a committed or local file.

## Goal

Three things change in the library:

- A vault id is a `VaultId`. It is validated once, where it enters, and
  every path builder takes one.
- Vault type and unlock method are enums.
- Fields that use `""` for "absent" are `Option`.

What a file contains, and what an unchanged file reads as, does not
change. Byte-exact tests, written first, prove it. Two exceptions are
deliberate, and each has an ADR:

- an unknown `vault.type` is refused (ADR-0053)
- an invalid global-config vault id is refused (ADR-0052)

## Depends on

The parent branch `131-divergence-types-errors` (these docs).

## Design references

- [D3 index](index.md): resolved decisions and the `Option` rule.
- [ADR-0006](../../../../../decisions/0006-vault-id-keying.md): vault-id
  keying and validation in and out.
- [ADR-0047](../../../../../decisions/0047-two-vault-lock-order.md): two
  vaults are locked in vault-id order. `VaultId`'s `Ord` must be that
  order.
- [ADR-0011](../../../../../decisions/0011-method-scope.md) and
  [ADR-0012](../../../../../decisions/0012-method-flag.md): method scope and
  the `--method` flag.
- [v2 §2.2, §2.7, §3.2, §3.4](../../../../../format/v2.md): the vault config,
  the enrollment request, the global config and the trust cache.
- `crates/gage/src/testsupport/format_v2.rs` (`install`, the fixture
  constants) and `crates/gage/src/format_v2_tests.rs`.

## Where things are

- **Validator.**
  - `vaultconfig/id.rs`: `valid_id`, which accepts only the canonical
    lowercase hyphenated non-nil UUID, and `new_id`.
  - `paths.rs:126`: `check_vault_id` and `paths::Error::InvalidVaultId`.
- **Enrollment request ids reuse `valid_id`**
  (`enrollment/seal.rs:202`, `enrollment/name.rs:35`). They are not
  vault ids.
- **Vault-id fields:**
  - `VaultMeta.id` (`vaultconfig/mod.rs:44`)
  - `VaultEntry.id` (`config.rs:42`, never validated today)
  - `Payload.vault_id` (`enrollment/seal.rs:32`)
  - `Vault.id` (`vault.rs:67`)
  - `CreateSpec.id`, `Info.id`, `ResolveError::Mismatch`
- **Builders:**
  - `paths::{identities_dir, identity_file_path, lock_file_path,
    trust_cache_dir}`
  - `identityfile::{create, remove, backup}`
  - `trustcache::remove`
  - `vaultlock` takes a `&Path`. Its path comes from
    `Vault::lock_file_path`, so it needs no change.
- **`""` checks on the id:**
  - `orphan.rs:59`
  - `gage-cli/src/commands/vault.rs:83`
  - `vault.rs:413` ("none recorded")
  - `entry/crud.rs:1451`, a test that relies on `vault.id =
    String::new()`
- **String-constant enums.** `vault.rs:33-44` defines them. They are
  used in:
  - `vault.rs:565`, `vault.rs:603`, `vault.rs:886-890`
  - `enrollment/seal.rs:96`
  - `gage-cli/src/shell.rs:222`
  - `gage-cli/src/commands/identity.rs:68`
  - the `init.rs:61-92` flags, whose help text is in
    `tests/fixtures/help/init.txt` and `identity_add.txt`

## Decisions to make first

**Settled** (reviewed 2026-10-28). Each item below is now the decision,
not a proposal, with these refinements:

- Decision 1: `vaultconfig::read` parses in two passes, so the
  `format_version` check still runs before the id check (v2 §2.2's order),
  and a bad id still lands in `vaultconfig::Error::InvalidVaultId`.
- Decision 4: the refusal is a new `ResolveError::NoVaultId { name, path }`,
  mapped to `LockedOrAuth`, so the exit code stays 5. (`path` was added
  to the approved `{ name }` so the message can say which vault can't be
  read.) The `entry/crud.rs` test became
  `a_registration_without_an_id_resolves_to_no_vault_before_any_path_is_built`,
  and the CLI half is in
  `lifecycle_tests::a_vault_without_an_id_fails_loudly_on_every_derived_path`.
- **The `format_v2_*` tests get type-only edits.** The plan's "unedited"
  can't hold: `Method.plugin`, `TrustRecord.mismatch_seen` and
  `Global.current` become `Option`, and the path builders take a
  `&VaultId`, so the old spellings stop compiling. Edits there are
  limited to spelling the same value in the new type (`None`,
  `Some(..)`, a `VaultId` built from `fx::VAULT_ID`). No assertion
  changes what it checks. The list is under task 1 for review.

1. **Where `VaultId` lives.** Proposed: `vaultconfig/id.rs`, next to
   `new_id`, re-exported as `vaultconfig::VaultId`. D4a regroups modules,
   and D3a doesn't pre-empt that.

   It holds the canonical `String`, so `Ord` is today's string order and
   the two-vault lock order (ADR-0047) doesn't move. Its API:
   - `TryFrom<&str>` and `TryFrom<String>` → `InvalidVaultId`
   - `#[serde(try_from = "String", into = "String")]`
   - `as_str`, `Display`, `new()` to mint one

   Request ids stay on `valid_id`, renamed if it reads better, and get
   no type here.
2. **The empty-string read helper.** Proposed: one
   `deserialize_with` fn, `serde_opt::empty_as_none`, applied to every
   field the `Option` rule converts.
   - `key = ""` reads as `None`.
   - Writing is `skip_serializing_if = "Option::is_none"`.

   For `Option<VaultId>` the helper must try the conversion after the
   empty check, so `id = ""` is `None` but `id = "x"` is an error.
3. **The unknown-method variant's shape.** Proposed:
   `UnlockMethod::Passphrase | UnlockMethod::Other(String)`.
   - Serde uses `from = "String"` / `into = "String"`, so the round trip
     is byte-exact for any value.
   - The CLI's `--method` flag rejects `Other` up front, with `Usage` and
     the same message as today.
   - `CreateSpec` and `seal` validation reject `Other` exactly where they
     reject an unknown string today.
4. **`Vault.id` for a registration without one.** `from_registration`
   (`vault.rs:523`) can fall back to the registration's id, which may be
   absent. Proposed: `from_registration` returns an error when neither
   the committed config nor the registration yields an id, so `Vault.id`
   is a plain `VaultId`.

   The test at `entry/crud.rs:1451` that builds `vault.id = ""` to reach
   `WriteError::Path` becomes a test that this refusal fires. Record here
   which error variant it lands in, and confirm its exit code stays 5.

## Tasks

### 1. Byte-exact serialization tests, green on today's code

Before any type changes. These are new tests; the existing `format_v2_*`
tests are not edited.

Written in a new file, `crates/gage/src/format_v2_bytes_tests.rs`, so the
type-only edits to `format_v2_tests.rs` stay a separate, small diff.
`trustcache::store_trust_cache` became `pub(crate)` so the trust test goes
through the real store path.

- [x] `format_v2_vault_config_reserializes_byte_identically`: read the
      fixture's committed `.gage/config.toml` (and `known-config.toml`)
      with `vaultconfig::read`, then serialize it the way `vaultconfig::write`
      does. The bytes equal the file.
- [x] `format_v2_global_config_reserializes_byte_identically` (from the
      template with plain paths: a Windows temp path's backslashes would
      let the serializer pick another string style): the same
      for the installed `config.toml`, through `config::read` and the
      `config::write` serializer.
- [x] `format_v2_trust_record_reserializes_byte_identically`: the same for
      `trust.toml`.
- [x] `format_v2_enrollment_payload_reserializes_byte_identically`: open the
      pending request at the fixture's clock, serialize the `Payload`, and
      compare it with the decrypted plaintext.
- [x] Unit tests for the empty-vs-absent spellings this milestone must
      keep (`format_v2_empty_optional_keys_are_omitted`,
      `format_v2_empty_required_keys_are_written`, spelled as TOML text so
      they outlive the type change):
  - an empty `Method.plugin`, `mismatch_seen`, `current`, `pubkey`, git
    `origin` and shell keys are omitted
  - a required empty key, e.g. `device = ""` on a `VaultEntry`, is still
    written as `key = ""`
- [x] All green on `main`'s code. If one is red, stop: see the index's
      process notes.

**Type-only edits to the `format_v2_*` tests**, for review. Each spells
the same value in the new type; no assertion checks anything different.

- `crates/gage/src/format_v2_tests.rs`:
  - `paths::identity_file_path(&i.dirs, fx::VAULT_ID, ..)` →
    `&fx::vault_id()` (twice), and `trust_cache_dir(.., fx::VAULT_ID).unwrap()`
    → `trust_cache_dir(.., &fx::vault_id())` (twice). `fx::vault_id()` is
    new in `testsupport/format_v2.rs`.
  - `Method { plugin: String::new() }` → `plugin: None`
  - `record.mismatch_seen == ""` → `None`
  - `global.current == fx::VAULT_NAME` → `Some(fx::VAULT_NAME.into())`
  - in `CreateSpec`: `vaultconfig::new_id()` → `VaultId::new()`,
    `vault_type: "git".into()` → `VaultType::Git`
- `crates/gage-cli/src/format_v2_tests.rs`:
  - the four `Shell` fields → `Some(..)`, and the same in their asserts
  - `global.current`, `entry.id`, `entry.pubkey` asserts → `Some(..)`
  - `entry.git.origin` → `.as_deref().unwrap()`
  - in the fixture writer (`#[ignore]`d): `registration.id` →
    `.unwrap()`, and `.as_str()` / `.as_deref().unwrap()` where a `&str`
    is needed

### 2. `VaultId`

- [x] Tests first, in `vaultconfig/id.rs`:
  - it accepts exactly what `valid_id` accepts
  - it rejects uppercase, braces, `urn:`, undashed, nil and `""`
  - serde refuses a bad id
  - `Ord` matches `str` order on a sample, as ADR-0047 requires
- [x] Implement it, per decision 1. `new_id()` is gone in favor of
      `VaultId::new()`. `AsRef<str>` and `AsRef<Path>` are outward-only
      conveniences; no constructor skips the check.
- [x] `paths` builders take `&VaultId`. `check_vault_id` and
      `paths::Error::InvalidVaultId` go. The test table is deleted and
      replaced by a `compile_fail` doctest in `paths`, with a twin that
      compiles with a real `VaultId`, so it fails for the right reason.
      The three builders that only checked the id now return `PathBuf`.
      `LockError::Path`, `TrustError::Path` and
      `ListIdentitiesError::Path` had no other source, so they are gone.
- [x] `identityfile::{create, remove, backup}`, `trustcache::remove`,
      `describe_vault_id`, `clone::access_lines` and the CLI callers take
      `&VaultId`.
- [x] `VaultMeta.id`, `Vault.id`, `CreateSpec.id` and `Info.id` are
      `VaultId`. `vaultconfig::read` checks the id in its first pass
      (decision 1), with `VaultId::try_from`, and the full parse checks it
      again by type. `vaultconfig::Error::InvalidVaultId` stays, and
      `format_v2_reader_rejects_what_the_spec_rejects` passes.
      `CreateError::InvalidId` and `EnrollmentError::NoVaultId` are gone:
      a `VaultId` can't be invalid.
  - **Deviation: `Payload.vault_id` stays a `String` on the wire.** As a
    `VaultId` field, a missing or bad id would fail the whole TOML parse
    as "not valid TOML" instead of "its sealed vault id … is not a
    uuid". `payload_tests` pins that reason, and `Payload`'s doc says
    why every field is validated after parsing. `validate_payload`
    converts it with `VaultId::try_from` before the wrong-vault
    comparison, so nothing calls `valid_id` on it.
- [x] `VaultEntry.id: Option<VaultId>`. Test that a global config with a
      bad id fails to load and names the key.
- [x] `orphan.rs:59` and `commands/vault.rs:83` match on `Option`.
      `ResolveError::Mismatch.registered_id` becomes `Option<VaultId>`,
      and the message for `None` stays "none recorded".
- [x] Decision 4's `from_registration` refusal, test first
      (`from_registration_refuses_when_no_id_is_known_at_all`).
- [x] Two-vault ordering (`entry/crud.rs:446-449`, `movecopy.rs:114`)
      compares `VaultId`s. Existing ordering tests stay green.

### 3. `VaultType` and `UnlockMethod`

Both live in `vaultconfig/kind.rs`, re-exported from `vaultconfig`.

- [x] Tests first:
  - `type = "git"` parses to `VaultType::Git`
  - `type = "s3"` is refused at read, and the error names the value
  - `default = "passphrase"` parses to `Passphrase`
  - `default = "yubikey"` parses to `Other("yubikey")` and
    re-serializes byte-identically
  - unlocking an `Other` method still gives
    `UnlockError::UnsupportedMethod` (`unlock_tests.rs:1029` stays
    green)
- [x] Implement both enums. `TYPE_GIT`, `METHOD_PASSPHRASE`,
      `ALLOWED_TYPES` and `ALLOWED_METHODS` go. Help text lists the
      values from the enums, and the help snapshots are unchanged.
  - `VaultMeta.vault_type` is required in `.gage/config.toml`. A
    global-config row's absent `type` reads as the default, `Git`.
  - An absent method reads as `UnlockMethod::Other("")`, the empty
    string it always was. Unlock's "records no device/method" check
    still fires for it.
  - An unknown `--type` is refused where `init` parses it
    (`UnknownVaultType`, same message). `CreateSpec` holds a
    `VaultType`, so `CreateError::UnknownType` is gone.
- [x] Typed fields:
  - `VaultMeta.vault_type`, `Method.default`
  - `VaultEntry.vault_type`, `VaultEntry.method`
  - `Payload.method`
  - `CreateSpec`, `Info`, `EnrollmentRequest`, `OpenedRequest`,
    `Recorded`
- [x] Callers become `match`es:
  - `vault.rs:565`: `Info.git` follows from the type
  - `vault.rs:603`: unlock
  - `shell.rs:222`: is the vault dirty
  - `identity.rs:68`: `--method`
- [x] `init --type`/`--method` and `identity add --method`: an unknown
      value is still `Usage` with today's message, pinned before the
      change by `init_unknown_type_and_method_messages_are_pinned` and the
      exact-stderr assert in
      `identity_add_method_defaults_to_the_vaults_and_stays_local`.

### 4. `Option` fields

- [x] Tests first: each converted field reads `key = ""` as `None`, reads
      an absent key as `None`, and omits `None` on write
      (`format_v2_empty_and_absent_optional_keys_read_as_none`, plus task
      1's omission test).
- [x] Convert the fields in the [`Option` rule](index.md#resolved-decisions).
      `TrustCache.mismatch_seen`, the in-memory twin of the record's, is
      `Option` too.
      Callers that tested `.is_empty()` match on the `Option`:
  - `session.rs:215`
  - `config.rs:160-189`
  - `auth.rs:57,60`
  - `shell.rs:59-118`
  - `orphan.rs:75`
  - `enroll.rs:87`
  - `trustcache.rs:370,522`
- [x] `EnrollmentRequest.id` (`enroll.rs:92`) becomes `Option<String>`.

### 5. ADRs

- [x] ADR-0052 *Vault ids are a validated type.* It covers:
  - the `VaultId` contract
  - validation at deserialize and construction, not at each path builder
  - the global-config tightening
  - how the type's `Ord` keeps ADR-0047's order
- [x] ADR-0053 *Vault type is closed, unlock method is open.* It says why
      the two differ: a future method must not make an older build refuse
      the vault config.
- [x] Link both from `doc/decisions/README.md`. Amend ADR-0006's
      "validated as a UUID before any path is built" to point at ADR-0052.
- [x] `doc/format/v2.md`: §2.2's reader checks gain "`vault.type` is
      `git`", and §3.2 says the reader refuses an invalid `….id`. This is
      a reader-rule clarification, not a format change: no
      `format_version` bump and no new fixture. ADR-0053 says so.

### 6. Green

- [x] `cargo check --workspace --all-targets` after each task.
- [x] `git diff --stat main -- tests/fixtures` is empty.
- [x] The help snapshots are green and unedited. The existing
      `format_v2_*` tests are green, with only the type-only edits listed
      under task 1.
- [ ] CI green on Linux, macOS and Windows.

## Definition of done

- Every box above is ticked, and decisions 1–4 are recorded here as
  settled.
- `make lint` and `make test` are clean.
- Nothing in either crate calls `valid_id` on a vault id or checks a vault
  id for `""`.
- CI is green on Linux, macOS and Windows.
