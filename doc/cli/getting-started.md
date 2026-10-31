# Getting started

## Installing

Install `gage` by any of the channels in the README's
[Install section](../../README.md#install), then check it:

```
$ gage --version
```

To build from a checkout instead, see [CONTRIBUTING.md](../../CONTRIBUTING.md).

## Create your first vault

```
$ gage init personal
gage: creating identity "laptop-1" for vault "personal".
gage: this passphrase protects this device's private key. It cannot be recovered or reset.
gage: it stays on this device and is never sent to anyone.
Choose a passphrase:
Confirm passphrase:
```

This creates a new vault named `personal`, generates a decryption identity
for the current device (defaulting to the passphrase method, so you are
prompted to set one), and registers the vault in your global config as the
current default. By default the vault's files live under
`$GAGE_DATA/vaults/personal` (see [Configuration](configuration.md) for
exactly where that is on your OS). Pass `--dir PATH` to put it somewhere
else.

`init` then generates an **offline recovery key** and shows it once, as text
and as a QR code:

```
gage: "personal" now has a recovery key. This is the only time it will be shown.

    AGE-SECRET-KEY-1...

Value for "the last 6 characters of the recovery key, to confirm you have saved it":
```

Write it down and keep it somewhere safe and offline. `init` asks you to type
the key's last six characters back before it finishes. If this device's
identity file or passphrase is ever lost, `gage recovery enroll` uses the key
to get you back in, and `gage` keeps no copy. See
[Storing your recovery key](identities-and-recipients.md#storing-your-recovery-key)
for what to do (and not do) with it. To skip it, pass `--no-recovery-key`;
`--recovery-key-out FILE` writes it to a file instead of showing it.

The vault starts with no remote. If you want it to sync to a git host later,
see [Git remotes and authentication](git-and-auth.md), or create it with a
remote up front:

```
$ gage init personal --remote https://github.com/you/personal-vault.git
```

## Store a secret

```
$ gage insert github-token
Enter passphrase for vault "personal" (device laptop-1):
Value for "github-token":
gage: inserted "github-token" (363da4eb-279f-4a40-ae06-aa76c1cac099)
```

`insert` unlocks the vault, then prompts for the entry's value, masked like a
passphrase. You can also pipe a value in or open an editor instead; see
[Entries](entries.md).

## Read it back

```
$ gage show github-token
Enter passphrase for vault "personal" (device laptop-1):
ghp_xxxxxxxxxxxxxxxxxxxx
```

`show` with no flags prints just the entry's `value` and nothing else, so it
composes cleanly with pipes and paste. (The passphrase prompt goes to the
terminal, not into a pipe.) Add `-c` to copy it to the clipboard instead
(auto-clearing after a short timeout) or `-q` to render it as a scannable
terminal QR code.

## List what's in the vault

```
$ gage ls
github-token  363da4eb  2026-09-01  2026-09-01  laptop-1
```

`ls` prints plain rows. Add `-H` for a labelled table:

```
$ gage ls -H
title        | id       | created at | updated at | updated by
-------------+----------+------------+------------+-----------
github-token | 363da4eb | 2026-09-01 | 2026-09-01 | laptop-1
```

## A faster way to work: session mode

Every command above unlocked the vault, did one thing, and dropped the key
from memory. That is fine for one-off lookups, but wasteful if you're doing
several things in a row. Run `gage` with no arguments to start an interactive
session instead:

```
$ gage
gage> use personal
Enter passphrase for vault "personal" (device laptop-1):
personal🔓 gage> insert wifi-password
Value for "wifi-password":
gage: inserted "wifi-password" (ab91fb9f-fd97-447f-9ff0-f508f654341f)
personal🔓 gage> show wifi-password
correcthorsebatterystaple
personal🔓 gage> exit
$
```

Inside a session you unlock a vault once and run as many commands against it
as you like, with no unlock cost. See [Session mode](session-mode.md) for the
full picture, including non-interactive automation.

## Add a second device

On the new machine, clone the vault and let `gage` set up an enrollment
request. Give the device its own name with `--device` (it defaults to the
hostname, and two devices can't share a name in one vault):

```
$ gage clone https://github.com/you/personal-vault.git --device laptop-2
gage: cloned "personal" to ~/.local/share/gage/vaults/personal
This device holds no identity for "personal", so it can't read anything here yet.
Set up an enrollment request now? [Y/n] y
gage: creating identity "laptop-2" for vault "personal".
...
Choose a passphrase:
Confirm passphrase:
gage: this device is "laptop-2", public key age1...

gage: enrollment request published (expires in 24h).

  Enrollment code:  GAGE-ZE3A-FWY7-XPV9-9XZE
```

Send that code to the machine that already has access (over chat, in
person, however you'd normally reach yourself) and approve it there:

```
$ gage sync
$ gage recipient approve --code GAGE-ZE3A-FWY7-XPV9-9XZE
gage: opened 1 pending request.

  device:  laptop-2
  pubkey:  age1...
  method:  passphrase
  created: 2026-09-01 10:15 PDT  (expires in 24h)

Add "laptop-2" as a recipient of "personal"? It will be able to read all 1 entry, including everything already in the vault. [y/N] y
Enter passphrase for vault "personal" (device laptop-1):
gage: approved "laptop-2" (age1...)
```

Back on the new device, `gage sync` and you can read the vault. The full
story, including denying requests and what the code does and doesn't
prove, is in [Adding a device](enrollment.md).

## Next steps

- Give another device (or another person) access: [Adding a device](enrollment.md) and [Identities and recipients](identities-and-recipients.md).
- Set up a remote so your vault syncs between machines: [Git remotes and authentication](git-and-auth.md).
- Learn the full entry format (descriptions, structured fields, generated passwords): [Entries](entries.md).
- Every command and flag in one place: [Command reference](command-reference.md).
