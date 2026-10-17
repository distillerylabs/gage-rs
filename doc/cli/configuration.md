# Configuration and file locations

`gage` follows the XDG Base Directory conventions on Linux and macOS, and
maps the same three roles onto native per-user directories on Windows:

| Role | Env var (wins on any OS if set) | Linux/macOS default | Windows default |
|---|---|---|---|
| Config | `XDG_CONFIG_HOME` | `~/.config` | `%APPDATA%` |
| Data (vaults, local identities) | `XDG_DATA_HOME` | `~/.local/share` | `%LOCALAPPDATA%` |
| State (trust cache, locks, tokens) | `XDG_STATE_HOME` | `~/.local/state` | `%LOCALAPPDATA%\state` |

This documentation writes `$GAGE_CONFIG`, `$GAGE_DATA` and `$GAGE_STATE`
for `<role root>/gage`: `$GAGE_CONFIG` is `~/.config/gage` on Linux and
macOS, and `%APPDATA%\gage` on Windows.

## The global config file

`$GAGE_CONFIG/config.toml` (mode `0600`) tracks the vaults this machine
knows. `gage init` creates it, and its directory, the first time a vault is
registered.

```toml
current = "personal"

[vaults.personal]
path = "/home/you/.local/share/gage/vaults/personal"
id = "208a0837-10f0-4024-b564-ffadc204c69f"   # the vault's own id, from its committed config
type = "git"
device = "laptop-1"       # this device's name in that vault
method = "passphrase"     # how THIS device unlocks it

[vaults.personal.git]
origin = "https://github.com/you/personal-vault.git"   # set by `gage git set-remote`

[vaults.work]
path = "/home/you/.local/share/gage/vaults/work"
id = "b41c9f02-7d55-4a18-9a30-6f2c8e1b7d44"
type = "git"
device = "laptop-1"
method = "passphrase"
```

`current` is the default vault (`gage vault set-default`). `device` and
`method` answer "who am I in that vault, and how do I unlock it": both are
local to this machine, and both can differ per vault.

`id` is the vault's own UUID, minted at `init` and committed inside the
vault, so every clone of a vault agrees on it. **Everything this device
stores about a vault is keyed by that id, not by the local name**, so two
unrelated vaults both called `personal` on one machine never share local
state. `gage` compares this copy with the vault's own before using a vault,
and refuses if they differ (see [Vaults](vaults.md#when-vault-info-refuses)).

Later phases add `pubkey` (the public key this device holds for the vault)
to each vault. An optional `[shell]` table holds this device's session
preferences: the prompt, the idle timeout, the history file and the
clipboard timeout (see [Session mode](session-mode.md#shell-settings)).

This file is small, plaintext, and safe to keep under your own dotfiles
management (`chezmoi`, `yadm`, and so on), which is why it holds nothing
secret and lives apart from the data below.

## Where things live

- **`$GAGE_DATA/vaults/<name>/`**: a vault's git repository, unless
  `init --dir` put it elsewhere.
- **`$GAGE_DATA/identities/<vault-id>/`**: this device's wrapped private
  keys, keyed by the vault's id. Deliberately outside the vault's git tree
  (it would be pushed) and outside `$GAGE_CONFIG` (which dotfile managers
  sync).
- **`$GAGE_STATE/locks/<vault-id>.lock`**: the per-vault write lock, held
  while a write or a sync command runs and released by the OS if the
  process dies.
- **`$GAGE_STATE/tokens/<host>`**: the HTTPS token `gage auth login`
  stored for a git host, mode `0600` in a `0700` directory. One per host,
  shared by every vault on it. Local device state, never synced: a token
  can always be had again by logging in. A token file anyone else can read
  is refused rather than used. See
  [Git remotes and authentication](git-and-auth.md).
- **`$GAGE_STATE/history`**: the command lines typed in sessions, mode
  `0600`, unless `[shell].history_file` names another path.
- **`$GAGE_STATE/<vault-id>/`**: the local trust cache: this device's
  record of the recipient list it last confirmed for the vault.
  `known-config.toml` is a verbatim copy of the vault's
  `.gage/config.toml`, and `trust.toml` holds the SHA-256 of
  `.age-recipients` with when it was recorded. Keyed by the vault's id like
  everything else here, and never committed or synced: it exists so that a
  recipient list changed by someone else is shown to you before anything is
  encrypted to it. See
  [the recipient-change confirmation](identities-and-recipients.md#the-recipient-change-confirmation).

## Inside a vault

A `git`-type vault's repository, as `gage init` creates it:

```
personal/                         # git repo root, on branch main
├── .gage/
│   └── config.toml               # vault-level config: id, type, method, recipients (committed, plaintext)
├── .age-recipients               # recipient public keys, one per line (committed, plaintext)
├── entries/                      # one encrypted file per entry (empty, so not yet committed)
├── .gitattributes                # marks .age-recipients and .gage/config.toml unmergeable
└── .gitignore                    # standard OS cruft (.DS_Store, Thumbs.db)
```

The one commit is authored by `gage <gage@localhost>` with the message
`gage: initialize vault`, whatever your own git configuration says.
`entries/` is mode `0700` and `.gage/` `0750` on Unix; the committed files
are `0644`.

`.gage/config.toml` looks like this:

```toml
[vault]
name = "personal"
id = "208a0837-10f0-4024-b564-ffadc204c69f"
type = "git"
format_version = 2
created = "2026-10-12"

[method]
default = "passphrase"

[[recipients]]
device = "laptop-1"
pubkey = "age18srydkf39n9sx40xtut6ry89fs22zetfng7gxqjdkkz0asjv9qnsjppaqj"
```

`created` is the UTC date the vault was made. `[method].default` is a
*suggestion* for devices joining later, not a constraint: each device's
actual method is the local-only `method` in your global config, and is
never committed. A recipient carries only a device label and a public key;
which method it unlocks with is never recorded where other readers can see
it.

`.age-recipients` stays flat at the vault root, one key per line with
nothing else, so the stock `age` CLI can use it directly
(`age -R .age-recipients -e file`). `.gitattributes` marks both
recipient-defining files unmergeable, so two devices that each change the
recipient list produce a conflict for a human to resolve rather than a
silent union of two lists neither of them reviewed.

## Environment variables

| Variable | What it does |
|---|---|
| `GAGE_PASSPHRASE` | Supplies the passphrase that opens an *existing* identity under `gage --script FILE` or `gage --stdin`, and only there: an ordinary one-shot command ignores it and prompts. It answers once, so a wrong value fails without a retry. It is never used to create a key (`init`, `identity add`). See [Session mode](session-mode.md#where-the-passphrase-comes-from). |
