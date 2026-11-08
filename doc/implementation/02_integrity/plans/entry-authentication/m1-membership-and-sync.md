[index](index.md) · [M0](m0-library-mac-and-v3.md) · **M1** · [M2](m2-frontend-and-docs.md)

# M1 — Membership & sync: rotation, re-encryption, keep-both, enrollment, recovery

> Recommended model: **Opus**. Key rotation ordering, crash-safety of the
> re-encryption, and making sure every path that produces an entry also
> stamps a valid `auth`.

## Goal

Every way an entry comes to exist produces an authenticated one, and the MAC
key reaches exactly the right holders: it is re-sealed to the recipients on
`recipient add`, rotated on `recipient remove --reencrypt`, delivered to a
device joining by enrollment, and readable with the recovery key. A removed
member can no longer produce a valid entry.

## Depends on

M0 (the MAC key, `auth` stamp/verify, format v3). Branch off the M0 branch.

## Design references

- `crates/gage/src/trust/recipient/mod.rs` — `add_recipient_locked`,
  `reencrypt_to` (the loop that reads and rewrites every entry).
- `crates/gage/src/trust/trustcache.rs` — the recipient-change trust check
  the `vault-key.age` change must be covered by.
- `crates/gage/src/sync/syncresolve.rs` — keep-both re-encrypts `theirs`
  (`encrypt_entry`), so it must re-stamp `auth`.
- `crates/gage/src/sync/gitrepo/merge.rs` — the merge moves existing entry
  blobs unchanged (their `auth` travels with them); confirm no path strips it.
- `crates/gage/src/enrollment/approve.rs` — `write_approvals` adds the joining
  device as a recipient; the MAC key must be re-sealed to include it.
- `crates/gage/src/recovery/swap.rs` — the recovery key is a recipient;
  confirm it receives the MAC key through the same sealing.

## Decisions to make first

1. **Rotation ordering and crash-safety.** On `remove --reencrypt`: mint a new
   MAC key, rewrite every entry's `auth` under it, write the new sealed
   `vault-key.age`, then drop the removed recipient — all in the existing
   atomic re-encryption commit. Specify the order so a crash never leaves
   entries authenticated under a key no current recipient holds.
2. **Does the trust check cover `vault-key.age`?** A change to the sealed key
   file is a recipient-set change; confirm the recipient-change warning fires
   when it changes unexpectedly, and that a normal `add`/`remove` regenerates
   the cache as today.
3. **Enrollment delivery.** Approving a device adds its pubkey as a recipient;
   re-sealing the MAC key to the new recipient set (the same step that
   re-encrypts entries, if approval does) delivers it. Confirm approval runs
   that re-seal, or add it.

## Tasks (tests first)

- [ ] **Rotate on remove.** Test: after `recipient remove --reencrypt`, the
      removed key can no longer produce an entry that verifies (the new MAC
      key is unknown to it), and current recipients still read every entry.
- [ ] **Re-seal on add.** Test: after `recipient add`, the newly added
      recipient can open `vault-key.age` and verify every entry.
- [ ] **keep-both re-MACs.** Test: resolving a conflict with "keep both"
      produces a new entry whose `auth` verifies.
- [ ] **Merge preserves `auth`.** Test: an automatic merge that takes a
      remote entry leaves its `auth` intact and verifiable.
- [ ] **Enrollment.** Test: a device approved by enrollment can open the MAC
      key and verify entries.
- [ ] **Recovery.** Test: the recovery key opens the MAC key and verifies
      entries.
- [ ] **Trust check.** Test: an out-of-band change to `vault-key.age` is
      surfaced by the recipient-change check, not silently accepted.

## Definition of done

- All tasks green; `make lint` and `make test` clean on all three platforms.
- A removed member cannot produce a valid entry after rotation; every
  legitimate holder (added recipient, enrolled device, recovery key) can
  verify.
- Every entry-producing path stamps a verifying `auth`.
- Stop for review. Do not start M2.
