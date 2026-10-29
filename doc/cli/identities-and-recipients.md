# Identities and recipients

`gage` separates two concerns that are easy to conflate: **who can read a
vault** (recipients: a vault-wide list of public keys) and **how a
particular device proves it holds the matching private key** (identity:
decided independently by each device, never dictated by the vault).

## Identity: how this device proves it can decrypt

```
gage identity add    [--use NAME] [--method passphrase] [--device NAME]
gage identity enroll [--use NAME] [--device NAME] [--ttl DURATION]
gage identity list   [--use NAME]
```

`identity add` registers how *this device* holds its private key and
prints the resulting public key so it can be added as a recipient: either
by you, if you're bootstrapping a new vault, or by an existing recipient
adding you to theirs with `gage recipient add <pubkey> --device NAME`.

`identity enroll` is the same thing plus publishing. It registers the
identity *and* commits a sealed request to join into the vault itself,
then prints a short code to hand to an existing recipient. That's the path
most second devices should take; see [Adding a device](enrollment.md).
`identity add` is still the answer when this device can only read the
remote, not write to it, or for an air-gapped transfer.

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
`gage identity enroll` works for this too, and spares you carrying the key
by hand.

Both need another device that can already read the vault. That's why
`gage init` also makes a recovery key by default: a vault that depends on
one identity file surviving forever has no real recovery story. If you hold
that key, `gage recovery enroll` gets this device back in with nobody else's
help. See [Storing your recovery key](#storing-your-recovery-key).

## Storing your recovery key

```
gage recovery verify [--use NAME]
gage recovery enroll [--use NAME] [--device NAME] [--replaces DEVICE]
                     [--no-new-recovery-key | --recovery-key-out FILE]
gage recovery rotate [--use NAME] [--recovery-key-out FILE]
```

`gage init` registers a second recipient, `recovery-paper-key`, and shows
its private key once, as text and as a QR code. It then asks you to type the
key's last six characters back, so you know you have it. **That key is a
bare age secret: it is not encrypted, and anyone who has it can read every
secret in the vault.** Nothing protects it but where you keep it. `gage`
keeps no copy, so it can't be shown again. Confirm the copy you stored with
`gage recovery verify`, and read [Using it](#using-it) for what it can and
can't do.

**Do**

- Print it on a printer you trust, or write it out by hand. Keep the paper
  where you would keep a passport: a safe, a safe-deposit box, or a sealed
  envelope with someone you trust.
- Or keep it on an offline encrypted drive.
- Check it after storing it: `gage recovery verify` reads the key you paste
  and confirms it is still one of the vault's recipients.

**Don't**

- Put it in cloud-synced notes, a password manager that syncs, email,
  chat, or a photo or screenshot of the terminal.
- Print it on a shared or networked printer that keeps copies.
- Leave it in your terminal's scrollback. Clear it once you've stored it.

If you ran `init` with `--recovery-key-out FILE`, that file is the key. Move
it somewhere offline and delete it. `gage` refuses to write it inside a
vault or under `$GAGE_DATA`, because it deletes files in both.

The label `recovery-paper-key` is reserved. No device can take it:
`init --device`, `identity add`, `recipient add` and `recovery enroll` all
refuse it. `recovery rotate` replaces whatever holds the label, so a device
registered under it would be locked out by the next rotation.

All three `recovery` commands are one-shot only. They don't run inside a
[session](session-mode.md).

### Checking it

`gage recovery verify` asks for the key, reads it without echo, and checks
it against the vault's plaintext recipient list. It never unlocks anything
and asks for no passphrase, so it works on a machine whose identity file is
gone. It reports two separate things:

- **Whether the key decrypts this vault.** That's true of any recipient's
  key, so it exits `0` and names the label the key is registered under.
- **Whether `gage recovery enroll` will accept it.** That's true only of the
  key registered as `recovery-paper-key`. A key under any other label (a
  hand-made backup added with `recipient add`, say) really does decrypt the
  entries, but enroll refuses it, and verify says so.

A key that isn't a recipient exits `5` (`locked-or-auth`). Something that
isn't an age secret key at all exits `2` (`usage`). Verify reads only the
recipient list, so it can't tell you the entries themselves are undamaged.

### Using it

If this machine's identity file is gone, or you have forgotten its
passphrase, the recovery key gets you back in with one command:

```
gage recovery enroll [--device NAME] [--replaces DEVICE]
```

It asks you to paste the key, without echo. Then it generates a fresh
identity for this device and asks for a new passphrase. It admits that
identity to the vault and **retires the key you just used**, all in a
single commit, which it pushes. A replacement recovery key is generated and
shown once, exactly as `init` shows one, so the vault is never left without
a way back. `--recovery-key-out FILE` writes it to a file instead of the
screen. `--no-new-recovery-key` skips it and warns that the vault now has
none.

- **`--replaces DEVICE`** names the machine that was lost. Its key is
  removed in the same commit, so a stolen identity file stops opening
  anything new. A name that matches no recipient is refused (exit `3`)
  rather than ignored.
- **Rebuilding a machine under the name it already had**
  (`--device laptop-1 --replaces laptop-1`) is the one case where an
  existing local identity file is replaced rather than refused. That's what
  makes the forgotten-passphrase case work: the file is still on disk, just
  unopenable. Its old passphrase is never asked for. If anything fails
  before the commit, the old file is put back exactly as it was.
- Without `--replaces`, an identity file already on this machine for that
  device is refused (exit `2`), and so is a device name the vault already
  lists (exit `1`). Both are refused before you're asked for the key.
- It works straight after `gage clone` on a machine that has never held an
  identity for the vault.
- The key must be the one registered as `recovery-paper-key`. Run
  `gage recovery verify` first if you're not sure. A wrong key is refused
  (exit `5`) before you're asked for a new passphrase.
- Without a terminal, enroll refuses before asking anything, unless you pass
  `--recovery-key-out` or `--no-new-recovery-key`. It won't write an
  unencrypted key into a log.

The steps run in an order chosen so a failure never costs you the new key.
If something fails before the commit, nothing is committed and the identity
it generated is removed again. After the commit, the replacement key is
shown before this machine's registration is saved. If that save then fails,
the error names the command that finishes the job:
`gage identity add --device NAME`.

The recovery key is retired because it has just been typed into a
terminal, and it reads everything. Retiring it stops it opening anything
written from now on, but it still opens every version already in the
vault's git history.

**This is all the recovery key can do inside gage.** It is not an unlock
method: no other command accepts it, and there is no way to use it as a
day-to-day identity. Outside gage it still decrypts, because every entry is
an ordinary age file encrypted to all of the vault's recipients:

```
age -d -i recovery.key entries/<uuid>.age
```

That remains the way to read your secrets out if `gage` itself is
unavailable.

### If it leaks

Treat every secret in the vault as exposed to whoever holds the key. Act in
this order, from a device that can still read the vault:

1. **Replace the key:** `gage recovery rotate`. It generates a new recovery
   key, registers it as `recovery-paper-key`, and retires the old one,
   re-encrypting every entry in a single commit. It shows you the new key
   once, as `init` does.
2. **Rotate the secrets themselves.** Retiring the key stops it opening
   anything written from now on. But anyone who already read entries with
   it still has those values, and it still opens every version already in
   the vault's git history. Re-encrypting can't take that back.

If you have lost the only device that can read the vault, use
`gage recovery enroll` first, then rotate.

### Skipping it

`gage init --no-recovery-key` leaves this device's identity file as the
only way into the vault. If it, or its passphrase, is ever lost, the vault
is unreadable forever. `gage recovery rotate` adds a recovery key to a vault
that has none.

## Recipients: who can decrypt a vault

```
gage recipient add <pubkey> --device NAME [--use NAME]
gage recipient remove <pubkey-or-name> --reencrypt [--use NAME]
gage recipient list [--use NAME]
gage recipient verify [--use NAME] [--repair]

gage recipient pending [--use NAME]                       # enrollment requests
gage recipient approve [ID...] [--code CODE ...] [--device NAME] [--use NAME]
gage recipient deny <ID> [--use NAME]
```

The last three belong to the enrollment flow and have their own page:
[Adding a device](enrollment.md). The rest are the manual, hand-carried-key
path. Every one of them that changes the recipient list (`add`, `remove`,
`approve`) also deletes expired enrollment requests, in the same commit.

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
"assume yes": approving an enrollment request, removing this device's own
key and resolving a sync conflict are different decisions, and a flag that meant "yes to whatever you were
going to ask" would answer them too. It also can't turn a mismatched
change into a reviewed one, because that rule is applied inside the
library from `verify`, not from who answered. It isn't listed in any
command's `--help`.

`--yes` is accepted in a [scripted session](session-mode.md#--yes-in-a-script)
(`gage --script deploy.gage --yes`, or `--stdin`), which is the CI case it
exists for: it answers the question for every write the script makes. It
is refused in an *interactive* session: a human is sitting right there,
and `gage --yes` entering a session would silently approve every recipient
change for as long as the session lasted.

## Sharing an entry with someone else

Recipients are vault-wide. To give someone a subset of your secrets, put
them in a different vault and move or copy entries there; see
[Sharing entries between vaults](sharing.md).
