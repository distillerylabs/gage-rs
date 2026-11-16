# ADR-0085: A vault whose working tree is gone is refused at unlock, not at resolve

- Status: Accepted
- Origin: [#250](https://github.com/distillerylabs/gage-rs/issues/250), found while fixing [#188](https://github.com/distillerylabs/gage-rs/issues/188) ([#249](https://github.com/distillerylabs/gage-rs/pull/249))

## Context

A registration outlives the vault it names. The clone can be deleted by
hand, half-removed, or, on iOS before [ADR-0084](0084-bindings-place-vaults-by-name.md),
left behind in a moved container. Nothing reported it.
`Vault::from_registration` reads an unreadable vault config as the case the
registration's id covers, `unlock` finds the identity under the roots by
vault id, and listing read the missing `entries/` as "no entries yet". The
vault opened, unlocked and listed as empty. To a person that looks like data
loss, and nothing points at the fix.

Resolving can't refuse it: `vault remove` and `remove_vault`
([ADR-0082](0082-removing-a-vault-from-a-device.md)) resolve a vault
precisely so that a deleted clone can be forgotten.

## Decision

**A vault whose `.gage/config.toml` is gone is missing.** Every vault has
that file from its first commit, so its absence covers a deleted directory
and a half-removed one. A config that is there but unreadable is not
missing; whatever reads it reports that.

- **`Vault::unlock` refuses a missing vault** with
  `UnlockError::VaultMissing`, after the device's registration is read and
  before any passphrase is asked for. Unlock is the one point both
  invocation modes pass through before a read, so `ls`, `search`, `show`
  and the bindings' `Vault.open` → `list` all get it. A vault this device
  has no identity for still says that first.
- **`Vault::entry_ids` refuses a missing vault** (`entry::Error::VaultMissing`)
  rather than reading the missing `entries/` as empty. That covers a tree
  deleted while a session holds the key. A present vault without
  `entries/` still has no entries.
- **Both are `NotFound`, with the `ReRegister` remedy:** forget the
  registration and clone again.
- **`Vault::from_registration` does not check.** Removal keeps working on a
  missing clone.

## Consequences

- A read path added later inherits the refusal through `unlock` and
  `entry_ids`; it needs no check of its own.
- Unlock tests that are about the identity, not the vault, give their vault
  the config file the check looks for.
