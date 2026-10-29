# ADR-0053: Vault type is closed, unlock method is open

- Status: Accepted

## Context

Vault type and unlock method were strings, checked against allowlists
(`ALLOWED_TYPES`, `ALLOWED_METHODS`) where a command needed them. Reading a
config never refused either value. A vault naming an unknown type parsed,
and then silently lost git behavior: `vault info` showed no git block and
the shell prompt never showed it dirty.

The two values change over time in different ways. Adding a vault type
means a new store, and a build without it can't operate on that vault at
all. Adding an unlock method is per device
([ADR-0011](0011-method-scope.md)). A vault whose `[method].default` names
a method this build lacks is still usable by every device that unlocks
some other way.

## Decision

Both are enums in `vaultconfig`, and they read differently.

- **`VaultType` is closed.** It has one variant, `Git`, and serializes as
  `"git"`. Any other `type`, in the committed config or the global config,
  fails the parse and the error names the value. In `.gage/config.toml` the
  key is required. In a global-config row, an absent key reads as the
  default, `Git`.
- **`UnlockMethod` is open.** It is `Passphrase | Other(String)`, and its
  serde goes through `String`, so any value round-trips byte for byte. An
  `Other` is refused only where a method is used:
  - unlock: `UnlockError::UnsupportedMethod`
  - `CreateSpec` validation: `CreateError::UnknownMethod`
  - sealing an enrollment request: `EnrollmentError::UnknownMethod`
  - opening one: `MalformedRequest`
  - `init --method` and `identity add --method`: `Usage`, with the same
    message as before

  An absent method reads as `Other("")`, the empty string it always was,
  never as a passphrase nobody chose.
- **The lists come from the enums.** `VaultType::ALL` and
  `UnlockMethod::SUPPORTED` replace the allowlists. Help text and refusal
  messages are built from them, so the help snapshots are unchanged.
- **An unknown `--type` is refused where it is parsed.** `init` parses the
  flag into a `VaultType` (`UnknownVaultType`, `Usage`, same message as
  before). `CreateSpec` holds a `VaultType`, so `Vault::create` no longer
  checks the type, and `CreateError::UnknownType` is gone.

## Consequences

- Refusing an unknown `vault.type` at read is a reader-rule clarification,
  not a format change. v2 §2.2 already said `"git"`, the only type. Every
  vault gage wrote says `git`. No `format_version` bump and no new fixture.
  v2 §2.2's reader checks now list it.
- A future method is additive: a new variant, and older builds keep
  opening the vault. A future type also takes a new variant, and older
  builds refuse that vault's config with an error naming the type. That is
  the right failure for a vault they can't operate on.
