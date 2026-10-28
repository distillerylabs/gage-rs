# ADR-0010: Device names: default, storage and validation

- Status: Accepted
- Origin: Q-DEVICE-NAME, A15 (original project's register)

## Decision

- **Storage.** `device`, `method` and `pubkey` live under
  `[vaults.<name>]` in the local global config, written by `init`,
  `identity add`, `identity enroll` and `clone`. They are per vault, because
  they can differ between vaults on one machine.
- **Default.** The normalized hostname: lowercased, truncated at the first
  dot, anything outside `[a-z0-9._-]` replaced with `-`, runs collapsed,
  length capped. If nothing usable remains, gage prompts rather than invent
  a name. `--device NAME` on `init` and `identity add` overrides it, and is
  the escape hatch for a hostname that discloses whose machine it is.
- **Validation is a security boundary.** A device name becomes a filename
  component in `$GAGE_DATA/identities/<vault-id>/<device>.age`, and it
  arrives from `.gage/config.toml`, which anyone with git write access can
  edit. A `[[recipients]]` entry reading `device = "../../etc/cron.d/x"`
  must be rejected, not resolved. Names are validated against the allowlist
  on read and on write, and no path is built from an unvalidated one.

## Consequences

`identity add` rejects a device name that already labels a recipient of the
vault. A name is a label, not an identity ([ADR-0043](0043-enroll-collisions.md)).
