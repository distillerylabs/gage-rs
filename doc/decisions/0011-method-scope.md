# ADR-0011: Decryption method is per-device

- Status: Accepted
- Origin: Q-METHOD-SCOPE, A14 (original project's register)

## Decision

A vault's `[method].default` in `.gage/config.toml` is a suggestion for
devices joining it, not a constraint. Each device's actual method is
recorded in local config only ([ADR-0010](0010-device-name.md)) and never
committed. `init --method` sets both the vault default and the first
device's method; `identity add --method` sets only that device's.

## Context

Nothing cryptographic forces one method per vault: age recipients are public
keys, and one `.age` file can be encrypted to a mix. A committed
"device X uses passphrase" line would tell anyone with read access which
recipient is the softest target, while enabling nothing: unlock consults
only your own method, and `recipient add` needs only a public key.
