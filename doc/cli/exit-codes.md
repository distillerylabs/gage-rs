# Exit codes

Every `gage` command exits with one of a fixed set of codes, so scripts and
CI can branch on *why* a command failed rather than just that it did. The
codes and their numbers are Go gage's.

| Code | Name | Meaning |
|---|---|---|
| `0` | `success` | The command did what it was asked. |
| `1` | `conflict` | Something needs a human to resolve. Today: a name that's already registered, inserting, generating or renaming to a title another entry already has (without `--force`), a write that couldn't get the vault's lock within ten seconds, an `init` target that isn't an empty directory, a vault whose committed config this `gage` refuses (wrong `format_version`, an unsafe id or device name), and a registration that points at a different vault. Later: sync divergence, a `recipient verify` mismatch. |
| `2` | `usage` | The command line itself was invalid: bad flags or arguments, an unknown (sub)command, an unknown `--type` or `--method`, an invalid `--device`, a malformed `--recipient`, a malformed `--field`, two of `insert`'s exclusive modes together (`-m`, `--value-stdin`, `-e`; `-e` with `--field`), a `generate` length below 8, an `insert -e` that comes back with nothing to save, or a command with no name and no default vault. |
| `3` | `not-found` | A name or query resolved to nothing. Today: a vault name that isn't registered, an entry query that matches no title or id, or a `show --field` name the entry doesn't have. |
| `4` | `ambiguous` | A query resolved to more than one candidate and there was nobody to ask (one-shot mode only). Today: any entry command whose query matches more than one entry at its first matching stage; see [Addressing entries](addressing-entries.md). |
| `5` | `locked-or-auth` | An unlock failed: wrong passphrase, no local identity for this device, an expired git auth token. Today `init` reaches it when no new passphrase is given, and every entry command when the unlock fails. |
| `6` | `internal` | An unexpected error that fits none of the above: an unreadable file, a malformed config, a git failure, an unset or unusable `$EDITOR`, or an editor that exits non-zero. |
| `7` | `unreachable` | A remote could not be reached at all (DNS failure, connection refused, timeout). Reached only by an explicitly invoked sync command, which doesn't exist yet. |

`unreachable` (`7`) is deliberately distinct from `conflict` (`1`): nothing
about either side's state is wrong when a remote is unreachable, and
there's nothing for a human to resolve. The same command is simply worth
retrying once the network is back, and a script can tell the two apart.

Every error message goes to stderr, prefixed `gage:` exactly once. A usage
error also prints the failing command's own help.
