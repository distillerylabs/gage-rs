# Vaults

A vault is the unit of trust in `gage`: it has its own git repository, its
own list of recipients (who can decrypt it), and its own default decryption
method. You can have as many vaults as you want (`personal`, `work`,
`shared-family`), each with a different security posture. If you need a
different set of readers for a subset of secrets, that's a new vault, not a
subdirectory of an existing one.

> **Status.** gage-rs can create a vault with this device's own
> passphrase-protected identity, and list, inspect, re-point and forget
> vaults. [Entries](entries.md) can be inserted, read, listed and deleted;
> sync arrives in a later phase of the [port plan](../implementation/00_gage-cli/plans/rust-port/index.md).

## Creating a vault

```
gage init <name> [--recipient PUBKEY ...]
                 [--dir PATH] [--device NAME]
                 [--type git] [--method passphrase]
```

`init` generates this device's identity for the new vault, then creates
the vault's directory, writes its committed config (`.gage/config.toml`),
its recipient list (`.age-recipients`), a `.gitignore` and a
`.gitattributes`, makes one git commit of all of it, and registers the
vault on this machine. It prints this device's name and public key: the
key is what you hand another device's `recipient add`.

Before asking for anything, it refuses:

- a name that's already registered (exit `1`, `conflict`);
- an unknown `--type` or `--method`, an invalid `--device`, or a malformed
  `--recipient` (exit `2`, `usage`).

A target directory that exists and isn't empty, or a target that isn't a
directory at all, is refused too (exit `1`), after the passphrase prompt.
The identity generated for it is removed again, so a failed `init` leaves
no key and no vault behind and can simply be retried.

### The passphrase

`init` asks you to choose a passphrase, twice: there is nothing to check a
new passphrase against, so a typo would otherwise only surface at the next
unlock. It protects this device's private key, stays on this device, and
**cannot be recovered or reset**. An empty passphrase is refused. The
prompts go to stderr, so they never end up in redirected output; when stdin
is a terminal, what you type isn't echoed. When stdin isn't a terminal
(a script), each answer is one line of stdin.

If no passphrase is given (end of input, or three attempts that never
matched), `init` exits `5` (`locked-or-auth`) having created nothing.

The [entry commands](entries.md) unlock the vault with the same
passphrase. A wrong one is re-asked, up to
three attempts in all, then exits `5`; a missing identity file is `5` too.
A damaged identity file is `1` (`conflict`) and is never re-asked, since no
passphrase can fix it.

### The identity file

The private key is stored, wrapped with your passphrase (age's scrypt, at
a work factor of 2^19), at
`$GAGE_DATA/identities/<vault-id>/<device>.age`: outside the vault, so it
is never committed or synced. The directory is `0700` and the file `0600`.
The file is an ordinary passphrase-encrypted age-keygen key file, so the
stock age tool can use it too (`age -d -i <device>.age ...`).

The directory is named by the vault's id, not its name; a plaintext
`vault-name.txt` beside the key says which vault it belongs to, for when
you back the file up by hand. Nothing reads it. Losing the identity file
loses this device's access to the vault: there is no copy, by design.

While a vault is unlocked, `gage` keeps the key in memory it asks the OS
not to swap out, and wipes it when it's done. Where the OS refuses (a low
`ulimit -l`, some containers), `gage` warns once and carries on. Core
dumps are disabled for the whole process; on Windows, which has no direct
equivalent, `gage` suppresses the crash dialog that would offer to write
one, which is a narrower guarantee.

### The options

- **`--recipient`** adds another recipient alongside this device's own key,
  for someone whose public key you already hold. It can be repeated; keys
  are written in the order given, after this device's, labelled
  `recipient-2`, `recipient-3`, and so on. A recipient is any well-formed
  `age1...` public key, including plugin recipients such as
  `age1yubikey1...`; an identity (`AGE-SECRET-KEY-1...`) is refused.
- **`--dir`**: without it, the vault is created at
  `$GAGE_DATA/vaults/<name>` (see [Configuration](configuration.md)).
- **`--device`** names this device: the label on its recipient entry and
  the name of its identity file. Without it, the name is your hostname,
  normalized: lowercased, cut at the first `.`, anything outside
  `a-z 0-9 . _ -` replaced with `-`. An explicit name must already be
  valid; it's refused rather than rewritten into a different one. If your
  hostname normalizes to nothing usable, `init` asks you to pass
  `--device`.
- **`--method`** sets the vault's *default* identity method, recorded in
  its committed config as a suggestion for devices joining later. Each
  device's actual method is local config and can differ. `passphrase` is
  the default and, today, the only option.
- **`--type`** is the vault's backing store. `git` is the default and the
  only option.

- **`--remote`** gives the vault an `origin` and publishes its first
  commit straight away. Without it the vault starts local-only, and
  `gage git set-remote` can give it a remote later (see below). See
  [Git remotes and authentication](git-and-auth.md).

`init` is one-shot only: it creates a vault rather than operating on one.

## Listing and inspecting vaults

```
gage vault list
gage vault info [<name>]
```

`vault list` prints every registered vault, sorted, with the default one
marked `*`.

`vault info` shows a vault's name, id, type, default method, recipient
count, and (for the `git` type) its remote and clean/dirty status. With no
name it reports on the default vault.

```
$ gage vault info
name: personal
id: 208a0837-10f0-4024-b564-ffadc204c69f
type: git
method: passphrase
recipients: 1
remote: (none)
status: clean
```

The **id** is a UUID minted at `init` and committed in the vault's own
`.gage/config.toml`, so every clone of a vault shares it. It's what `gage`
keys this device's local state by, which is why two unrelated vaults that
happen to be called `personal` on one machine never collide. See
[Configuration](configuration.md).

`status: dirty` means the vault's working tree has changes that aren't
committed, untracked files included.

### When `vault info` refuses

The committed `.gage/config.toml` is plaintext that anyone who can push to
the vault can edit, so `gage` checks it before using anything in it. These
all exit `1` (`conflict`), and nothing is reported from the file:

- **An unrecognized `format_version`.** A vault written by a newer `gage`
  asks you to upgrade `gage`. A vault written by an older one (or with no
  `format_version`) asks you to re-create it: format 2 added the vault id,
  and there is no id to invent for a vault that never had one.
- **An id or a recipient's device name that isn't safe.** Both become file
  names on this machine, so one that could escape its directory
  (`../../etc/x`, `/etc/passwd`, a separator) is refused, never cleaned up.
- **A registration that points at a different vault.** If the id this
  machine recorded at `init` doesn't match the id the vault at that path
  carries (a vault re-created at the same path, a registration edited by
  hand), `gage` refuses rather than letting either side win, and tells you
  to re-register it.

## Changing the default vault

```
gage vault set-default <name>
```

Changes which vault commands use when none is given, persistently, in your
global config. The first vault you register becomes the default
automatically.

## Setting a vault's remote

```
gage git set-remote <name> <url>
```

Sets (or changes) the vault repository's `origin`, and records it in your
global config, in one step so the two never drift apart. Nothing is
contacted. gage never shells out to `git`: remotes are set through the git
library it links.

## Removing a vault (locally)

```
gage vault remove <name>
```

This forgets the vault in your local config. It does **not** delete the
vault's directory, its git repository, or its remote: if you want the data
gone, delete those yourself. It reads nothing from the vault, so it works
even when the vault's files have already been moved or deleted. Removing
the default vault leaves no default until you set one.

Afterwards it may offer to delete this device's identity file for the
vault (the wrapped private key under `$GAGE_DATA/identities/`), but only
when that file is orphaned: the vault's recipient list, read fresh from
disk, no longer holds this device's **public key**. Keys are compared,
never labels, so a device renamed in the list still counts as listed. The
offer is always a `[y/N]` question, and a bare Enter keeps the file, so a
scripted `vault remove` keeps it by construction:

```
laptop-1's key is no longer a recipient of "personal". Delete this device's identity file for it?
  ~/.local/share/gage/identities/208a0837-.../laptop-1.age
This is the only copy of that private key, and deleting it cannot be undone. [y/N]
```

Every other case keeps the file and says why on stderr, so a key that
might still be the only way into a vault is never deleted quietly:

- the key is still a recipient, under any label (the message names it and
  how to remove it first, with `gage recipient remove ... --reencrypt`
  from another device)
- no public key is recorded for this device
- the registration points at a different vault than the one on disk
- the recipient list can't be read
- you declined

With no identity file, or no device or vault id recorded, there is
nothing to say. The trust cache for the vault is always deleted, as
before.
