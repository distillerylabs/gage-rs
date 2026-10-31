# Security policy

`gage` is a secrets manager, so security reports are taken seriously. Please
report vulnerabilities privately rather than in a public issue.

## Reporting a vulnerability

Use GitHub's private vulnerability reporting: open the repository's
**Security** tab and choose **Report a vulnerability**
(<https://github.com/distillerylabs/gage-rs/security/advisories/new>).

Please include:

- what you found and its impact (what an attacker gains, and what access
  they need first);
- steps or a minimal repro, including the `gage` version (`gage --version`)
  and OS;
- whether you'd like to be credited when the fix is published.

You should get an acknowledgement within a few days. `gage` is maintained on
a best-effort basis, so fix timelines depend on severity and complexity; we'll
keep you updated and coordinate disclosure with you.

## Supported versions

Only the latest release is supported; fixes are not backported
([ADR-0058](doc/decisions/0058-release-and-versioning.md)). Before 1.0, a
fix may ship in a minor release.

To check that what you downloaded is what this repository built, see
[Verifying a download](README.md#verifying-a-download).

## Scope and known limitations

`gage` has **not been independently audited**. Before reporting, please read
the [security model](doc/design/security-model.md) and the "Trust boundaries"
section of the [design doc](doc/design/gage-cli-design.md). These are
documented limits rather than vulnerabilities:

- Write access to a vault's git remote is, transitively, the ability to add a
  recipient for *future* writes (already-encrypted entries stay unreadable).
- A compromised local machine, or one where an attacker can read an unlocked
  session's memory or an identity file, is out of scope.
- Remote authentication supports HTTPS with a token, or ssh-agent for SSH
  remotes; `~/.ssh/config`, git credential helpers and `insteadOf` rewrites
  are intentionally not honored.

Reports that break a guarantee the docs *do* make are in scope, for example:
decrypting entries without a valid key, leaking titles or metadata into
ciphertext filenames, commit messages, or logs, leaking a token or key into
output, bypassing the recipient-change warning, path traversal via device or
vault names, or unsafe file permissions on key material.
