# ADR-0049: The QR crate: encode-only, no decoder

- Status: Accepted
- Origin: Q-QR-CRATE ([entry](../archive/rust-port/open-questions.md#q-qr-crate))

## Context

`-q/--qr` shows a value, and the recovery key, as a terminal QR code. Two
questions: which crate encodes, and how the rendered output is tested
without a second crate to read it back.

## Decision

`qrcode` with `default-features = false` encodes, at error-correction level
M with a 4-module quiet zone. The half-block renderer, two modules per
character cell, is gage's own.

- The crate's default features pull in `image` for rendering gage does not
  use. Its module grid (`to_colors`) is all the renderer needs.
- Tests decode the rendered characters back to a grid and compare it with
  the encoder's own grid for the same value. Whether the encoder encodes
  correctly is the encoder's to prove. No decoder dependency is added.
- The recovery-key display reuses the same renderer.

## Consequences

- `crates/gage-cli` depends on `qrcode`; `crates/gage` does not, which the
  dependency check in CI enforces.
