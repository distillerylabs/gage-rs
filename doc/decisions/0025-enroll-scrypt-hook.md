# ADR-0025: The enrollment seal's work factor and hook

- Status: Accepted
- Origin: Q-ENROLL-SCRYPT-HOOK ([entry](../archive/rust-port/open-questions.md#q-enroll-scrypt-hook)); D-ENROLL-SEAL-COST ([ADR-0040](0040-enroll-seal-cost.md))

## Decision

[ADR-0024](0024-scrypt-hook.md)'s pattern, in full, for the seal:

- `SHIPPED_ENROLLMENT_SCRYPT_WORK_FACTOR = 14` is a shipped `const` of its
  own. It is licensed by the code's 80 generated bits; its comment says not
  to merge it with `SHIPPED_SCRYPT_WORK_FACTOR`.
- It has its own live atomic and its own
  `set_enrollment_work_factor_for_tests` (`cfg(test)` / `test-support`).
  `fast_scrypt()` lowers both factors.
- Sealing goes through `crypt::Recipient::passphrase_at` at that factor.
  Opening goes through `enrollment_scrypt_identity`, a wrapper over the one
  scrypt identity constructor with the enrollment cap
  ([ADR-0026](0026-enroll-scrypt-cap.md)). Sealing never reads the identity
  file's live factor, so a suite that lowers one cannot silently lower the
  other.

## Consequences

Guard rows prove four things: the shipped value is deliberate; it reaches a
real age header; only the setter writes it; the two factors are independent
in both directions. The source-scan guards (one recipient site, one identity
site) are extended to the enrollment atomic rather than given exceptions.
