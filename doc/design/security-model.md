# Security model and limitations

What gage protects, what it does not, and which limits are known and
accepted. The full analysis of who can do what is the "Trust boundaries"
section of the [design document](gage-cli-design.md). To report a
vulnerability, see [SECURITY.md](../../SECURITY.md).

gage is pre-1.0 and has **not been independently audited**. It leans on the
[age](https://age-encryption.org/) library for encryption and on the
RustCrypto HMAC-SHA256 for entry authentication, and implements no primitive
of its own, but the surrounding vault, sync and key-handling logic is new.
Evaluate it accordingly before trusting it with secrets you cannot afford to
lose or expose, and keep independent backups.

## What gage protects

- **Ciphertext is safe at rest and in the remote.** Anyone with read access
  to the repository but no vault key sees only opaque `.age` files with
  random names. Titles, descriptions and categories are inside the encrypted
  payload, and no filename or directory structure encodes vault contents.
- **Only a vault member can write an entry gage accepts.** age alone doesn't
  give this: encrypting takes only the public keys, which are committed.
  Every entry therefore carries an HMAC tag under a per-vault key that is
  sealed to the recipients, bound to the id in its filename. Every read
  checks the tag before parsing anything, so a forged entry, an altered one,
  or a genuine one moved to another id is refused
  ([ADR-0065](../decisions/0065-entry-authentication.md)). A refused entry
  fails the command rather than being skipped, so a listing never looks
  complete when it isn't. Removing a recipient replaces the key in the same
  commit, and a live entry verifies against the current key only
  ([ADR-0066](../decisions/0066-mac-key-rotation.md)).
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
  removes it however the edit ends, including on `SIGINT`, `SIGQUIT`,
  `SIGTERM` and `SIGHUP` on Unix
  ([ADR-0036](../decisions/0036-editor.md),
  [ADR-0070](../decisions/0070-edit-scratch-signals.md)).
- **Untrusted vault metadata is validated.** The vault id, device names and
  token hosts become path components, and they arrive from files anyone
  with git write access can edit. They are validated before any path is
  built from them ([ADR-0006](../decisions/0006-vault-id-keying.md),
  [ADR-0010](../decisions/0010-device-name.md)).
- **The recipient list and the vault key are guarded.** gage keeps a local
  trust cache and warns, and asks for confirmation, when `.age-recipients`
  changes unexpectedly ([ADR-0004](../decisions/0004-trust-hash.md),
  [ADR-0005](../decisions/0005-trust-cache-path.md),
  [ADR-0033](../decisions/0033-recipient-prompt.md)). A merge never builds
  a recipient list neither device wrote
  ([ADR-0021](../decisions/0021-sync-conflict-resolution.md)). The cache
  also records `.gage/vault-key.age`, and a change to that file alone warns
  as its own case ([ADR-0066](../decisions/0066-mac-key-rotation.md)).
- **A replaced vault key is refused.** Anyone can seal a new key to the
  public recipient list, so each device records a one-way fingerprint of the
  key it last saw and refuses a key file that doesn't contain that key as
  its current or a retired key. Someone without the key can't write a file
  that passes, even if they also add a recipient, and `--yes` doesn't get
  past it ([ADR-0067](../decisions/0067-key-continuity.md)).

## Limits

- **Git write access is, transitively, the ability to become a recipient.**
  `.age-recipients` is a plain, committed file. Someone who can push to the
  vault can add their own key, and future writes will encrypt to it.
  Already-encrypted entries are never exposed this way, but treat write
  access to a vault as equivalent to eventual read access. The trust cache
  exists to make such a change visible. Adding a key does not hand over the
  vault key, which stays sealed to the keys it was sealed to until a member
  re-encrypts the vault.
- **Entry authentication says a recipient wrote this, not which one.** Every
  current recipient holds the same key, so any of them can write or alter
  any entry. There is no per-device provenance, and `updated_by` is what the
  writer said. A vault is the unit of trust.
- **A removed member has a window.** They hold the key until the removal's
  rotation happens, so what they push before it is accepted like any other
  write from before the rotation. Afterwards their key produces nothing a
  live read accepts, though someone who can still push can forge a past
  revision, visible only through `history`, which accepts retired keys. They
  also keep whatever they could already read from git history
  ([ADR-0066](../decisions/0066-mac-key-rotation.md)).
- **Authentication is not freshness.** A tag proves an entry was written by
  a member, not that it is the newest version or that nothing is missing.
  Someone with git write access can put back an older revision still tagged
  under the current key, delete an entry, or roll back the whole vault.
- **A device's first look at a vault is trusted.** The key check compares
  against what the device has seen, so a device that has never read a vault
  takes the key it finds. `pull` needs no key, so a replaced key file still
  reaches a clone, where every read and write then refuses it
  ([ADR-0067](../decisions/0067-key-continuity.md)).
- **One unverifiable entry stops the vault's reads.** That is the
  fail-closed choice above, and it means anyone with git write access can
  deny service by pushing a bad file, as they could already by pushing a
  corrupt one. gage can't address such an entry either, so the way out is
  `git` in the vault's directory; see
  [When an entry fails to verify](../cli/entries.md#when-an-entry-fails-to-verify).
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
- **A SIGKILL or power loss leaves the `edit` scratch file behind**, and so
  can Ctrl-C or closing the console on Windows: nothing runs to remove it
  ([ADR-0070](../decisions/0070-edit-scratch-signals.md)).
- **Decrypted values are not page-locked or reliably wiped.** Identity
  secrets and the session index are; an entry's `value` and `fields`, once
  decrypted to be shown or edited, live in ordinary heap. Copies can outlast
  the command, stay after `lock`, and reach swap, though core dumps stay
  suppressed.
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
