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

## Decisions (settled 2026-11-07, reviewed before code)

1. **MAC input canonicalization: exact bytes plus a trailer, no
   re-serialization.** The payload is the emitter's YAML body followed by
   one final line, `auth: hmac-sha256:<64 lowercase hex>\n`. The tag is
   `HMAC-SHA256(mac_key, "gage-entry-auth-v3\n" ‖ uuid ‖ "\n" ‖ body)`,
   where `uuid` is the filename's id in its canonical 36-character
   lower-case hyphenated form and `body` is every byte before the `auth`
   line. On read, the last line is split off and checked before anything
   is parsed, and the tag is compared in constant time. Chosen over the
   proposal (serialize without `auth`, and on read re-serialize and
   compare) because that ties every existing MAC to the emitter's exact
   output forever, and parses unauthenticated bytes. `auth` becomes a
   reserved key: an entry carrying it among its unknown fields is refused
   on write, and a body that contains it is refused on read.
2. **The key file: `.gage/vault-key.age`**, an `age` file whose plaintext is
   the raw 32-byte key, encrypted to every key in `.age-recipients`. It sits
   under `.gage/` beside `config.toml`, and is marked `-merge` in
   `.gitattributes` like the recipient files.
3. **A v2 vault is refused** at config load through the existing
   `UnsupportedFormatVersion` path (skew `Older`). The older-skew advice says
   the vault was written in an older format this build no longer reads.
4. **A v3 vault without a readable `vault-key.age` is a hard error**
   (`Error::VaultKey`), never an unauthenticated read.
5. **The M0/M1 boundary for membership.** M0's `reencrypt_to` also re-seals
   the *existing* key to the new list (after the entries, before the
   recipient files), so `recipient add`, enrollment approval and the
   recovery swap keep working and M0 is green on its own. M1 keeps
   rotation on remove, the trust-check coverage, and the explicit
   per-holder tests. (For M1: `history` verifies old revisions with the
   current key, so rotation must keep old revisions readable.)
6. **API consequences.** Stamping needs the key and the key needs an
   identity, so the writers that took none now take one:
   `Vault::write_entry(id, e, identity)` and
   `Vault::update(identity, id, e, prompter)`. `Vault::create` seals the key
   to the initial recipients, so it now refuses a recipient this build
   cannot encrypt to (a plugin key), which `write_entry` already refused.

## Tasks (tests first)

- [x] **MAC key type + sealing.** Test: a generated key round-trips through
      seal→open with a recipient identity; a non-recipient cannot open it.
      Then: a small module (under `crypto`) that generates a 32-byte key,
      seals it to a recipient list (reusing `crypt::encrypt`), and opens it
      with an `Identity`.
- [x] **Stamp on write.** Test: `write_entry` produces a payload whose `auth`
      verifies. Then: `write_entry_to` / `encrypt_entry` compute `auth` over
      `uuid ‖ fields` and include it. `encrypt_entry` takes the id so it can
      bind it.
- [x] **Verify on read.** Test: a correct entry reads; an entry whose `auth`
      is wrong, missing, or computed for a different uuid is rejected with a
      new `Error::Unauthenticated`. Then: `decrypt_entry` / `read_entry`
      recompute and compare against the filename's uuid.
- [x] **Non-recipient cannot produce a valid entry.** Test (ports the
      assessment's forgery check, neutrally): an entry encrypted to the public
      recipients *without* the MAC key is rejected on read; an entry file
      placed under a different id is rejected.
- [x] **Format v3.** Bump `CURRENT_FORMAT_VERSION` to 3; write
      `doc/format/v3.md` (entry payload with `auth`, the `vault-key.age`
      artifact, the MAC construction); add `tests/fixtures/format-v3/` (a
      golden v3 vault); add an ADR for the authentication decision. Update the
      `format_v2_*` tests so the frozen v2 fixture now asserts refusal.
- [x] **Init mints the key.** Test: `Vault::init` / `create` leaves a
      readable `vault-key.age` sealed to the initial recipients. Then wire it
      into vault creation.

### Where each task's tests are

- MAC key: `crypto/vaultkey.rs` (round trip, non-recipient, wrong length,
  absence, RFC 4231 known answer, the tag's exact input).
- Stamp and verify: `entry/auth.rs` (the `auth` line's shape, missing,
  mismatch) and `entry/store.rs` (`write_entry_stamps_an_auth_tag_that_verifies`,
  `read_entry_rejects_a_wrong_auth_tag`, `…_without_an_auth_tag`,
  `…_a_tag_computed_for_a_different_id`, `auth_is_a_reserved_field_name`,
  `read_and_write_without_the_vault_key_fail_typed`).
- Forgery: `an_entry_produced_without_the_vault_key_is_rejected` and
  `an_entry_file_placed_under_a_different_id_is_rejected` (`entry/store.rs`),
  and against the golden vault in
  `format_v3_vault_key_opens_and_authenticates_every_entry`.
- Format v3: `doc/format/v3.md`, `tests/fixtures/format-v3/` (written once by
  `write_format_v3_fixture`), `format_v3_*` in both crates, ADR-0065. The
  `format_v2_*` tests that opened the v2 vault now prove it is refused
  (`format_v2_vault_is_refused_as_an_older_format`,
  `format_v2_vault_is_refused_by_every_command`); those that didn't open it
  are unchanged.
- Init: `create_seals_a_fresh_vault_key_to_every_initial_recipient`,
  `create_refuses_a_recipient_it_cannot_seal_the_key_to`, and the CLI's
  `init_with_a_recipient_it_cannot_encrypt_to_fails_before_creating_files`.

### Beyond the task list (forced by the change)

- `mv`/`cp` unlock the destination too (decision 6): `Vault::move_to` /
  `copy_to` take the destination's identity, and
  `Session::vault_and_destination` returns it. `doc/cli/sharing.md` and the
  design doc said the destination is never unlocked, and are corrected.
- Test fixtures that built an entry by copying another entry's file (the CLI
  sync conflict fixture) now write a genuine entry under the target id.

## Definition of done

- Every task's tests are green; `make lint` and `make test` clean on Linux,
  macOS and Windows.
- An entry produced without the MAC key, or relocated to another id, is
  rejected on read; a legitimate recipient round-trips.
- Format is v3 with spec, fixture and ADR landed together; a v2 vault is
  refused.
- Stop for review. Do not start M1.
