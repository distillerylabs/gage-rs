# ADR-0008: `.gage/pending/` is lazy and inert

- Status: Accepted
- Origin: A22 (original project's register)

## Decision

The vault layout includes `.gage/pending/`, holding one sealed enrollment
request per file. Two properties bind every reader of the layout:

- **Lazy.** The directory is created by the first `identity enroll`. Git
  does not track empty directories, so its absence is normal and means "no
  pending requests", never an error.
- **Inert.** Nothing in `pending/` is read at encryption time. Pending files
  carry no authority, which is what keeps the directory from widening the
  trust boundary.

`.gitattributes` deliberately does **not** cover `pending/`: a union of two
devices' pending requests is the correct merge outcome, unlike a union of
two recipient lists ([ADR-0021](0021-sync-conflict-resolution.md)).

## Consequences

Inertness governs authority, not cost; the cost of opening requests is
bounded by [ADR-0040](0040-enroll-seal-cost.md).
