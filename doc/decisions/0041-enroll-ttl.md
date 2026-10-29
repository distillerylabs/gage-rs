# ADR-0041: Request lifetime

- Status: Accepted
- Origin: D-ENROLL-TTL ([design doc](../design/gage-cli-init-design.md))

## Decision

A request lives 24 hours by default; `--ttl` overrides it, with a hard
ceiling of 7 days. `--ttl` is validated at the boundary (positive, at most
the ceiling) and rejected as a usage error before anything is generated or
written. The ceiling also bounds the filename's expiry value
([ADR-0042](0042-enroll-expiry-in-name.md)): an epoch further than the ceiling
past now is treated as expired and pruned, which stops a forged far-future
name from parking a blob in the tree. Expiry is always enforced from the
sealed copy, never from the filename or the listing.
