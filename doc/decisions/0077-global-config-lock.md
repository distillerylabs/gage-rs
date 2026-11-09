# ADR-0077: The global config is rewritten under its own lock

- Status: Accepted
- Origin: found while planning A1a ([#183](https://github.com/distillerylabs/gage-rs/issues/183))

## Context

`$GAGE_CONFIG/config.toml` is one file listing every vault on the device.
Commands changed it by loading it, changing their copy, and saving the whole
file. The per-vault lock doesn't cover this, since the file isn't any one
vault's.

`init` loaded it, asked for a passphrase twice, then saved. `clone` loaded
it, cloned over the network, then saved. A vault registered by another
process in that time was overwritten. For `init`, the lost registration was
the only record pointing at that vault's identity.

## Decision

Every change to the global config goes through `config::update`. It takes an
advisory lock at `$GAGE_STATE/global-config.lock`, reads the file,
applies the change, saves, and releases. The lock is the same primitive as
the vault lock ([ADR-0003](0003-file-lock.md)), with the same 10-second
timeout. Reads don't take it: a save is atomic, so a reader sees one version
or the other.

The slow step stays outside the lock. `init` and `clone` still read the
registry first, to refuse a taken name before asking for anything, and read
it again under the lock to register. A name taken in between is refused with
`AlreadyRegistered`.

The closure given to `update` takes no other lock, and so this lock is always
the last one taken. It can't join a cycle with the vault locks
([ADR-0047](0047-two-vault-lock-order.md)).

This is not a format change, and `format_version` doesn't move. That version
describes a vault, and the lock file is a machine-local rendezvous that no
vault's contents depend on. No artifact in the [v3 spec](../format/v3.md)
changes, and the spec isn't edited.

## Consequences

- The process that loses a race for a name now gets an error, and its vault
  or clone is rolled back, as after a failed save
  ([ADR-0078](0078-failed-registration-rollback.md)).
- An unwritable `$GAGE_STATE` now fails any command that rewrites the global
  config, as it already fails every write to a vault.
- A build older than this one doesn't take the lock, so a mix of old and new
  processes can still race.
