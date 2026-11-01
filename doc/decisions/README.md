# Decision log

Architecture decision records for gage-rs. An ADR records one decision that
constrains future code: what was decided, why, and what follows. History
and discarded options stay in the [archive](../archive/README.md); an ADR
says what binds today.

**Every new decision is an ADR.** Add the next number, one decision per
file, and link it from the index below.

## Format

`NNNN-slug.md`, with:

- **Status**: Accepted, or Superseded by ADR-NNNN.
- **Origin**: the archived `Q-` entry or the original project's register
  entry the decision came from, so a `Q-NAME` reference in code maps to its
  ADR. Absent for decisions made here.
- **Context**, **Decision**, **Consequences**.

The on-disk format is governed by [doc/format/](../format/README.md), not
by this log; a format change needs an ADR as well as a new spec.

## Index

### Project

- [0001](0001-divergence-from-go-gage.md) gage-rs diverges and becomes its own project
- [0055](0055-module-hierarchy.md) The library is grouped by concern; the root re-exports what a frontend names
- [0056](0056-test-seams-off-production-types.md) Test seams live off production types; `Clock` is the session's injected time
- [0057](0057-typed-command-dispatch.md) The registry carries a typed `Command`; dispatch is exhaustive
- [0058](0058-release-and-versioning.md) Release and versioning policy
- [0059](0059-release-pipeline.md) The release pipeline's trust model
- [0060](0060-mobile-bindings.md) Mobile bindings live in this repo; the apps don't

### Format and storage

- [0002](0002-invariants-that-survive-the-port.md) Invariants kept from the port's compatibility decision
- [0003](0003-file-lock.md) Advisory lock: `std::fs::File::lock`
- [0004](0004-trust-hash.md) Recipient hash and diff
- [0005](0005-trust-cache-path.md) Trust cache location
- [0006](0006-vault-id-keying.md) Per-vault local state is keyed by an assigned vault id
- [0007](0007-dirty-tree-reset.md) A write resets the whole working tree
- [0008](0008-pending-directory.md) `.gage/pending/` is lazy and inert
- [0009](0009-unconditional-reencrypt.md) `recipient add` always re-encrypts
- [0010](0010-device-name.md) Device names: default, storage and validation
- [0011](0011-method-scope.md) Decryption method is per-device
- [0012](0012-method-flag.md) `--method` is optional and allowlisted
- [0013](0013-orphan-identity-by-pubkey.md) `vault remove` compares public keys and asks
- [0052](0052-vault-id-type.md) Vault ids are a validated type
- [0053](0053-vault-type-closed-method-open.md) Vault type is closed, unlock method is open

### Git and sync

- [0014](0014-git-crate.md) `git2`, statically vendored
- [0015](0015-ssh.md) SSH remotes: ssh-agent only
- [0016](0016-git-isolation.md) libgit2 ignores the user's git config
- [0017](0017-sync-classify.md) Fetch and push failures are classified by type
- [0018](0018-remote-timeout.md) Network operations are bounded to 30 s
- [0019](0019-file-merge.md) gage's own file-level merge
- [0020](0020-remote-auth.md) Tokens reach libgit2 through the credentials callback
- [0021](0021-sync-conflict-resolution.md) Conflict resolution

### Crypto and secret memory

- [0022](0022-age-crate.md) `age` 0.12, default features
- [0023](0023-identity-secret.md) `Identity` holds its key in locked memory
- [0024](0024-scrypt-hook.md) The scrypt work factor and its test hook
- [0025](0025-enroll-scrypt-hook.md) The enrollment seal's work factor and hook
- [0026](0026-enroll-scrypt-cap.md) The enrollment open path caps the claimed work factor at 18
- [0027](0027-recovery-secret-memory.md) How recovery secrets are held
- [0028](0028-recovery-noclobber.md) `--recovery-key-out` never overwrites
- [0029](0029-index-arena.md) The metadata index holds plaintext in one locked arena

### CLI behavior

- [0030](0030-help-render.md) clap renders help; the registry is the source
- [0031](0031-help-surfaces.md) Two help surfaces, one registry
- [0032](0032-command-availability.md) Which commands work in a session
- [0033](0033-recipient-prompt.md) Recipient-change confirmation and `--yes`
- [0034](0034-script-passphrase.md) `GAGE_PASSPHRASE` in script mode
- [0035](0035-clipboard.md) Clipboard and its clear
- [0036](0036-editor.md) `edit`: finding `$EDITOR` and the scratch file
- [0037](0037-exit-code-taxonomy.md) Exit codes 0–7
- [0049](0049-qr-crate.md) The QR crate: encode-only, no decoder
- [0050](0050-line-editor.md) The line editor and the history file
- [0051](0051-recovery-key-out-refusals.md) What `--recovery-key-out` refuses
- [0054](0054-frontend-neutral-errors.md) The library's errors are frontend-neutral

### Device enrollment

- [0038](0038-enroll-prompter.md) The enrollment code is an argument
- [0039](0039-enroll-code-format.md) Enrollment code format
- [0040](0040-enroll-seal-cost.md) What opening a request may cost
- [0041](0041-enroll-ttl.md) Request lifetime
- [0042](0042-enroll-expiry-in-name.md) The expiry is in the filename
- [0043](0043-enroll-collisions.md) Device names are labels; the public key is the identity
- [0044](0044-enroll-print-only.md) No offline enrollment path
- [0045](0045-enroll-remote.md) When enroll touches the network
- [0046](0046-enroll-verbs.md) Enrollment command names

### Locking and dependencies

- [0047](0047-two-vault-lock-order.md) Two vaults are locked in vault-id order
- [0048](0048-yaml-crate.md) The YAML crate
