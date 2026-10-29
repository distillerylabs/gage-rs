# ADR-0006: Per-vault local state is keyed by an assigned vault id

- Status: Accepted
- Origin: A20, Q-IDENTITY-VAULT-NAME (original project's register)

## Context

Identity files were keyed by the vault's local registration name, which is
not unique. Two vaults registered under one name in sequence shared a
directory, and `vault remove` could then delete the only copy of a private
key that belonged to another vault.

## Decision

Every vault has an id: a UUIDv4 minted once by `init` and written to
`.gage/config.toml` as `[vault].id`. It travels with the vault, so every
clone agrees on it. It is assigned, not derived.

All per-vault local state is addressed by it, and nothing under
`$GAGE_DATA` or `$GAGE_STATE` is addressed by a local name:

- identities: `$GAGE_DATA/identities/<vault-id>/<device>.age`
- trust cache: `$GAGE_STATE/<vault-id>/known-config.toml`
- lock: `$GAGE_STATE/locks/<vault-id>.lock`

Global config records the id (and this device's public key,
[ADR-0013](0013-orphan-identity-by-pubkey.md)) under `[vaults.<name>]`, so
`vault remove` can find what a vault owns after its own files are gone. The
identities directory carries a plaintext, advisory marker naming the vault;
nothing reads it to decide anything.

The id is untrusted input: it arrives in a committed file any git writer can
edit and becomes a path component. It is validated as a UUID before any
path is built from it, and a config whose id does not parse is refused
rather than coerced. Since D3a the validation is a type: an id is checked
once, where it is read, and the path builders accept only a checked one
([ADR-0052](0052-vault-id-type.md)).

## Consequences

- A hash of the origin URL was rejected: a URL has many spellings, changes
  with `git set-remote`, and does not exist for a local-only vault.
- One repository registered twice under two names is a supported state; both
  registrations share one id, one lock and one identity directory.
- `mv` and `cp` refuse a source and destination with the same id up front
  ([ADR-0003](0003-file-lock.md): locking one vault twice would wait out the
  timeout), and lock two vaults in order of id, so opposite-order
  invocations cannot deadlock.
