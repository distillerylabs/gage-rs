# ADR-0082: Removing a vault from a device refuses to lose what only it holds

- Status: Accepted
- Origin: [#229](https://github.com/distillerylabs/gage-rs/issues/229), from gage-ios's Vaults → Edit → Remove and its `reRegister` and `reclone` remedies

## Context

`gage vault remove` forgets a registration and nothing else. The person
chose where the vault lives, so deleting it is theirs to do. It offers to
delete the identity file only once its key is off the recipient list.

The mobile app chooses nothing: the bindings clone into `<data>/vaults/<name>`
inside its container, where the person can't reach the files. Removing a
vault there has to remove the clone too, and the layout is gage-rs's, so the
library does it. But the clone may be the only place some of the vault
exists. It may have no remote, or commits never pushed. And the identity
file may hold the vault's only key, so that no copy anywhere opens once it
is deleted.

## Decision

**`Vault::remove_from_device(dirs, name, options)` removes the clone, the
trust cache, the identity if asked, and then the registration.** The
bindings export it as `remove_vault`. The CLI's `vault remove` is unchanged.

- **The identity is the caller's choice** (`delete_identity`). A plain
  Remove deletes it. The `ReRegister` and `Reclone` remedies keep it, so
  cloning again reads, or asks to join, with the same key. When it is
  deleted while the key is still a recipient, the result says so
  (`still_a_recipient`): the device stays listed until someone who can write
  the vault removes it from a computer.
- **What exists only here is refused unless the caller says to discard it**
  (`discard_local_only`). That covers a vault with no remote, uncommitted
  changes, a remote never pushed to, commits the remote lacks as of the last
  fetch, and, when deleting the identity, a key that is the vault's only
  recipient. The refusal is `Conflict`, with `SetRemote` or `Push` where
  either fixes it, and removes nothing. Nothing is fetched to decide: the
  answer is as of the last sync.
- **Only gage's own directory is deleted.** The clone goes only when it sits
  directly inside `$GAGE_DATA/vaults/` and no other registration names the
  same path. Anything else is kept and reported, unread.
- **A clone gage can't read as a repository is removed without asking.** A
  missing or broken clone, the usual reason for `ReRegister`, has nothing
  left to lose.
- **The registration goes last, so a failure part way is retried by calling
  again.** The clone's `.git` goes first: once it has gone, what's left can't
  be read as a repository, so the retry isn't refused as uncommitted changes.
- **The saved token for the remote's host is left alone.** Other vaults may
  use the same host, and the bindings already have `remove_remote_token`.
- **The vault's write lock is held throughout,** so no write is half done
  underneath.

## Consequences

- "Pushed" means pushed as of the last fetch. A commit pushed from here and
  then lost from the remote (a force-push, a deleted repository) reads as
  safe.
- Deleting the identity of a vault with other recipients loses only this
  device's access, and its key stays on the list. Removing it there is a
  write, which needs an unlocked identity and a computer's `recipient remove`.
