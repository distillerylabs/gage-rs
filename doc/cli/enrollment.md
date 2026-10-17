# Adding a device: enrollment

The other way to get a second device into a vault is to hand-carry a public
key. You run `gage identity add` on the new machine, copy the key it prints
into a chat window, and someone runs `gage recipient add` with it. That
path is still supported (see
[Identities and recipients](identities-and-recipients.md)). But it asks the
two people involved to check a 62-character string by eye, which nobody
actually does.

**Enrollment inverts it.** The joining device publishes a *sealed request*
into the vault's own git repository and prints a short code. The code, not
the public key, is what travels out-of-band. An existing recipient runs
`gage recipient approve --code ...`. The code both finds the request and
proves it came from whoever gave them the code.

```
gage identity enroll [--use NAME] [--device NAME] [--ttl DURATION]
gage recipient pending [--use NAME]
gage recipient approve [ID...] [--code CODE ...] [--use NAME] [--device NAME]
gage recipient deny <ID> [--use NAME]
```

## The joining device

When `gage clone` finds that this machine holds no identity for the vault
it just cloned, it offers to enroll, on a terminal:

```
$ gage clone https://github.com/you/personal-vault.git
gage: cloned "personal" to ~/.local/share/gage/vaults/personal
This device holds no identity for "personal", so it can't read anything here yet.
Set up an enrollment request now? [Y/n] y
```

This is the one yes/no question in `gage` that defaults to **yes**. Every
other confirmation defaults to no. It answers a problem `clone` just
reported; it isn't an action you might not want.

There is deliberately **no `--enroll` flag**. `clone` already detects and
reports this exact condition, so a flag that opts into acting on it would
carry no information. And enrollment commits and pushes. Quietly turning
"fetch a copy" into "fetch a copy and publish a request naming this
machine" would widen what `clone` does beyond what its name suggests.

Answering `n`, or cloning with nobody at a terminal to answer, prints the
manual path instead and exits `0`:

```
gage: this device ("desktop") has no identity for "demo", so it cannot decrypt anything in it yet.
gage: run `gage identity enroll` here to generate a key and publish a request to join,
gage: then give the code it prints to someone who can already read the vault.
gage: or, if this device cannot write to the remote: run `gage identity add` here and
gage: have someone with access add the printed key with `gage recipient add`.
```

`gage identity enroll` does the same thing for a vault you've already
cloned. Use it when you cloned first and decided to join later, or
answered `n` above:

```
$ gage identity enroll --use personal
gage: creating identity "laptop-1" for vault "personal".
gage: this passphrase protects this device's private key. It cannot be recovered or reset.
gage: it stays on this device and is never sent to anyone.
Choose a passphrase:
Confirm passphrase:
gage: this device is "laptop-1", public key age1hs5xpch6r8vxwvn7v2zn7va5td2her8lzethz68edtr5zqlnsy9qjz5a9r

gage: enrollment request published (expires in 24h).

  Enrollment code:  GAGE-EQ7M-ZX5R-42MJ-E5FV

gage: this code is safe to send and is not your passphrase. Give it to someone who
gage: can already read "personal", over a channel where they can tell it came from
gage: you — not through this vault's remote. They run:
gage:   gage recipient approve --code GAGE-EQ7M-ZX5R-42MJ-E5FV

gage: then run `gage sync` here.
```

`identity enroll` **is** `identity add` plus publishing. It creates or
reuses a key the same way, and differs only in what happens once the key
exists. A device that already holds a wrapped identity file for this vault
reuses that key rather than generating a second one and orphaning the
first. That is what makes the command safe to re-run after a failed push.

The two paths prompt differently. Creating a key asks for a new passphrase
twice. Reusing one asks once, for that key's existing passphrase, because
the request has to name the public key and only the private key can
produce it.

### Which secret is which

Two different secrets appear a few lines apart on one screen. Confusing
them is the expensive mistake this flow is designed against:

| | Chosen by | Leaves the machine? | Lifetime |
|---|---|---|---|
| **Identity passphrase** | you | never | forever: you type it at every unlock |
| **Enrollment code** | `gage` | yes, that's its job | 24 hours by default |

So each one is labelled where it's used. The passphrase prompt says the
answer stays on this device. The code is printed with an explicit "safe
to send and is not your passphrase". The failure being designed against
is someone pasting their identity passphrase into a chat window because
they took it for the code, or typing the code at their next unlock prompt
and concluding `gage` is broken.

A third secret lives on the *other* device: the approver's own identity
passphrase. Approval asks for it because re-encrypting the vault needs
their key. It has nothing to do with either of the above.

### The code

The code has the form `GAGE-XXXX-XXXX-XXXX-XXXX`: 16 characters of
[Crockford base32](https://www.crockford.com/base32.html), 80 bits from
the operating system's random number generator.

- The alphabet leaves out `I`, `L`, `O` and `U`. If you type `I`, `L` or
  `O` anyway, they are read as `1`, `1` and `0` instead of being refused,
  because a code containing one is a misreading of a code `gage`
  generated.
- Lowercase, spaces instead of hyphens, and a missing `GAGE-` prefix are
  all accepted. Retype it however it arrived.

The code's entropy is what protects the request, not the key derivation
behind it. The sealed request sits in a git repository every reader of the
vault can fetch, so anyone can try codes against it offline, as many times
as they like. That is also why `gage` generates the code instead of
letting you choose one.

### `--ttl`

A request lives 24 hours by default. `--ttl` changes that, up to a hard
ceiling of 7 days (`--ttl 2h`, `--ttl 168h`). The ceiling does two jobs.
It limits what you can ask for, and it limits what `gage` will *honor*
when it reads a request back. A request claiming an expiry more than
7 days out is treated as already expired, so nobody can park a permanent
file in the tree.

### Enrollment needs the network, and fails if it can't reach it

This is the one place `gage` drops its usual "warn and carry on" over a
network failure. `identity enroll` fetches and fast-forwards before it
commits. It **fails outright** if the remote can't be reached or the two
histories have diverged. That happens before it generates a key, prompts
for anything, or writes anything. A read is still worth something offline.
An enrollment request that was never published is just a private key, a
local commit, and a code nobody can use.

It also needs git **write** access. A token that can read but not write is
only found out at the push. The error then says the identity was created
and the request committed locally but not published, and names
`gage auth login`. A device without write access uses the manual path
(`identity add` here, `recipient add` there). That path is also the answer
for an air-gapped transfer.

### When enrollment does nothing

- **This device's key is already a recipient.** This is reported as a
  success, with nothing generated and nothing published. It is checked by
  public key, not device name, so it also holds for a device an approver
  relabeled.
- **The device name already labels a *different* recipient.** This is
  refused before any key is generated, and the message names `--device` as
  the way out. Two machines whose hostnames normalize to the same name end
  up here.

## The approving device

`recipient pending` lists what's waiting. It reads only the request's
*filename*, so it needs no unlock, no code, and no walk through history:

```
$ gage recipient pending --use personal
gage: 1 pending enrollment request for "personal":

  931b0148   expires 2026-09-12 22:25 PDT  (in 24h)

gage: nothing here can decrypt anything until it is approved.
gage: run `gage recipient approve --code <code>` with a code you received out-of-band.
```

Expiry times are shown in your local time zone.

Device names are deliberately left out: they're under the seal, and
listing them would tell every reader of the vault who is setting up a new
machine before anyone approved anything. The cost is that an approver with
several pending requests can't tell them apart at a glance. That's the
right trade.

`pending` does **not** fetch. Both `pending` and `approve` read your local
copy of the vault, so a request published a minute ago only shows up after
a `gage pull` or `gage sync`. `approve` does fetch under its own write lock
once you've confirmed, but by then it's too late to have listed anything.

Then approve:

```
$ gage recipient approve --code GAGE-EQ7M-ZX5R-42MJ-E5FV
gage: opened 1 pending request.

  device:  laptop-1
  pubkey:  age1hs5xpch6r8vxwvn7v2zn7va5td2her8lzethz68edtr5zqlnsy9qjz5a9r
  method:  passphrase
  created: 2026-09-11 22:25 PDT  (expires in 24h)

Add "laptop-1" as a recipient of "personal"? It will be able to read all 1 entry, including everything already in the vault. [y/N] y
Enter passphrase for vault "personal" (device laptop):
gage: approved "laptop-1" (age1hs5xpch6r8vxwvn7v2zn7va5td2her8lzethz68edtr5zqlnsy9qjz5a9r)
gage: 1 recipient added, 1 entry re-encrypted, 1 request cleared; committed locally as 2e3e08e28bd9
```

Opening the request with the code also finds it, so the approver never has
to know or type an ID.

- A positional `ID` narrows the search when you want to be explicit.
- `--code` can be repeated, to approve several devices in one commit.
- With no `--code` at all, `gage` asks for one (masked, like a passphrase)
  and asks again after a wrong one. A bare Enter cancels. A code typed on
  the command line lands in your shell's history, and in a session's
  history file; the prompt records nothing.

### The order things happen in

1. Open the request.
2. Check for a device-name collision.
3. Show what the request claims.
4. Ask the `[y/N]` question.
5. *Then* unlock.
6. Then write.

**The unlock comes late on purpose.** A wrong code, an expired or
malformed request, a request for another vault, a name collision or a "no"
all cost you no passphrase. In a session where the vault is already
unlocked there's no passphrase prompt at all. A session vault that isn't
unlocked yet is unlocked once, after the "yes", and stays unlocked.

The write adds the recipients, re-encrypts every entry, and deletes the
approved request files, all in **one commit**, or none of it happens.
Before it writes, it fetches and fast-forwards under its own write lock.
An approval rewrites every entry, so one made on top of an out-of-date copy
would conflict on every one of them.

- If the histories have diverged, it refuses before writing anything and
  points you to `gage sync`.
- If the remote can't be reached, it warns and carries on, because an
  approval that lands locally is real work.
- If the push afterwards fails, it says the approval is committed locally
  but hasn't taken effect for anyone else: the request is still pending on
  the remote and the joining device is still locked out until you run
  `gage push`.

What gets written is what was verified **under the lock**, not what was
shown on screen. Each request is opened again with its code once the lock
is held. If the file is gone, or no longer matches what you were shown
(another device approved or denied it in the meantime, say), nothing is
written. `gage` reports that the request is no longer pending, naming that
likely cause, and exits `not-found` (`3`).

`--yes` does **not** answer the "add this device?" question. That flag
answers the [recipient-change
confirmation](identities-and-recipients.md#the-recipient-change-confirmation)
and nothing else. Letting a device into a vault is exactly the decision a
run nobody is watching must not make by default. With nobody to ask (a
script, CI), approval fails with `locked-or-auth` (`5`) and the request
stays pending.

The two questions really are separate. If another device changed the
recipient list since you last confirmed it, approval asks the
recipient-change question too, after the unlock. Saying no to it leaves
the request pending, with nothing committed.

### Approval always re-encrypts

There is no `--reencrypt` flag on `approve`. Approval re-encrypts every
entry, every time, for the same reason
[`recipient add`](identities-and-recipients.md#recipients-who-can-decrypt-a-vault)
does. A recipient who can read only part of a vault isn't an access level
anyone chose. It's invisible in the interface, since the dividing line is
"written before or after you were approved". And it spreads: a partially
admitted device can neither repair its own access nor grant full access to
anyone else.

The other side of that rule: **you can only approve if you can read every
entry yourself.** An approver who can't is refused with a count of how many
entries they can't read, instead of failing halfway through the write on an
opaque entry id. Because that check has to decrypt, it comes after the
unlock. This is the one refusal that costs a passphrase.

### `--device`

```
gage recipient approve <ID> --code CODE --device other-laptop
```

This records the approved key under a name the approver chooses, instead of
the one the request claims. It exists for when the claimed name now
collides with an existing recipient. Without it, such a request could never
be approved. It's the approver's call: the code authenticated the *key*,
not the label.

`--device` is only accepted when the run opens exactly one request, because
one flag can't say which of several it relabels. Pass an ID to narrow a
run that opens several; otherwise `gage` refuses rather than guessing. An
invalid name is refused before anything is opened or asked.

### Denying

```
$ gage recipient deny 931b0148
gage: removed pending request 931b0148 from "personal". Nothing was granted; nothing to re-encrypt.
```

Denying needs no code and no unlock. Refusing access doesn't require
proving anything, it changes no recipient list, and anyone with git write
access could delete the file directly anyway. It still takes the vault
write lock and commits like any other write. It warns if the push fails,
because a deny that never reached the remote leaves the request live for
every other device.

### Addressing a request by ID

`approve <ID>` and `deny <ID>` match against the **UUID part of the
filename only**, never the expiry, in the same order
[entry queries](addressing-entries.md#enrollment-request-ids-reuse-the-same-order)
use:

1. Exact UUID (all 36 characters).
2. Part of the UUID. The eight-character prefix the listing shows is this
   case, and it's what people actually type.
3. More than one match: `gage` lists the candidates and exits `ambiguous`
   (`4`) rather than guessing. This matters more here than for a read,
   because `deny` deletes.
4. No match: `not-found` (`3`).

There is no matching on device name, because the name is sealed. That is
the whole point of the filename scheme. Request ids aren't offered by Tab
completion in a session either: nothing can be completed without reading
`pending/`, and the listing is where ids come from.

## What a pending request is, on disk

```
myvault/
└── .gage/
    └── pending/
        └── <request-uuid>-<expires-epoch>.age
```

The first enroll creates the directory; a vault that has never had a
request doesn't have one. Each file is an age file encrypted **to the
code**, not to any recipient. It holds the device name, public key,
method, request id, vault id, and expiry.

The property everything rests on is that **`pending/` is inert**. Nothing
in it can decrypt anything. A request grants access only once an approver
commits the recipient change. A hostile git writer can add files there and
achieve nothing but noise. That is why:

- The expiry is in the filename, so listing and pruning need no unlock, no
  code, and no walk through history.
- `.gitattributes` deliberately does **not** mark `pending/` as
  unmergeable, unlike `.age-recipients` and `.gage/config.toml`. Two
  devices enrolling at once write two differently named files that merge
  cleanly, and a union of inert files is harmless.
- Commit messages name no device (`gage: enrollment request`), so someone
  reading the repository learns nothing about who is joining, or who was
  turned away.
- A file whose name doesn't parse is left alone, never deleted. By
  definition it isn't something `gage` wrote.

## Expiry and pruning

- Requests live 24 hours by default, 7 days at most.
- **Approval checks the sealed expiry** and refuses an expired request
  whatever the filename or the listing said. That is the only expiry check
  that matters for security.
- **Approval deletes the request file in the same commit** as the
  recipient change. A request can only be used once, so there is nothing
  left to replay.
- Pruning reads the filename's expiry. It rides along with a write that was
  happening anyway, rather than taking the lock just to tidy up. Every
  recipient write prunes in its own commit: `recipient add`, `recipient
  remove`, `approve` and `deny`.
- A request whose sealed expiry is earlier than its creation time is
  reported as **clock skew** on the requesting device, not as an expired
  request. The two have nothing in common as fixes.

## Limits

A run that supplies a code without naming an ID tries it against at most
**32** live requests. Beyond that it refuses and tells you to name an ID,
with the exact command to run. An ID is resolved from the filename and
costs one attempt per code, however full the directory is. Naming every id
yourself gets the same refusal: the limit follows how many requests the
run tries, not whether you typed an ID.

The limit is on *work*, not on how many devices may enroll. The count comes
from a directory any git writer can fill, and each attempt is a slow
key-derivation run. Expired requests don't count toward it.
`recipient pending` and `recipient deny` open nothing, so a stuffed
directory never stops you from inspecting or cleaning it.

Every `--code` you supply must open something. One that opens nothing
fails the whole run, before any confirmation and before anything is
committed. A mistyped code is far likelier than a deliberately extra one,
and partly approving a batch you confirmed as a whole would be the wrong
way to help.

## What can go wrong, and what it exits with

| Situation | Exit code |
|---|---|
| Code opens nothing (wrong, mistyped, or for another vault's request) | `5` locked-or-auth |
| Nobody to answer the `[y/N]` (no terminal); request stays pending | `5` locked-or-auth |
| Declined at the `[y/N]`, or a bare Enter at the code prompt; request stays pending | `1` conflict |
| Request expired, sealed id doesn't match its filename, or sealed for a different vault | `1` conflict |
| Request's sealed contents are malformed (never shown) | `1` conflict |
| Requesting device's clock is wrong (expiry earlier than creation) | `1` conflict |
| Device name already labels a different recipient (either side) | `1` conflict |
| Approver can't read every entry | `1` conflict |
| More than 32 live requests to try a code against | `1` conflict |
| Approval's fetch found a diverged history | `1` conflict |
| ID matches several pending requests | `4` ambiguous |
| ID matches none, or the request was resolved by another device meanwhile | `3` not-found |
| `approve --device` with an invalid name, or against several requests | `2` usage |
| `identity enroll` with no remote configured, or a bad `--ttl` | `2` usage |
| `identity enroll` can't reach the remote | `7` unreachable |
| `identity enroll` on a diverged history | `1` conflict |

See [Exit codes](exit-codes.md) for the full list.
