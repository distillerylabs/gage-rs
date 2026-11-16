# ADR-0084: The bindings find a vault from their roots and its name, not its recorded path

- Status: Accepted
- Origin: [#188](https://github.com/distillerylabs/gage-rs/issues/188), found during A1c ([#181](https://github.com/distillerylabs/gage-rs/issues/181))

## Context

A registration in the global config records the vault's working tree as an
absolute path (`vaults.<name>.path`). On a computer that path is the
person's choice, and nothing moves it.

iOS can move an app's container across an update or a restore, and Apple
advises against persisting absolute paths into it. The files come along,
but every recorded path still names the old container. The app passes
fresh `Roots` on every launch, and everything else gage keeps is found
from them: identities, locks, the trust cache and tokens are all keyed by
vault id under the roots. The recorded path is the one thing that goes stale.

Stale, it failed quietly. `Vault::from_registration` reads an unreadable
vault config as the case the registration's id covers, so `Vault.open`
succeeded and every read after it failed. `remove_vault` was worse: the
stale path isn't inside the current `$GAGE_DATA/vaults/`, so it reported
the clone `KeptOutsideVaultsDir` and left it on disk.

The bindings never let the app choose a path. `initVault` and `cloneVault`
always place a vault at `<data>/vaults/<name>`, so for them the recorded
path repeats what the roots and the name already say.

## Decision

**A `Layout` says where a registered vault is looked for.** `Recorded`
uses the registration's path, as the CLI always has. `Managed` uses
`$GAGE_DATA/vaults/<name>`, whatever the registration records.
`Layout::resolve` turns a registration into the one used, and both readers
of the path go through it: a frontend resolves before
`Vault::from_registration`, and `RemoveOptions::layout` tells
`Vault::remove_from_device`.

- **The bindings pass `Managed`,** in `Vault.open` and `remove_vault`.
  The CLI keeps `Recorded`.
- **The id check still runs** against the vault at the placed path, so a
  different vault sitting at `<data>/vaults/<name>` is still refused.
- **A name that isn't one plain path component keeps its recorded path.**
  The bindings check every name before registering it, so no name they
  made fails, and a name from a hand-edited config never becomes a path.
- **Removal compares placed paths.** Under `Managed` each name is its own
  directory, so no registration shares another's clone.

## Consequences

- **No format change.** The registration still records the absolute path,
  and the CLI still reads it. Recording a path relative to the data root
  would have changed what `vaults.<name>.path` means for every reader.
- **The recorded path is stale after a move and stays stale.** Nothing
  rewrites it, so a read never turns into a write. Nothing that resolves
  with `Managed` reads it.
- **Through the bindings, `remove_vault` can no longer report
  `KeptOutsideVaultsDir` or `KeptSharedWith`.** The variants stay, since
  the library's `remove_from_device` still returns them under `Recorded`.
- **A vault must sit at `<data>/vaults/<name>` to open through the
  bindings.** That is where they put every vault, so this only bites a
  registration made some other way.
