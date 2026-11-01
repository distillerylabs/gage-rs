# ADR-0062: No SSH remotes on iOS or Android

- Status: Accepted
- Origin: A1b, iOS build and simulator CI ([#180](https://github.com/distillerylabs/gage-rs/issues/180)); amends [ADR-0015](0015-ssh.md)

## Context

[ADR-0015](0015-ssh.md) supports SSH remotes through git2's `ssh` feature
and ssh-agent only. A phone has no ssh-agent: there is nothing for
`Cred::ssh_key_from_agent` to ask, and gage reads no key files. On iOS the
`ssh` feature still builds libssh2, and wires in `libssh2-sys` with a
vendored OpenSSL, for a path that can never succeed there.

## Decision

**iOS and Android build `gage-core` without git2's `ssh` feature.**
`crates/gage/Cargo.toml` declares git2 without `ssh`, and adds `ssh` back
(and `libssh2-sys`) in a `cfg(not(any(target_os = "ios", target_os =
"android")))` target table. Every other target is unchanged. Cargo's
resolver 3 enables a target table's features only when building for that
target; `tests/no_ssh_on_mobile.rs` proves it with `cargo tree --target`
for two mobile and three desktop targets. (`cargo metadata` can't: it
unifies features across platforms.)

**An SSH remote is refused up front on those targets.**
`remoteauth::method` returns `Error::SshUnsupported`, kind `Usage`,
`Remedy::UseHttps`, before the alias check, the agent check or any
network. Both SSH spellings are covered (`ssh://` and scp-like). HTTPS and
local remotes resolve as on desktop. The switch is a constant of the target
(`SSH_SUPPORTED`) passed into `method_for`, so host tests cover both
answers; `Cred::ssh_key_from_agent` is never reached on mobile.

## Consequences

- A mobile app joins a vault by its HTTPS URL and a token, the path
  [ADR-0020](0020-remote-auth.md) already makes the supported one.
- **OpenSSL stays linked on iOS.** git2 itself depends on `openssl-sys` on
  every Unix but macOS, and libgit2 on iOS uses Secure Transport for TLS,
  so the OpenSSL objects are dead weight git2 still names. Removing them
  means patching git2, which is out of scope. Measured on Apple silicon,
  release, against the spike's numbers:

  | | Spike (with `ssh`) | Without `ssh` |
  |---|---|---|
  | `libgage_ffi.a`, one iOS target | 42.7 MiB | 42.4 MiB |
  | linked Swift smoke executable | 15.0 MiB | 14.8 MiB |

  The saving is small; the point is that no libssh2 is built or linked
  for a path that cannot work.
- Android is covered by the manifest and the dependency-graph test; no
  Android build exists yet.
