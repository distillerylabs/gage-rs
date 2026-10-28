# ADR-0004: Recipient hash and diff

- Status: Accepted
- Origin: Q-TRUST-HASH ([entry](../archive/rust-port/open-questions.md#q-trust-hash))

## Context

The trust cache detects an unexpected change to `.age-recipients` and shows
what changed.

## Decision

Hash `.age-recipients` with SHA-256 from `sha2` (a direct dependency at the
version already in the tree through `age`; `cargo tree -d` must show no
second copy). Compute the diff with gage's own LCS unified diff.

## Consequences

The diff covers a few short TOML files, so a diff crate would add a
dependency for about a hundred lines. The LCS diff has its own test.
