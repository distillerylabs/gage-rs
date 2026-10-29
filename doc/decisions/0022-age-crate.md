# ADR-0022: `age` 0.12, default features

- Status: Accepted
- Origin: Q-AGE-CRATE ([entry](../archive/rust-port/open-questions.md#q-age-crate))

## Decision

Use the `age` crate, 0.12.1, default features only. Armor is never written,
so the `armor` feature stays off. The identity file is age-keygen format and
is parsed with `age::IdentityFile::from_buffer`.

- **Recipient validation** at the CLI boundary is a shape-only Bech32 check
  (`agekey`), not `x25519::Recipient` parsing. `.age-recipients` may legally
  hold plugin recipients (`age1yubikey1…`), and a vault shared with a device
  that uses one must still round-trip its config.
- **Scrypt work factor** is per recipient (`scrypt::Recipient::set_work_factor`)
  and is written into the file's stanza ([ADR-0024](0024-scrypt-hook.md)).
  `scrypt::Identity` caps the factor it accepts at a per-machine calibrated
  value, so gage sets its cap explicitly.
- **Errors.** A wrong passphrase is `DecryptError::DecryptionFailed` on a
  header where `Decryptor::is_scrypt()` is true. An X25519 file offered to a
  scrypt identity is `NoMatchingKeys` with `is_scrypt()` false, so it is a
  corrupt identity file. For entries, `NoMatchingKeys` means "not a
  recipient" and everything else is corrupt ciphertext. A damaged scrypt
  stanza body is indistinguishable from a wrong passphrase.
- **Mixing.** `Encryptor::with_recipients` refuses a scrypt recipient with
  anything else; gage refuses first.
- **`.age-recipients` parsing** in the round-trip test uses
  `x25519::Recipient` per non-blank, non-comment line. The `cli-common`
  feature is not used: it pulls `rpassword` and `pinentry` into the library.

## Consequences

- Debug builds run scrypt about 50x slower; `[profile.dev.package.*]`
  `opt-level = 3` for `scrypt`, `salsa20`, `pbkdf2`, `sha2` and `hmac` fixes
  that.
- `deny.toml` allows BSD-3-Clause for the dalek crates.
