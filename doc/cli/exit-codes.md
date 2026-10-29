# Exit codes

Every `gage` command exits with one of a fixed set of codes, so scripts and
CI can branch on *why* a command failed rather than just that it did. The
codes and their numbers are a contract
([ADR-0037](../decisions/0037-exit-code-taxonomy.md)). The numbers belong
to the command line: the library reports what kind of failure happened
(`gage::error::ErrorKind`), and `gage-cli`'s `exitcode` module maps each
kind to its code.

| Code | Name | Meaning |
|---|---|---|
| `0` | `success` | The command did what it was asked. |
| `1` | `conflict` | Something needs a human to resolve. Today: a name that's already registered, inserting, generating or renaming to a title another entry already has (without `--force`), a write that couldn't get the vault's lock within ten seconds, an `init` target that isn't an empty directory, a vault whose committed config this `gage` refuses (wrong `format_version`, an unsafe id or device name), and a registration that points at a different vault. From the sync commands: a divergence `pull` won't merge, a conflict `push` can't merge, a `sync` resolution that was skipped, aborted or had nobody to ask, a conflict on a file that isn't an entry or on the recipient files, and uncommitted changes in the vault's repository (see [Sync and conflicts](sync.md#what-sync-exits-with)). A `recipient verify` mismatch. A recipient-change confirmation answered no (the write is cancelled and nothing is written). A `recipient add` or `remove` run over a disagreement between `.age-recipients` and `.gage/config.toml`. From [enrollment](enrollment.md#what-can-go-wrong-and-what-it-exits-with): an approval declined at its `[y/N]` or cancelled at the code prompt, a request that is expired, malformed, sealed for another vault or skewed by the requesting device's clock, a device name that already labels a different recipient, an approver who can't read every entry, more than 32 live requests to try a code against, and a diverged history under `identity enroll` or `recipient approve`. From the [recovery key](identities-and-recipients.md#storing-your-recovery-key): a new recovery key whose last six characters weren't typed back (`init`, `recovery rotate`, `recovery enroll`; the change is committed anyway), a `recovery enroll --device` name the vault already lists without `--replaces` naming it, and a `recovery rotate` that would remove the key this device unlocked with. |
| `2` | `usage` | The command line itself was invalid: bad flags or arguments, an unknown (sub)command, `--script` with `--stdin`, either with a command, or a script file that can't be opened, an unknown `--type` or `--method`, an invalid `--device`, a malformed `--recipient`, a malformed `--field`, two of `insert`'s exclusive modes together (`-m`, `--value-stdin`, `-e`; `-e` with `--field`), a `generate` length below 8, an `insert -e` that comes back with nothing to save, or a command with no name and no default vault, `recipient remove` without `--reencrypt`, `recipient add` with the removed `--reencrypt`, or `mv`/`cp` without `--to-vault` or with the source vault as the destination. `identity enroll` with no remote or a bad `--ttl`, and `recipient approve --device` with an invalid name or against several requests. From the recovery key: `init`, `recovery enroll` or `recovery rotate` with no terminal to show a new key on and no flag saying where else it goes, `--no-recovery-key` (or `--no-new-recovery-key`) with `--recovery-key-out`, a `--recovery-key-out` file that exists or would land inside a vault or `$GAGE_DATA`, a device named `recovery-paper-key`, a pasted recovery key that isn't an age secret key (`recovery verify`, `recovery enroll`), a local identity file `recovery enroll` would replace without `--replaces` naming that device, and `recovery enroll --replaces recovery-paper-key`. |
| `3` | `not-found` | A name or query resolved to nothing. Today: a vault name that isn't registered (`mv`/`cp`'s `--to-vault` included), an entry query that matches no title or id, or a `show --field` name the entry doesn't have. An enrollment request id that matches no pending request, or a request another device approved or denied while `recipient approve` was running. A `recovery enroll --replaces` name that matches no recipient. |
| `4` | `ambiguous` | A query resolved to more than one candidate and there was nobody to ask (one-shot mode only). Today: any entry command whose query matches more than one entry at its first matching stage; see [Addressing entries](addressing-entries.md). Also an enrollment request id matching several pending requests. |
| `5` | `locked-or-auth` | An unlock failed: wrong passphrase, no local identity for this device. Or a git host refused gage: a rejected or expired token, a token file others can read, no ssh-agent for an SSH remote. Today `init` reaches it when no new passphrase is given, every entry command when the unlock fails, and any command that contacts a remote when the host refuses it (see [Git remotes and authentication](git-and-auth.md)). A `--script`/`--stdin` run reaches it when it has no passphrase source (no `GAGE_PASSPHRASE`, and no terminal to ask on), when `GAGE_PASSPHRASE` is wrong, and when a line would create an identity with no human to type its passphrase (see [Session mode](session-mode.md#where-the-passphrase-comes-from)). From enrollment: a code that opens no pending request, and an approval with nobody there to confirm it. From the recovery key: a pasted key that isn't one of the vault's recipients (`recovery verify`, `recovery enroll`), and one `recovery enroll` won't accept because it isn't registered as `recovery-paper-key`. `recovery verify` exits `0` for that second kind: the key does decrypt the vault. |
| `6` | `internal` | An unexpected error that fits none of the above: an unreadable file, a malformed config, a git failure, an unset or unusable `$EDITOR`, or an editor that exits non-zero. |
| `7` | `unreachable` | A remote could not be reached at all (DNS failure, connection refused, timeout). Reached only by a sync command you ran yourself (`sync`, `pull`, `push`), by `clone`, and by `identity enroll`: the automatic pull and push warn and carry on instead. |

`unreachable` (`7`) is deliberately distinct from `conflict` (`1`): nothing
about either side's state is wrong when a remote is unreachable, and
there's nothing for a human to resolve. The same command is simply worth
retrying once the network is back, and a script can tell the two apart.

`gage identity enroll` is the one command that *fails* on `7` rather than
warning and carrying on: an enrollment request that was never published
accomplishes nothing. See
[Adding a device](enrollment.md#enrollment-needs-the-network-and-fails-if-it-cant-reach-it).

Every error message goes to stderr, prefixed `gage:` exactly once. A usage
error also prints the failing command's own help.

A `--script` or `--stdin` run stops at its first failing line and exits
with that line's own code. Its message begins with where the line came
from, `commands.gage:3: show Foo: ...` (`<stdin>:3:` for `--stdin`).
