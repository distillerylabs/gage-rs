# ADR-0046: Enrollment command names

- Status: Accepted
- Origin: D-ENROLL-VERBS, Q-ENROLL-VERBS ([design doc](../design/gage-cli-init-design.md))

## Decision

`gage identity enroll` on the joining side; `gage recipient pending`,
`approve` and `deny` on the approving side. No new top-level noun and no new
help group. Re-running `identity enroll` mints a fresh request and a fresh
code, never reusing the pending one: reuse would force gage either to
persist the code ([ADR-0039](0039-enroll-code-format.md)) or to leave two
live ids for one device. Duplicates are harmless because `pending/` is inert,
expiry prunes them, and a late approval of an already-recipient key is a
no-op ([ADR-0043](0043-enroll-collisions.md)).
