# ADR-0013: `vault remove` compares public keys and asks

- Status: Accepted
- Origin: Q-ORPHAN-BY-NAME, A20 (original project's register)

## Context

Whether to delete this device's wrapped private key on `vault remove` was
decided by comparing device **names**. A name is a label the vault happens to
store, not proof of which key it refers to. The comparison was wrong both
ways: a relabelled recipient (for example after `recipient approve
--device`) made an active key look orphaned, and the delete is
unrecoverable. Comparing public keys needs an unlock, and prompting for a
passphrase to decide a cleanup side effect is backwards.

## Decision

1. Record this device's `pubkey` in the local per-vault config when the
   identity is created ([ADR-0010](0010-device-name.md)). It is public and
   already in the vault's committed config. The comparison becomes a key
   comparison, with no unlock.
2. Never delete without a `Confirm` that defaults to keep. A stale record
   (a hand-swapped file) would otherwise be confidently wrong; a prompt
   turns a wrong answer into a suggestion a human reads. A scripted
   `vault remove` therefore keeps the file.
3. When the public key is unknown (missing field, hand-edited config),
   say so and keep.
