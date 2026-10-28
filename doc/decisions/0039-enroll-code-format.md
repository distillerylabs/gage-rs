# ADR-0039: Enrollment code format

- Status: Accepted
- Origin: D-ENROLL-CODE-FORMAT ([design doc](../design/gage-cli-init-design.md)); Q-CROCKFORD ([entry](../archive/rust-port/open-questions.md#q-crockford))

## Decision

Crockford base32, 16 characters (80 random bits), four hyphenated groups,
shown behind a cosmetic `GAGE-` prefix: `GAGE-7K4M-9QX2-P3RH-8WVN`.

- The encoding is hand-rolled in the library (a few lines over 80 bits from
  `OsRng`); no base32 crate is used, as the normalization rules are gage's own.
- The alphabet excludes `I`, `L`, `O`, `U`; decoding maps `I`/`L` → `1` and
  `O` → `0`.
- Input is normalized before use: uppercased, hyphens, spaces and any
  `GAGE-` prefix stripped.
- A code failing length or alphabet validation is rejected **before any
  decryption**, so a typo costs nothing instead of N scrypt runs.
- The joining device never persists the code: not in the vault, local state
  or session history. It is shown once.
- On the approving side the code reaches command history if given with
  `--code`; this is an accepted risk. Omitting `--code` and answering the
  masked prompt avoids it.

## Consequences

A word list was rejected: it would have to be shipped and versioned, and a
changed list would invalidate older codes. Adding one later is additive.
