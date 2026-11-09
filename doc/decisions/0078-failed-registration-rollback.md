# ADR-0078: A failed registration undoes the vault or clone

- Status: Accepted
- Origin: found while planning A1a ([#184](https://github.com/distillerylabs/gage-rs/issues/184))

## Context

Registering in the global config is the last step of `Vault::init` and
`Vault::clone_remote`. If it failed, or another process took the name
meanwhile ([ADR-0077](0077-global-config-lock.md)), the files stayed on disk
and nothing pointed at them. A retry under the same name was blocked by the
non-empty target, and an `init` retry minted a new id, orphaning the first
identity under the old one.

The alternative was to keep the files and let a retry adopt them. That needs
a way to tell an adoptable leftover from a directory that merely looks like
one, for a vault that holds nothing yet, and it would break the promise
[ADR-0061](0061-library-creates-and-registers.md) makes: a registered,
usable vault, or nothing of the call's own making.

## Decision

**A failed registration is rolled back like every earlier failure.**
`config::update` reports an error only when the save didn't land, so no
registration names the files being removed.

**`init` removes only a vault it can show is its own.** Its target was absent
or empty when `Vault::create` accepted it. The vault id is minted by the
call, so the rollback removes the target only if `.gage/config.toml` there
still carries that id. Anything else at the path is left alone. A vault this
new has one commit and no entries, so nothing written by a person is lost.

**The vault goes before the identity.** If the vault can't be removed, the
identity stays with it, and the failure says what is left. The trust cache
entry `Vault::create` recorded is removed best-effort.

**A target directory that existed stays.** One this call made is removed; an
empty one it was given is emptied, as a failed clone already does.

A rollback that itself fails is `InitError::RegistrationRollbackFailed` or
`CloneError::RollbackFailed`, of kind `Internal`, carrying both errors.

## Consequences

- A failed registration can be retried under the same name with nothing to
  clean up. The prompts and the network clone have to be repeated.
- A clone has no minted id to check, since every clone of a vault carries the
  same one. It relies on git refusing a non-empty target, as the rollback of
  a failed clone already did.
