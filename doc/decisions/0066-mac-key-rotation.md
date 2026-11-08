# ADR-0066: The MAC key rotates when a key is dropped; retired keys stay for history; a merge straddling a key change is re-stamped

- Status: Accepted
- Origin: Entry authentication ([#198](https://github.com/distillerylabs/gage-rs/issues/198), M1 [#200](https://github.com/distillerylabs/gage-rs/issues/200))

## Context

[ADR-0065](0065-entry-authentication.md) gave each vault a MAC key and left
rotation to this milestone. Rotating has consequences that key alone
doesn't settle. Git keeps every old revision of an entry, tagged under the
key of its day, and `history` has to keep reading them. A device that
wrote an entry offline during a rotation merges that entry back tagged
under a key that is no longer current. And anyone can seal a key to the
public recipient list, so the key file itself proves nothing about who
wrote it.

## Decision

**Any change that drops a key rotates.** `recipient remove`, `recovery
enroll` and `recovery rotate` go through one write, and it compares the old
list with the new one: a key leaving means a new MAC key. A list that only
grows re-seals the existing key. Nobody has to remember to ask for it.

**The order.** Read every entry and mint the key in memory, then write the
entries under it, then `.gage/vault-key.age`, then the recipient files, then
make one commit. HEAD therefore never carries an entry tagged under a key
its own key file lacks. A crash leaves a dirty tree that the next write's
reset discards. Reads of that tree refuse until then, which is the honest
answer for a half-written rotation.

**Retired keys stay in the key file.** Its plaintext is the current key
followed by the keys it replaced, newest first, 32 bytes each. A live entry
verifies against the current key and nothing else, so a removed member's
old key produces nothing a reader accepts. Only `history` also accepts a
retired key, for revisions written when that key was current.

**The trust cache records the key file.** `trust.toml` gains
`vault_key_hash`. A change to the key file alone warns as its own case.
Since anyone can seal a key to the public list, this check is what stands
between a replaced key file and a vault of forged entries. A record without
the field adopts the current file on first use, as a device with no cache
does.

**A merge straddling a key change is re-stamped.** When exactly one side
changed the key file and the other changed entries, those entries carry a
stale tag and were encrypted to a stale list. The automatic merge stops
and reports them, since it has no identity to fix them with. `gage sync`
unlocks, verifies each one under its own side's key, and re-seals it under
the merged key and list in the merge commit. Whichever version of a real
conflict is kept is re-sealed the same way.

Before trusting the other side's key for this, `gage sync` requires one key
to continue the other: one side's current key must be the other's current
or retired key. Someone holding only the public keys can't produce a file
that passes.

**v3 is amended rather than bumped.** By the rule in
[doc/format/README.md](../format/README.md) these are format changes. v3
exists only in this unmerged stack and nobody holds a v3 vault, so a
`format_version` 4 would describe a version nobody used. This is a one-off:
v3.md §2.8, §2.9 and §3.4 are amended here, and once v3 ships the rule
applies in full. The frozen fixture and the `format_v3_*` tests are
unedited, and they still pass, since a 32-byte key file is a key file with
nothing retired and the new trust field is optional.

## Consequences

- **An entry written during a rotation needs `gage sync`.** A background
  push reports a divergence instead of merging. This also applies to a
  re-seal, since without decrypting a re-seal and a rotation look the same.
  It closes an older gap too: an entry written during a `recipient add` now
  reaches the new recipient.
- **A device removed by a rotation can't complete that merge.** It can't
  open the new key, so its `gage sync` fails, typed, with nothing applied.
- **The key file grows** by 32 bytes per rotation.
- **Not covered.** A removed member who can still push can forge a *past*
  revision under a retired key, visible only through `history`, never as a
  live entry. If such a member's push reaches a merge before the rotation
  does, the re-stamp treats it like any other write from before the
  rotation: the window [ADR-0065](0065-entry-authentication.md) already
  leaves open. A list a straddling merge brings in is encrypted to before
  this device has reviewed it, because the entries must reach that list in
  the merge commit; the review warning follows the merge, and the
  continuity check limits who can cause this to holders of the vault's key.
