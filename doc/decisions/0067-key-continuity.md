# ADR-0067: A device refuses a key file that doesn't continue the key it last saw

- Status: Accepted
- Origin: Replaced key file and entries read as genuine ([#207](https://github.com/distillerylabs/gage-rs/issues/207))

## Context

[ADR-0065](0065-entry-authentication.md) tags every entry under a key only
recipients hold, but sealing a key takes only the public list. Someone with
push access could replace `.gage/vault-key.age` with a key of their own and
every entry with one tagged under it. Each entry then authenticated against
the file beside it. [ADR-0066](0066-mac-key-rotation.md)'s trust cache
noticed the change but only warned on a read, and a `--yes` write silenced
it. Refusing only when the key file changed alone would not help: the same
attacker can add a recipient and turn it into an ordinary membership change.

## Decision

**A key file must continue the key this device last saw.** The check is
ADR-0066's merge check applied to reads: the recorded key must be the file's
current key or one of its retired keys. Someone holding only the public
keys can't produce a file that passes, and rotation keeps every retired key,
so a legitimate history always does, however many rotations a device sees
at once.

**What's recorded.** `HMAC-SHA256(key, "gage-vault-key-fingerprint-v3\n")`
of the current key, in `$GAGE_STATE/<vault-id>/key-continuity.toml`. The key
is 32 random bytes, so the fingerprint is one-way and the key itself is
never stored. It is a file of its own, not a field in `trust.toml`: reads
advance it without the vault's lock, and writes rewrite `trust.toml` whole
under that lock. It is separate from "last confirmed" for the same reason
the two move differently: a human confirms a recipient list, and the
continuity check moves this record by itself.

**Where it runs.** In `Vault::vault_key`, the one place that opens the key
for a read, a session, `history` or a write. A refusal is
`vaultkey::Error::Discontinuous`, `ErrorKind::Conflict`, and `--yes` has no
effect on it. A record that can't be read or understood also refuses:
treating it as absent would be a first use.

**The record moves only to a published key.** That is, when the bytes opened
are the file at the remote-tracking ref, or at HEAD for a vault with no
remote. A reader during a rotation, or after a crash in one, sees a key the
next write's reset discards, and two devices can rotate from one key with
one losing the race. A record that moved to either would refuse the vault
permanently. A key still local passes as a continuation of the recorded one,
and failing to save a record doesn't fail a read, since the key has already
passed.

**First use is trusted.** `init` records the key it minted. Any other device
adopts the key on its first read.

**There is no override.** A rollback across a rotation a device has seen is
refused, and it looks exactly like an attack. Restoring the good file
through git reads again with nothing else to do, since a refusal never moves
the record. To trust a vault afresh on purpose, `vault remove` deletes the
record and a new clone is a first use. The refusal's remedy doesn't say so,
because that is exactly the step an attacker wants: it says to ask another
member, and the docs describe the rest.

**v3 is amended rather than bumped.** A new refusal and a new local file are
format changes under [doc/format/README.md](../format/README.md). ADR-0066
amended v3 as a one-off until v3 shipped, and v3 still hasn't: it is in no
release tag. A `format_version` 4 would need a golden vault identical to
v3's, since nothing in a vault changes. This amends §2.9 and §3.4 of
[v3.md](../format/v3.md), and the rule applies in full once v3 ships. The
frozen fixture and the `format_v3_*` tests are unedited.

## Consequences

- **A device removed and re-added needs nothing.** While removed it can't
  open the key and reads fail as not-a-recipient, so the record stays. The
  file it later reads carries that key among the retired ones.
- **A refused device can't repair the vault through gage,** because writes
  and `gage sync`'s merge use the same check. The way out is restoring the
  file through git, from a clone or another device.
- **Not covered.** `pull` has no identity, so a forged key still reaches the
  local clone, where it is refused. A removed member who holds a retired key
  can still fool a device that hasn't seen the rotation that removed them:
  the window ADR-0065 and ADR-0066 already leave open. First use is
  trust-on-first-use. The check runs when an entry decrypts, so an attacker
  who deletes every entry gets an empty listing, not a refusal: that is
  deletion, a limit of "authentication is not freshness" in the [security
  model](../design/security-model.md).
