# ADR-0052: Vault ids are a validated type

- Status: Accepted

## Context

A vault id becomes a local path component
([ADR-0006](0006-vault-id-keying.md)). Until D3a it was a `String`, and each
path builder in `paths` re-checked it before building a path. That made the
rule "no path is built from an unvalidated id" depend on every builder
remembering to check. An empty id also travelled as a normal value until a
builder refused it.

The global config's copy, `[vaults.<name>].id`, was never validated as it
was read. A bad value failed later, at whichever builder it reached first.

## Decision

A vault id is a `vaultconfig::VaultId`. It holds the canonical string: the
lowercase hyphenated form of a non-nil UUID, as `vaultconfig::valid_id`
defines it.

- **Construction is validation.** There are three ways to get one:
  `VaultId::new` mints a UUIDv4, and `TryFrom<&str>` and `TryFrom<String>`
  validate. Deserialization goes through `TryFrom<String>`
  (`#[serde(try_from = "String", into = "String")]`). No constructor skips
  the check, and there is no `Default`.
- **Path builders take `&VaultId`.** `paths::{identities_dir,
  identity_file_path, lock_file_path, trust_cache_dir}` take a `VaultId`.
  They no longer check the id or return an error for it.
  `paths::Error::InvalidVaultId` and `check_vault_id` are gone. A
  `compile_fail` doctest in `paths` proves a string can't be passed.
- **Where an id enters, it is checked once:**
  - The committed `.gage/config.toml`: `vaultconfig::read` still checks
    `format_version` first (v2 §2.2), then the id. It reads both in a
    first pass over plain values, so a bad id is still reported as
    `vaultconfig::Error::InvalidVaultId`, naming the id.
  - The global `config.toml`: `[vaults.<name>].id` is an
    `Option<VaultId>`. `id = ""` and an absent key read as `None`. Any
    other invalid value refuses the whole file as it is read, and the
    parse error names the file and the line.
  - The enrollment payload: `vault_id` stays a string on the wire, so a
    bad one is refused with its own reason, like the payload's other
    fields. `validate_payload` turns it into a `VaultId` before comparing
    it with the vault's.
- **A `Vault` always has an id.** `Vault::from_registration` refuses a
  registration that records no id when the vault's own config can't be
  read for one: `ResolveError::NoVaultId`, exit code 5 (`LockedOrAuth`).
  That is the code the empty id used to reach at a path builder.
- **`Ord` is the string's order.** The type derives `Ord` over the
  canonical string. Two vaults are locked in vault-id order
  ([ADR-0047](0047-two-vault-lock-order.md)), and the type must not move
  that order. For the canonical form, the string's order is also the
  UUID's numeric order.

Enrollment request ids keep using `valid_id` and get no type. They are
compared, not used to build a path.

## Consequences

- Refusing a bad id in the global config is a deliberate tightening. Only
  gage writes that file, so a bad id there is local corruption. A human
  can still see which key to fix, but `vault remove` can't forget the
  registration until the file parses.
- The errors only a bad id could produce are gone: `LockError::Path`,
  `TrustError::Path` and `ListIdentitiesError::Path`. They had no other
  source once the builders stopped failing on an id.
- Tests that built an invalid id now write it as text: a `VaultId` can't
  hold one.
- The format does not change. v2 §3.2 now states the reader rule. Any file
  gage ever wrote reads as it did, apart from the global-config tightening
  above.
