# ADR-0055: The library is grouped by concern; the root re-exports what a frontend names

- Status: Accepted
- Origin: divergence plan D4a ([#141](https://github.com/distillerylabs/gage-rs/issues/141))

## Context

`crates/gage` had 35 flat `pub mod`s, one per Go package it was ported
from, and no re-exports. A second frontend (a GUI or TUI) would have had to
learn that list to find `Vault`, and `recipient` and `recipients` were two
modules for one concern. Nothing marked which items a frontend may rely on.

## Decision

**Areas.** `lib.rs` declares these public modules and no others
(`crates/gage/tests/module_layout.rs` pins the list):

| Area | Holds |
|---|---|
| `storage` | `atomicfile`, `config`, `paths`, `xdgpaths`, `vaultconfig`, `vaultlock`, `identityfile` |
| `crypto` | `agekey`, `crypt`, `identity`, `locker`, `memlock`, `generate` |
| `sync` | `gitrepo`, `remotesync`, `syncerr`, `syncresolve`, `remoteauth` |
| `trust` | `recipient`, `trustcache`, `orphan` |
| `entry` | the record, plus `history`, `search`, `movecopy`, `index` |
| `enrollment`, `recovery` | unchanged |
| root | `vault`, `session`, `prompter`, `error`, `devicename`; test-gated `gittest`, `testsupport` |

`recipients` (the `.age-recipients` file) is a private `file` submodule of
`trust::recipient`; its error is `RecipientsFileError`. `Vault`'s
device-identity methods live in a private `vault::identities` submodule.
`*_tests.rs` files are declared by the module they test.

**Re-exports.** The crate root re-exports what a frontend names in a
signature or a match, and nothing else: `Vault`, `CreateSpec`, `Session`,
`SessionConfig`, `Entry`, `Identity`, `VaultId`, `VaultType`,
`UnlockMethod`, `Prompter`, `Warning`, `ErrorKind`, `HasKind`, `Remedy`.
Everything else is reached through its area (`gage::sync::SyncReport`).

**Visibility.** An item is `pub` if `gage-cli`'s production code uses it or
a `pub` signature exposes it. A module only `gage-cli`'s tests use is `pub`
under `cfg(any(test, feature = "test-support"))` and `pub(crate)` otherwise
(`crypto::agekey`, `crypto::locker`, `storage::vaultlock`,
and the `.age-recipients` `load`/`save`). `memlock`, `search` and `index`
are crate-private.

## Non-goals

- **No handle types for `Vault`** (`vault.entries()`, `vault.recipients()`).
  Each `impl Vault` block already sits in the module for its concern;
  handles would change every call site for no behavior gain.
- Nothing about behavior, the on-disk format or the command line changes.

## Consequences

- Source-scan guards name files by their path under `src/`, `/`-separated
  on every platform (`CRYPT_SOURCE`), and still assert they found the file.
- Source paths in `doc/format/v2.md` and other docs follow the files.

`crypto::crypt` stays `pub`: its doctests pin the external contract (the
recipient's fields are private, an `Identity` can't be copied), and a
doctest sees the crate as an outside user without `test-support`.
