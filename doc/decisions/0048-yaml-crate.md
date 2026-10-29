# ADR-0048: The YAML crate

- Status: Accepted
- Origin: Q-YAML-CRATE ([entry](../archive/rust-port/open-questions.md#q-yaml-crate))

## Context

An entry is a YAML document inside the encrypted payload
([format v2](../format/v2.md)). gage does not hand the entry to a library to
style: it chooses a style per scalar, re-parses its own output, and falls
back to quoting every string if anything changed, because a library's
styling can lose a leading newline or write tabs into a block it cannot read
back. That property is gage's to own whatever the library, so the question
was only which crate reads entries.

## Decision

`saphyr` 0.2 parses, a gage-owned emitter writes, and `jiff` parses
timestamps.

- `saphyr` (pure Rust, YAML 1.2, MIT/Apache) loaded with
  `early_parse(false)` keeps every scalar's original text and style, so an
  unknown field `x: 0x1F` stays `0x1F` and is re-emitted as written. It
  rejects duplicate keys and records spans, which `cat`'s line alignment
  needs. It does not strip a BOM and accepts several documents, so gage
  strips the one and refuses the other.
- `serde_yaml_ng` was rejected: it reads `0x1F` back as `31` and offers no
  per-scalar style control, so the quoted fallback cannot be expressed.
  `serde_norway` is the same lineage. `serde_yml` is unsound
  (RUSTSEC-2025-0068).
- `jiff` parses RFC 3339 with any offset and normalizes to UTC seconds.
  `humantime` accepts only `Z`.

## Consequences

- `saphyr` is used with `default-features = false`, which adds **Zlib**
  (`foldhash`) to the `deny.toml` license allowlist.
- The rules for reading gage's own fields (raw text, `null` as empty, what
  is a parse error) and for emitting live in the entry module and
  [format v2](../format/v2.md).
