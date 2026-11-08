# ADR-0065: Entries are authenticated by a per-vault MAC key; format v3; v2 is refused

- Status: Accepted
- Origin: Entry authentication ([#198](https://github.com/distillerylabs/gage-rs/issues/198), M0 [#199](https://github.com/distillerylabs/gage-rs/issues/199))

## Context

An entry is an age file encrypted to every key in `.age-recipients`. age
protects the ciphertext's integrity, but it doesn't say who wrote it:
producing an entry takes only the public keys, and those are committed in
the vault for anyone with read access to the repository. An entry's payload
was also not bound to its filename, so a genuine entry's file could be
copied over another's and read as that entry. The [security
model](../design/security-model.md)'s goal that only a vault member can
write an entry the client trusts was therefore not enforced.

## Decision

**A per-vault MAC key that only recipients hold.**

- **The key.** 32 random bytes, minted at `init` and stored as
  `.gage/vault-key.age`, an age file sealed to every key in
  `.age-recipients`, exactly as an entry is. Opening it takes a
  recipient's private key. Every re-encryption to a new recipient list
  writes it to that list too. Rotating it when a recipient is removed is
  [ADR-0066](0066-mac-key-rotation.md).
- **The tag.** Every entry payload is the entry's YAML body followed by
  one last line, `auth: hmac-sha256:<64 lowercase hex>`. The tag is
  `HMAC-SHA256(key, "gage-entry-auth-v3\n" ‖ id ‖ "\n" ‖ body)`, with `id`
  the filename's canonical UUID and `body` the exact bytes before the
  line. [v3.md](../format/v3.md) §2.6 specifies it.
- **Verification.** Every decrypt recomputes the tag before parsing
  anything and refuses a missing or wrong one (`entry::Error::Unauthenticated`).
  A vault without a readable key is a hard error (`Error::VaultKey`),
  never an unauthenticated read.
- **Format v3; v2 is refused.** The key file and the `auth` line are a
  format change. A v2 vault is refused at config load like v1, with
  advice saying it is an older format this build no longer reads. No
  migration or unauthenticated read path is kept. That is viable because
  gage is pre-release with no other users, and any fallback would be
  exactly the path a forger takes.

**Why exact bytes and not a re-serialization.** Re-serializing on read and
comparing would tie every existing tag to the emitter's exact output
forever, and would parse unauthenticated input. MACing the body's bytes as
stored keeps the emitter's style choices outside the format, and nothing is
parsed until it has authenticated.

**Why a shared MAC key and not signatures.** "A recipient wrote this"
matches gage's trust model: a vault is the unit of trust, and its
recipients trust each other. Per-device Ed25519 signatures would add a
second key type across enrollment, recovery and the mobile bindings for a
provenance guarantee nothing else in gage relies on.

## Consequences

- **Writing an entry takes an identity.** `Vault::write_entry` and
  `Vault::update` take the `Identity` that opens the key. `mv`/`cp`
  unlock the destination as well as the source. Writing into a vault this
  device isn't a recipient of is no longer possible, by design.
- **`init` refuses a recipient this build can't encrypt to** (a plugin
  key), since it seals the key to every recipient. Writing an entry
  already refused such a recipient.
- **`auth` is a reserved key.** An entry carrying it among its unknown
  fields is refused on write, and a body containing it is refused on read.
- **Old-format fixtures can be refused rather than read.** The format
  README now allows an old fixture's tests to become refusal tests when
  the bump's ADR refuses the old format. `format-v2/` stays, unedited, as
  the vault every command must refuse.
- **Not covered.** Per-device provenance: any current recipient can write
  any entry. A removed recipient keeps the key until it is rotated
  ([ADR-0066](0066-mac-key-rotation.md)), just as it keeps the history it
  could already read. Rollback and freshness of the vault as a whole are
  separate work.
