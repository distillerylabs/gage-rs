# ADR-0012: `--method` is optional and allowlisted

- Status: Accepted
- Origin: Q-METHOD-FLAG, A13 (original project's register)

## Decision

`--method` is treated like `--type`: optional, defaulting to `passphrase`,
validated against an allowlist of accepted values. A new method arrives as
an additional accepted value, so the CLI surface grows additively. The usage
line documents what the tool accepts today; not-yet-real values are
described in prose only.
