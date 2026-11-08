[index](index.md) · [M0](m0-library-mac-and-v3.md) · **M1** · [M2](m2-frontend-and-docs.md) · [M3](m3-qa-coverage.md)

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

## Decisions (resolved)

Recorded in [ADR-0066](../../../../decisions/0066-mac-key-rotation.md).

1. **Rotation ordering and crash-safety.** Every entry is read and the new
   key minted in memory first. Then, in order: entries are rewritten under
   the new key, `vault-key.age` is written, the recipient files are written,
   and one commit takes all of it. Nothing is committed until all of it is
   on disk, so HEAD never holds an entry tagged under a key its own
   `vault-key.age` doesn't carry. A crash leaves a dirty tree, which the
   next write's reset discards. Reads of that tree refuse until then.
   **Any change that drops a key rotates**, decided where the list is
   written: `remove`, `recovery enroll` and `recovery rotate` alike.
2. **The trust check covers `vault-key.age`.** The cache records the key
   file's hash, and a change to it alone warns as its own case. A normal
   add or remove regenerates the cache as before.
3. **Enrollment delivery.** Approval runs through `add_recipient_locked`,
   which re-seals. Nothing to add; a test pins it.
4. **Old revisions after a rotation** (not in the original plan; ADR-0065
   required it). The key file keeps retired keys behind the current one.
   Live entries verify against the current key only; `history` also accepts
   retired keys.
5. **A merge that straddles a key change** (not in the original plan). When
   one side replaced the key file and the other changed entries, the
   automatic merge stops, and `gage sync` re-stamps the other side's entries
   under the merged key and list, in the merge commit.
6. **Amending v3.** v3 has not shipped, so ADR-0066 amends it in this stack
   rather than bumping. The frozen fixture and `format_v3_*` tests are
   unedited.

## Tasks (tests first)

- [x] **Rotate on remove.** Test: after `recipient remove --reencrypt`, the
      removed key can no longer produce an entry that verifies (the new MAC
      key is unknown to it), and current recipients still read every entry.
- [x] **Re-seal on add.** Test: after `recipient add`, the newly added
      recipient can open `vault-key.age` and verify every entry.
- [x] **keep-both re-MACs.** Test: resolving a conflict with "keep both"
      produces a new entry whose `auth` verifies.
- [x] **Merge preserves `auth`.** Test: an automatic merge that takes a
      remote entry leaves its `auth` intact and verifiable.
- [x] **Enrollment.** Test: a device approved by enrollment can open the MAC
      key and verify entries.
- [x] **Recovery.** Test: the recovery key opens the MAC key and verifies
      entries.
- [x] **Trust check.** Test: an out-of-band change to `vault-key.age` is
      surfaced by the recipient-change check, not silently accepted.
- [x] **History survives rotation.** Test: every revision from before a
      rotation still reads through `history`, and a retired key is accepted
      nowhere else.
- [x] **Crash during rotation.** Test: an interrupted rotation leaves HEAD
      intact, and the next write's reset restores a vault that verifies.
- [x] **Straddling merge.** Tests: the automatic merge stops; `gage sync`
      re-stamps a new entry in both directions; a kept stale-side version of
      a real conflict verifies; a remote key that doesn't continue this
      device's is refused.

## Definition of done

- All tasks green; `make lint` and `make test` clean on all three platforms.
- A removed member cannot produce a valid entry after rotation; every
  legitimate holder (added recipient, enrolled device, recovery key) can
  verify.
- Every entry-producing path stamps a verifying `auth`.
- Stop for review. Do not start M2.
