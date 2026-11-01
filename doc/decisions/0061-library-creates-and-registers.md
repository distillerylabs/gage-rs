# ADR-0061: Creating or joining a vault is one library operation, registration included

- Status: Accepted
- Origin: A1a, library create/join operations ([#179](https://github.com/distillerylabs/gage-rs/issues/179))

## Context

`gage init` and `gage clone` were orchestrated in `gage-cli`. `init`
created this device's identity, called `Vault::create`, removed the
identity again if that failed, and registered the vault in global config.
`clone` cloned, emptied or removed its target directory if that failed,
checked the vault's `format_version`, and registered it. The library
offered the pieces and never wrote global config.

A second frontend (the iOS app, through `gage-ffi`) has to create and join
vaults too. Copying that orchestration into it would copy the part that
must never go wrong: the order that keeps a private key from being
orphaned, and a user's directory from being deleted.

## Decision

**`Vault::init` and `Vault::clone_remote` own the whole operation**
(`vault::lifecycle`): validation, the registry check, the identity or the
clone, the rollback of either, and the registration. A frontend calls one
of them and does not call `identityfile::create`, `Vault::create`,
`gitrepo::clone`, `Global::register` or `config::save_creating_dir` to
create or join a vault.

**`Vault::init` mints the vault id itself.** No caller passes one, so the
identity is always filed under the id the vault carries
([ADR-0006](0006-vault-id-keying.md)), and the identity file is always
new, so its rollback can never delete a key that existed before the call.

**The frontend keeps what is about its own surface:**

- the device-name default (the CLI reads the hostname)
- the target path (the CLI defaults to `$GAGE_DATA/vaults/<name>`)
- a clone's name inferred from its URL
- the recovery key: the frontend generates it, passes its public key in,
  and delivers the private half
- publishing, and the token prompt before it
- the enroll offer after a clone

**Failures are typed.** A rollback that itself fails is `RollbackFailed`,
carrying both errors. A registered name is `RegistryError::AlreadyRegistered`,
and a frontend adds its own hint (the CLI's `--name`). A registry save that
fails after the vault exists is its own variant, and is not rolled back.

This narrows the old rule that the library never writes global config: it
writes the registration a create or join makes. Other registration edits
(`identity add` recording a key, `vault remove`) stay in the frontend
until a second frontend needs them.

## Consequences

- No CLI output, exit code or prompt order changed.
- Neither operation locks global config while it registers. Two processes
  can lose a registration, and a failed save is not rolled back. Both are
  tracked as their own issues; this ADR moved the code without changing
  either behavior.
