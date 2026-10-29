# ADR-0027: How recovery secrets are held

- Status: Accepted
- Origin: Q-RECOVERY-SECRET-MEMORY ([entry](../archive/rust-port/open-questions.md#q-recovery-secret-memory))

## Decision

- `RecoveryKey.secret` is a `Zeroizing<String>`; the `SecretString` age
  returns is copied into it and dropped.
- A pasted key arrives as the `Zeroizing<String>` that `Prompter::value`
  returns. `render_qr` returns `Zeroizing<String>`. The key file's bytes
  are a `Zeroizing<Vec<u8>>`, allocated at its final size so it never
  reallocates and leaves an unzeroed copy.
- The library parses a pasted key with `KeyLine::parse` into the same
  page-locked `LockedBuf` an identity file uses ([ADR-0023](0023-identity-secret.md)).
- `recover_device` takes the secret **by value**. It is dropped (and zeroed)
  as soon as `KeyLine::parse` has copied it into the locked buffer, rather
  than staying alive across the lock, re-encryption and push, and it is
  dropped on every return.
- A parse failure is reported as a malformed recovery key, with age's error
  dropped rather than wrapped: age's parse errors can quote their input. This is pinned by a signature test and a no-copy
  source scan.
- The stderr text is written from the key's own buffer, never formatted into
  an intermediate `String`.

## Consequences

Recorded gaps, not closed: the display copies are zeroed but not
page-locked, so they can reach swap; and the `qrcode` crate's own buffers
(encoded data, module grid) are built from the secret and dropped without
being zeroed, with no way to wipe them. `show -q` goes through the same path.
Closing these would mean page-locking the QR renderer's output and stderr's
buffering.
