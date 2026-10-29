# The format-v2 golden vault

This directory is a vault and one device's local state, as gage-rs wrote
them in format v2. It is specified by [`doc/format/v2.md`](../../../doc/format/v2.md).

**Nothing here is a secret.** The passphrase, the recovery key and the
enrollment code below are public, and they open nothing but this
fixture.

## Never edit it

The fixture was written once by the generator, `write_format_v2_fixture`
in `crates/gage-cli/src/format_v2_tests.rs`, and is never regenerated in
place. `format_v2_fixture_is_byte_stable` checks every file against
`SHA256SUMS`.

A change to the format adds a `format-v3/` fixture beside this one. This
one stays, as the proof that v2 still reads or still migrates. See
[`doc/format/README.md`](../../../doc/format/README.md).

`.gitattributes` marks this tree `-text`, so no checkout rewrites its
line endings.

## What it holds

| Path | What it is |
|---|---|
| `vault.git/` | The vault's remote, a bare repository. Tests copy it, clone the copy, and read the clone. |
| `config/config.toml.in` | `$GAGE_CONFIG/config.toml`. `@VAULT_PATH@` and `@ORIGIN@` stand for the clone's and the remote's absolute paths. |
| `data/identities/<vault-id>/` | `$GAGE_DATA`: `laptop-1`'s wrapped identity and the `vault-name.txt` marker |
| `state/<vault-id>/` | `$GAGE_STATE`: `laptop-1`'s trust cache (`known-config.toml`, `trust.toml`) |
| `recovery-key.txt` | The secret half of the `recovery-paper-key` recipient |
| `SHA256SUMS` | Every other file's hash |

The vault:

- **Name** on this device: `personal`
- **Id:** `ff336b5e-3a36-4164-a014-1b6890bb9785`
- **Recipients:**
  - `laptop-1`: `age1yrmhdexp2nu9m4ty3m7s8yyupyv5kdcgkwxce9nkr82jycnmde6qxph5l3`
  - `recovery-paper-key`: `age1g6ltwxkauxtfhedctepamjta92w5cteyxe0zuxju4twpaleleamsjjgs30`
- **Entries:**
  - `github`: a description, two `fields`, and two keys this build
    doesn't know (`x-future-plain`, `x-future-map`)
  - `recovery notes`: a multi-line value
  - `wifi`: a title and a value only
- **Pending:** an enrollment request from `phone-1`
  - id `f61694fd-e69d-4d65-ab67-371884a7fce5`
  - code `GAGE-YTYX-6Z0M-SDX2-JTV3`
  - Its expiry passed long ago, so tests read it at a fixed clock.
- **History:**
  - `init`
  - two entry writes
  - the enrollment request
  - an entry written while the remote was unreachable
  - the `gage: merge origin` commit that joined them

`laptop-1`'s identity file:

- **Passphrase:** `format-v2 fixture passphrase`
- **Wrapped at:** scrypt work factor 10, not the shipped 19. Readers cap
  the factor at 22 and set no floor, so this is the real unwrap path,
  only cheaper.

`[shell]` sets every key, both durations included.
