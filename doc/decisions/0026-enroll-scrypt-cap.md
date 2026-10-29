# ADR-0026: The enrollment open path caps the claimed work factor at 18

- Status: Accepted
- Origin: Q-ENROLL-SCRYPT-CAP ([entry](../archive/rust-port/open-questions.md#q-enroll-scrypt-cap))

## Context

`pending/` is writable by any git writer, and each sealed file *claims* its
own scrypt work factor. With r=8, scrypt needs 1 KiB × N of memory, so a
file claiming 22 costs 4 GiB and about 8× a factor-19 unlock. Up to 32
requests are tried per run, which is about three minutes of KDF per code
tried, with an out-of-memory kill likely on a small machine. The identity
file's cap (22) suits a local file gage wrote at 19, not this.

## Decision

`ENROLLMENT_SCRYPT_MAX_WORK_FACTOR = 18`: four doublings above
`SHIPPED_ENROLLMENT_SCRYPT_WORK_FACTOR` (14), the headroom age's own default
cap leaves. One hostile file then costs 256 MiB and about 0.35 s.
There is one scrypt identity constructor, `crypt::capped_scrypt_identity`,
which takes the cap. It has two callers: `scrypt_identity` (identity files,
22) and `enrollment_scrypt_identity` (requests, 18). A guard test requires
that every call passes one of the two named cap constants.

## Consequences

- age refuses a request claiming more than 18 before running any KDF. It is
  then "not opened" by that code, the same as a wrong code. gage never
  writes such a file, so only a hostile or damaged one gets here.
- Raising the seal factor later must keep it at or below this cap. A guard
  test pins `SHIPPED_ENROLLMENT_SCRYPT_WORK_FACTOR <= ENROLLMENT_SCRYPT_MAX_WORK_FACTOR`.
