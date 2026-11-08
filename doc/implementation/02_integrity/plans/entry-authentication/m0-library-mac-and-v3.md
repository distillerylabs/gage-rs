[index](index.md) · **M0** · [M1](m1-membership-and-sync.md) · [M2](m2-frontend-and-docs.md)

# M0 — Library: MAC key, authenticated entries, format v3

> Recommended model: **Opus**. Security-sensitive crypto and a format
> version bump; the authentication construction and the v2-refusal ordering
> need careful reasoning.

## Goal

The library mints a per-vault MAC key, stamps an `auth` tag on every entry it
writes, and verifies it on every read. The on-disk format becomes v3; a v2
vault is refused. After M0, an entry produced without the MAC key fails to
read, and an entry's ciphertext is bound to its id.

## Depends on

Nothing. First milestone; cut off the parent branch `198-entry-authentication`.

## Design references

- `crates/gage/src/entry/store.rs` — `write_entry` / `write_entry_to` /
  `encrypt_entry` / `decrypt_entry` / `read_entry` (where stamp + verify hook
  in).
- `crates/gage/src/entry/mod.rs` — the `Entry` struct and its fields.
- `crates/gage/src/entry/yaml.rs` — `to_yaml` / `from_yaml` (the payload
  wire form the `auth` field joins).
- `crates/gage/src/crypto/crypt.rs` — `encrypt` / `decrypt`, `Recipient`;
  the model for sealing a secret to recipients.
- `crates/gage/src/storage/vaultconfig/mod.rs` — `CURRENT_FORMAT_VERSION`
  (=2) and the version-mismatch refusal.
- `doc/format/v2.md`, `tests/fixtures/format-v2/` — the format contract and
  the frozen golden vault.
- CLAUDE.md "Format stability".

## Decisions to make first (settle in this doc, get reviewed, before code)

1. **MAC input canonicalization.** `auth = HMAC-SHA256(mac_key, uuid ‖ C)`
   where `C` is a deterministic encoding of the entry's fields *excluding
   `auth`*. Proposal: compute the MAC over the v3 payload bytes with the
   `auth` field omitted (serialize, MAC, then insert `auth`), and on read,
   remove `auth`, re-serialize, and compare. Alternative: MAC a fixed-order
   concatenation of the typed fields. Pick one and specify it exactly in
   `v3.md` — the encoding is part of the format contract.
2. **Where the MAC key file lives and its shape.** Proposal:
   `.gage/vault-key.age`, an `age` file encrypting the raw 32-byte key to the
   same recipients as entries. Confirm the path and that it sits beside
   `.age-recipients` under `.gage/`.
3. **What a v2 vault does.** Refused at config load via the existing
   version-mismatch path. Confirm the message tells the user the vault is an
   older format this build does not read.
4. **MAC key absence on a v3 vault.** A v3 vault whose `vault-key.age` is
   missing or unreadable is a hard error (not a silent unauthenticated read).

## Tasks (tests first)

- [ ] **MAC key type + sealing.** Test: a generated key round-trips through
      seal→open with a recipient identity; a non-recipient cannot open it.
      Then: a small module (under `crypto`) that generates a 32-byte key,
      seals it to a recipient list (reusing `crypt::encrypt`), and opens it
      with an `Identity`.
- [ ] **Stamp on write.** Test: `write_entry` produces a payload whose `auth`
      verifies. Then: `write_entry_to` / `encrypt_entry` compute `auth` over
      `uuid ‖ fields` and include it. `encrypt_entry` takes the id so it can
      bind it.
- [ ] **Verify on read.** Test: a correct entry reads; an entry whose `auth`
      is wrong, missing, or computed for a different uuid is rejected with a
      new `Error::Unauthenticated`. Then: `decrypt_entry` / `read_entry`
      recompute and compare against the filename's uuid.
- [ ] **Non-recipient cannot produce a valid entry.** Test (ports the
      assessment's forgery check, neutrally): an entry encrypted to the public
      recipients *without* the MAC key is rejected on read; an entry file
      placed under a different id is rejected.
- [ ] **Format v3.** Bump `CURRENT_FORMAT_VERSION` to 3; write
      `doc/format/v3.md` (entry payload with `auth`, the `vault-key.age`
      artifact, the MAC construction); add `tests/fixtures/format-v3/` (a
      golden v3 vault); add an ADR for the authentication decision. Update the
      `format_v2_*` tests so the frozen v2 fixture now asserts refusal.
- [ ] **Init mints the key.** Test: `Vault::init` / `create` leaves a
      readable `vault-key.age` sealed to the initial recipients. Then wire it
      into vault creation.

## Definition of done

- Every task's tests are green; `make lint` and `make test` clean on Linux,
  macOS and Windows.
- An entry produced without the MAC key, or relocated to another id, is
  rejected on read; a legitimate recipient round-trips.
- Format is v3 with spec, fixture and ADR landed together; a v2 vault is
  refused.
- Stop for review. Do not start M1.
