# ADR-0024: The scrypt work factor and its test hook

- Status: Accepted
- Origin: Q-SCRYPT-HOOK ([entry](../archive/rust-port/open-questions.md#q-scrypt-hook))

## Decision

- `SHIPPED_SCRYPT_WORK_FACTOR = 19` is a `const`.
- The live value is a `static AtomicU8` that the crypto path reads.
- Its setter exists only under `#[cfg(any(test, feature = "test-support"))]`,
  so it is absent from the shipped binary. `gage-cli` enables
  `test-support` only as a dev-dependency, which resolver 3 keeps out of
  normal builds.
- `testsupport::fast_scrypt()` (a `std::sync::Once`) lowers the value for
  fixtures that create or unlock vaults; a test that forgets it is slow, not
  wrong.

The read side has a cap: `SCRYPT_MAX_WORK_FACTOR = 22` is set explicitly on
every scrypt identity, because age's own ceiling is calibrated per machine
and a damaged or hostile identity file claiming 2^30 must cost a bounded
wait, not a hang.

Three guard tests stay:

1. the constant is 19;
2. a source scan allows only the one declaration, `.load` reads and the
   setter's body, so an alias of the static is refused too;
3. a real file written with the shipped value carries 19 in its header,
   proved in a child process (the test binary re-executed) that never calls
   the hook, because tests share one process and run in parallel.

## Consequences

Every scrypt recipient is built at one site (`Recipient::passphrase_at`) and
every scrypt identity at one site (`capped_scrypt_identity`), which always
sets a cap; source-scan guards (`every_scrypt_identity_caps_its_work_factor`
among them) enforce both. [ADR-0025](0025-enroll-scrypt-hook.md) and
[ADR-0026](0026-enroll-scrypt-cap.md) extend the same pattern.
