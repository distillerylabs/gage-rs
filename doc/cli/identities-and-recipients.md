# Identities and recipients

`gage` separates two concerns that are easy to conflate: **who can read a
vault** (recipients: a vault-wide list of public keys) and **how a
particular device proves it holds the matching private key** (identity:
decided independently by each device, never dictated by the vault).

> **Not yet in gage-rs.** Enrolling a second device with a short code
> (`identity enroll`, `recipient pending`, `approve` and `deny`) is phase
> P7 of the [port plan](../implementation/00_gage-cli/plans/rust-port/index.md),
> and the recovery key (`gage recovery ...`, and the `recovery-paper-key`
> recipient `init` will register) is P8. Until then a new device joins by
> the hand-carried-key path below.

## Identity: how this device proves it can decrypt

```
gage identity add  [--use NAME] [--method passphrase] [--device NAME]
gage identity list [--use NAME]
```

`identity add` registers how *this device* holds its private key and
prints the resulting public key so it can be added as a recipient: either
by you, if you're bootstrapping a new vault, or by an existing recipient
adding you to theirs with `gage recipient add <pubkey> --device NAME`.

- **`--method`** is this device's own choice, not the vault's. Omitted, it
  takes the vault's configured default method. `passphrase` is the only
  method today.
- **`--device`** names this device as a recipient label. Omitted, it
  defaults to your normalized hostname (lowercased, truncated at the first
  dot, non-`[a-z0-9._-]` characters replaced). A name already registered as
  a recipient of this vault is rejected rather than silently taken over:
  pass `--device` explicitly to get past that, which is what two machines
  with the same normalized hostname need.
- It **reuses an existing identity file** rather than generating a second
  key and orphaning the first, so re-running it after a failure is safe.
  It prompts for a new passphrase twice when it creates a key.
- A hostname-derived name is convenient but not private: it's stored in a
  plaintext, committed file every recipient can read. If that's a concern
  for a shared vault, pass `--device` explicitly.

`identity list` shows the keys this machine holds for a vault.

For the passphrase method, `identity add` also writes a wrapped identity
file to local, device-only storage (never inside the vault's own git tree;
see [Configuration](configuration.md) for exactly where). **Losing that
file is a recovery problem, not a backup problem:** the fix is the same as
"this device is gone". Run `gage identity add` again to register a fresh
key, and have any other recipient add the printed key with
`gage recipient add`. Neither verb overwrites an identity file that's
still there; recovery means registering a *new* key, never restoring the
old one from a backup.

## Recipients: who can decrypt a vault

```
gage recipient add <pubkey> --device NAME [--use NAME]
gage recipient remove <pubkey-or-name> --reencrypt [--use NAME]
gage recipient list [--use NAME]
gage recipient verify [--use NAME] [--repair]
```

- **`recipient add`** takes a bare public key and **requires `--device`**:
  a public key says nothing about whose it is, and the label is what every
  later `recipient remove` and `list` addresses it by, so there's no
  sensible default to invent.
- **`recipient add` authorizes that key and re-encrypts every existing
  entry** to include it, so the new recipient can read the vault's whole
  history. There is no `--reencrypt` flag and no way to skip it: passing
  the flag is rejected as a usage error rather than accepted and ignored,
  so a script that asked for the old optional behavior finds out. A
  recipient who can read only part of a vault isn't an access tier anyone
  chose, and the state spreads: a partially admitted device can neither
  repair its own access nor grant full access to anyone else. The other
  side of that rule is that adding a recipient requires being able to read
  every entry yourself; run it from a device that can. If you can't,
  `gage` says so, naming how many entries are unreadable, before it
  changes anything.
- **`recipient remove`** *requires* `--reencrypt`: `gage` refuses to
  silently leave old ciphertext readable by someone you just revoked. It
  prints a clear warning either way: **removing a recipient revokes future
  access only.** Anything they already read can't be unread. Removing this
  device's own key is allowed but asks first (and `--yes` does *not* answer
  that question); removing the last recipient is refused outright.
- **Re-encryption is all-or-nothing.** Every entry is re-encrypted in the
  working tree, and the recipient list plus every touched entry land in
  exactly one commit: nothing commits until the whole operation succeeds.
  If it's interrupted (crash, kill, power loss), the vault is left exactly
  as it was before the command ran, and re-running the command starts
  cleanly from scratch.
- **`recipient verify`** checks that `.age-recipients` and
  `.gage/config.toml` agree on who's a recipient. It needs no unlock (both
  files are plaintext), so it works before any identity is available and
  is safe to run in CI. It exits `0` and prints "in sync" on a match, or
  exits `1` and lists the specific differences otherwise.
- **`recipient verify --repair`** is the way out of that disagreement. It
  rewrites `.age-recipients` from `.gage/config.toml`'s list and commits
  it, after naming every key it drops and every key it adds: `gage` says
  what it dropped rather than only that it succeeded, since those keys
  could read everything written while they were listed. `recipient add`
  and `remove` refuse to run over a disagreement at all (they rebuild
  `.age-recipients` from `config.toml`, so running one would erase the
  stray key and commit that as an ordinary recipient change), which is why
  the repair is its own explicit step. It takes no `--reencrypt`, since
  rewriting a plaintext key list says nothing about whether existing
  ciphertext should be rewritten, and it does not clear the trust cache,
  so the next write still shows you the recipient list for review.

## The recipient-change confirmation

Before encrypting to a vault's current recipient list, `gage` compares it
against a locally cached copy from the last time you used this vault (its
"local trust cache", see [Configuration](configuration.md#where-things-live)).
If the list has changed since then (a new recipient was added, on this
device or another) you get a warning and a confirmation prompt before the
write proceeds, so a compromised or carelessly merged recipient list can't
silently start encrypting your secrets to someone new. Answering no
cancels the write (exit `1`) and leaves the vault and the cache as they
were, so the same question comes back next time.

- **What counts as a change.** The cache holds a verbatim copy of
  `.gage/config.toml` and a hash of `.age-recipients`. The first is what
  gets diffed and shown, since device names make a legible warning; the
  second catches someone editing only the file encryption actually reads.
  The first time you use a vault there is nothing to compare against, and
  that first look is trusted.
- **Two outcomes.** If the two recipient files still agree, confirming
  records the new list as reviewed and the question stops. If they
  *disagree* (the signature of someone editing `.age-recipients` alone),
  confirming lets this one write through but records nothing, so
  `recipient verify` keeps failing and the next write asks again.
- **Unlock and sync only warn.** Unlocking a vault, and a `sync`, `pull`
  or `push` that brings in a changed list, print a one-line warning and
  carry on. The blocking question is asked before the next write that
  encrypts something, which is where it can be acted on.
- **Deleting doesn't ask.** `rm` encrypts nothing, so it isn't gated.

Pass the global `--yes` to answer this one question *yes* in scripts and
CI, where there's nobody to confirm:

```
gage --yes insert Token --value-stdin < token.txt
```

`--yes` answers **only** this question. It is deliberately not a general
"assume yes": removing this device's own key and resolving a sync conflict
are different decisions, and a flag that meant "yes to whatever you were
going to ask" would answer them too. It also can't turn a mismatched
change into a reviewed one, because that rule is applied inside the
library from `verify`, not from who answered. It isn't listed in any
command's `--help`.

`--yes` is accepted in a [scripted session](session-mode.md#yes-in-a-script)
(`gage --script deploy.gage --yes`, or `--stdin`), which is the CI case it
exists for: it answers the question for every write the script makes. It
is refused in an *interactive* session: a human is sitting right there,
and `gage --yes` entering a session would silently approve every recipient
change for as long as the session lasted.

## Sharing an entry with someone else

Recipients are vault-wide. To give someone a subset of your secrets, put
them in a different vault and move or copy entries there; see
[Sharing entries between vaults](sharing.md).
