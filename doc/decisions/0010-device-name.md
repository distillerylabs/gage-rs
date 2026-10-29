# ADR-0010: Device names: default, storage and validation

- Status: Accepted
- Origin: Q-DEVICE-NAME, A15 (original project's register)

## Decision

- **Storage.** `device`, `method` and `pubkey` live under
  `[vaults.<name>]` in the local global config. They are per vault, because
  they can differ between vaults on one machine. `device` and `method` are
  written by `init`, `identity add`, `identity enroll` and `clone`; `pubkey`
  only where a key is created or opened (`init`, `identity add`,
  `identity enroll`). `clone` holds no identity and records an empty one.
- **Default.** The normalized hostname: lowercased, truncated at the first
  dot, anything outside `[a-z0-9._-]` replaced with `-`, runs collapsed,
  capped at 63 characters. If nothing usable remains, the command fails as a
  usage error (exit 2) naming `--device` rather than inventing a name.
  `--device NAME` on `init` and `identity add` overrides it, and is the
  escape hatch for a hostname that discloses whose machine it is. An
  explicit name must already be valid; it is never rewritten.
- **Validation is a security boundary.** A device name becomes a filename
  component in `$GAGE_DATA/identities/<vault-id>/<device>.age`, and it
  arrives from `.gage/config.toml`, which anyone with git write access can
  edit. A `[[recipients]]` entry reading `device = "../../etc/cron.d/x"`
  must be rejected, not resolved.
- **The rule** (`devicename::valid`): non-empty, at most 63 characters,
  only `[a-z0-9._-]`, and **not made only of dots**. The allowlist alone
  admits `..`; the all-dots rule is what stops a name from naming the
  parent directory, so it is load-bearing. The allowlist excludes every
  path separator on every platform (`/`, and `\` and `:` on Windows).
- **Where it runs:** when `.gage/config.toml` is read (a config with an
  invalid recipient device name is refused), in the path builder
  (`paths::identity_file_path`), and in every writer that adds a recipient
  or seals or opens an enrollment request. No path is built from an
  unvalidated name.
- `recovery-paper-key` is reserved for the recovery recipient and refused as
  a device's name. That check is separate from `valid`, because the
  recovery recipient itself is written under that label.

## Consequences

`identity add` rejects a device name that already labels a recipient of the
vault. A name is a label, not an identity ([ADR-0043](0043-enroll-collisions.md)).
