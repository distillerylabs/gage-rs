# ADR-0040: What opening a request may cost

- Status: Accepted
- Origin: D-ENROLL-SEAL-COST ([design doc](../design/gage-cli-init-design.md))

## Context

`pending/` is inert for authority ([ADR-0008](0008-pending-directory.md)) but
not for cost. Opening cost is the product of how many files sit in
`pending/`, the work factor each claims, and the factor gage writes seals at.
The first two are supplied by whoever can write to the repository.

## Decision

1. The open path caps the claimed work factor ([ADR-0026](0026-enroll-scrypt-cap.md)),
   or one committed file claiming 2^30 hangs `approve` for hours.
2. Seals are written at factor 14, not the identity file's 19
   ([ADR-0025](0025-enroll-scrypt-hook.md)). The code has 80 bits, which is
   infeasible to guess at any work factor, so the KDF adds almost nothing
   there; the realistic cost driver is a mistyped code, which must try every
   request before concluding it opens none.
3. One run attempts at most 32 live requests; more is refused.

The factor removes the cost at its root, and the bound still holds if the
factor is ever raised.
