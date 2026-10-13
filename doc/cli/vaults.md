# Vaults

A vault is the unit of trust in `gage`: it has its own git repository, its
own list of recipients (who can decrypt it), and its own default decryption
method. You can have as many vaults as you want (`personal`, `work`,
`shared-family`), each with a different security posture. If you need a
different set of readers for a subset of secrets, that's a new vault, not a
subdirectory of an existing one.

> **Status.** gage-rs can create, list, inspect, re-point and forget
> vaults. Nothing in them is encrypted yet: identities, entries and sync
> arrive in later phases of the [port plan](../implementation/00_gage-cli/plans/rust-port/index.md).

## Creating a vault

```
gage init <name> --recipient PUBKEY [--recipient PUBKEY ...]
                 [--dir PATH] [--device NAME]
                 [--type git] [--method passphrase]
```

`init` creates the vault's directory, writes its committed config
(`.gage/config.toml`), its recipient list (`.age-recipients`), a
`.gitignore` and a `.gitattributes`, makes one git commit of all of it, and
registers the vault on this machine. It refuses, before touching anything:

- a name that's already registered (exit `1`, `conflict`);
- a target directory that exists and isn't empty, or a target that isn't a
  directory at all (exit `1`);
- an unknown `--type` or `--method`, an invalid `--device`, or a malformed
  `--recipient` (exit `2`, `usage`).

The options:

- **`--recipient`** is required for now and can be repeated. gage-rs can't
  generate this device's own key yet, so the first key you give is recorded
  as this device's (labelled with the device name) and any later ones as
  `recipient-2`, `recipient-3`, and so on. Repeating a key writes it again.
  A later release generates this device's key itself, and `--recipient`
  becomes additive, as it is in Go gage. A recipient is any well-formed
  `age1...` public key, including plugin recipients such as
  `age1yubikey1...`; an identity (`AGE-SECRET-KEY-1...`) is refused.
- **`--dir`**: without it, the vault is created at
  `$GAGE_DATA/vaults/<name>` (see [Configuration](configuration.md)).
- **`--device`** names this device as a recipient label. Without it, the
  name is your hostname, normalized: lowercased, cut at the first `.`,
  anything outside `a-z 0-9 . _ -` replaced with `-`. An explicit name must
  already be valid; it's refused rather than rewritten into a different
  one. If your hostname normalizes to nothing usable, `init` asks you to
  pass `--device`.
- **`--method`** sets the vault's *default* identity method, recorded in
  its committed config as a suggestion for devices joining later. Each
  device's actual method is local config and can differ. `passphrase` is
  the default and, today, the only option.
- **`--type`** is the vault's backing store. `git` is the default and the
  only option.

A vault starts local-only. `gage git set-remote` gives it a remote (see
below); Go gage's `init --remote` arrives with sync.

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
