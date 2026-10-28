# Security model and limitations

What gage protects, what it does not, and which limits are known and
accepted. The full analysis of who can do what is the "Trust boundaries"
section of the [design document](gage-cli-design.md). To report a
vulnerability, see [SECURITY.md](../../SECURITY.md).

gage is pre-1.0 and has **not been independently audited**. It leans on the
[age](https://age-encryption.org/) library for all cryptography and
implements none of its own, but the surrounding vault, sync and key-handling
logic is new. Evaluate it accordingly before trusting it with secrets you
cannot afford to lose or expose, and keep independent backups.

## What gage protects

- **Ciphertext is safe at rest and in the remote.** Anyone with read access
  to the repository but no vault key sees only opaque `.age` files with
  random names. Titles, descriptions and categories are inside the encrypted
  payload, and no filename or directory structure encodes vault contents.
- **No long-lived agent.** Unlocking happens in-process; the decrypted key
  lives only as long as that `gage` process (or session) runs. There is no
  daemon or socket holding a key after you walk away.
- **Key material is kept out of swap and core dumps where the OS allows.**
  Identity secrets, the in-session metadata index and pasted recovery keys
  live in page-locked buffers that are zeroed on drop
  ([ADR-0023](../decisions/0023-identity-secret.md),
  [ADR-0029](../decisions/0029-index-arena.md),
  [ADR-0027](../decisions/0027-recovery-secret-memory.md)), and core dumps
  are suppressed for the process.
- **Plaintext on disk is short-lived.** `edit` writes a scratch file with an
  unpredictable name, mode 0600, preferring a tmpfs, and overwrites and
  removes it on every exit path ([ADR-0036](../decisions/0036-editor.md)).
- **Untrusted vault metadata is validated.** The vault id, device names and
  token hosts become path components, and they arrive from files anyone
  with git write access can edit. They are validated before any path is
  built from them ([ADR-0006](../decisions/0006-vault-id-keying.md),
  [ADR-0010](../decisions/0010-device-name.md)).
- **The recipient list is guarded.** gage keeps a local trust cache and
  warns, and asks for confirmation, when `.age-recipients` changes
  unexpectedly ([ADR-0004](../decisions/0004-trust-hash.md),
  [ADR-0005](../decisions/0005-trust-cache-path.md),
  [ADR-0033](../decisions/0033-recipient-prompt.md)). A merge never builds
  a recipient list neither device wrote
  ([ADR-0021](../decisions/0021-sync-conflict-resolution.md)).

## Limits

- **Git write access is, transitively, the ability to become a recipient.**
  `.age-recipients` is a plain, committed file. Someone who can push to the
  vault can add their own key, and future writes will encrypt to it.
  Already-encrypted entries are never exposed this way, but treat write
  access to a vault as equivalent to eventual read access. The trust cache
  exists to make such a change visible.
- **A compromised local machine is out of scope.** If an attacker controls
  the device while a vault is unlocked, or can read your identity file,
  gage cannot help.
- **Remote authentication is HTTPS with a token, or ssh-agent.** All git
  operations run in-process through libgit2 rather than the `git` binary.
  `~/.ssh/config`, credential helpers and `insteadOf` rewrites are not
  honored ([ADR-0016](../decisions/0016-git-isolation.md)). HTTPS uses a
  token stored by `gage auth login` and offered only to the host it was
  stored for ([ADR-0020](../decisions/0020-remote-auth.md)). SSH remotes
  authenticate through ssh-agent only, never from key files, and an SSH host
  that is an alias in `~/.ssh/config` is refused with an explanation
  ([ADR-0015](../decisions/0015-ssh.md)).
- **Entries carry no format version of their own.** The vault's
  `format_version` governs them; see [doc/format/](../format/README.md).

## Accepted risks

Known, bounded, and not fixed on purpose.

- **A SIGKILL leaves a copied secret on the clipboard** if `--clip` was in
  use: the clear runs in the process that wrote the value
  ([ADR-0035](../decisions/0035-clipboard.md)).
- **The enrollment code reaches command history on the approving device**
  when given with `--code`. The exposure is device-local, the code dies in
  24 hours and the request is deleted on approval; omitting `--code` and
  answering the masked prompt avoids it
  ([ADR-0039](../decisions/0039-enroll-code-format.md)).
- **A hostname used as a device name discloses whose machine it is** to
  anyone who can read the vault's config. `--device` chooses another name
  ([ADR-0010](../decisions/0010-device-name.md)).
- **Display copies of a recovery key are zeroed but not page-locked**, so
  they can reach swap, and the QR encoder's internal buffers are not zeroed
  ([ADR-0027](../decisions/0027-recovery-secret-memory.md)).
- **Batch enrollment approval costs one scrypt run per code per pending
  request.** The cost is bounded by a work-factor cap and a limit of 32
  requests per run ([ADR-0026](../decisions/0026-enroll-scrypt-cap.md),
  [ADR-0040](../decisions/0040-enroll-seal-cost.md)).
- **Decryption of an identity file is only as strong as its passphrase.** The
  scrypt work factor is 2^19 ([ADR-0024](../decisions/0024-scrypt-hook.md)).
