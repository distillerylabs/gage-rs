# ADR-0009: `recipient add` always re-encrypts

- Status: Accepted
- Origin: A19 (original project's register); archived CLAUDE.md invariant

## Context

Adding a recipient without re-encrypting yields a device that reads entries
written after its admission and not before. That contradicts "a vault is the
unit of trust" (the answer to "these people should see less" is a second
vault and `mv --to-vault`), cannot be diagnosed from the interface (`ls`
shows every entry; decryption fails on an arbitrary-looking subset), and
spreads: a partially admitted device cannot grant full access to anyone.

## Decision

`recipient add` (and `recipient approve`) re-encrypt every entry, with no
flag. `recipient remove` keeps a mandatory `--reencrypt`, which means
something different. An actor who cannot read every entry cannot add a
recipient: the command refuses up front with `CannotGrantFullAccess`,
naming a count, before the lock and before any confirmation.

## Consequences

Every `recipient add` rewrites every entry: a larger repo over time, and a
no-op-plaintext revision in each entry's history. Accepted; vaults are
small and the operation is rare. There is no partial-recipient state.
