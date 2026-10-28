# ADR-0007: A write resets the whole working tree

- Status: Accepted
- Origin: A21 (original project's register)

## Decision

Before a write, gage resets the **whole** working tree to HEAD and deletes
untracked files; it does not scope the reset to `entries/`.

## Context and consequences

A crash after a re-encryption writes the recipient files but before it
commits dirties those two files as well. A reset scoped to `entries/` would
leave exactly the half-migrated state the all-or-nothing re-encryption
exists to rule out. The same reset discards a stray `.gage/pending/` file
on the next write on that machine; nothing may rely on one surviving.
