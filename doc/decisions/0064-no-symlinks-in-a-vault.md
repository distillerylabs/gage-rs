# ADR-0064: A vault contains only regular files; gage never follows or materializes a symlink

- Status: Accepted
- Origin: Hardening the sync merge writer and checkout ([#195](https://github.com/distillerylabs/gage-rs/issues/195))

## Context

Everything in a vault is untrusted input ([v2 format](../format/v2.md), §1):
a git writer with no vault key controls every tree the remote serves. gage
already treats tree paths as hostile — [`safe_join`](../../crates/gage/src/sync/gitrepo/mod.rs)
refuses `..`, absolute paths and NUL before resolving a path under the vault.

It did not account for the *type* of a tree entry. git can carry a symlink
(filemode `0o120000`) or a gitlink (`0o160000`) as a blob. Two paths let one
reach the filesystem:

- **Checkout.** `clone` and `fast_forward` go through libgit2, which on a
  symlink-capable filesystem lays a symlink blob down as a real, traversable
  symlink.
- **The merge writer.** gage's file-level merge ([ADR-0019](0019-file-merge.md))
  writes merged files itself, not through libgit2. Its writer opened each
  path in place and created parent directories with `create_dir_all`, so a
  symlink already on disk (left by a checkout, an earlier gage, or an external
  tool) was *followed*: a merged write could land on the symlink's target,
  outside the vault.

Together these turn "can write to the remote" into "can overwrite an
arbitrary file on a victim's machine during a routine pull" — the escalation
the threat model names as its most severe class. gage itself never writes a
symlink or gitlink into a vault, so nothing legitimate depends on them.

## Decision

**A vault contains only regular files.** gage never writes a symlink or
gitlink into one, never follows a symlink when writing, and never materializes
one on checkout.

- **The merge refuses a non-regular-file tree entry.** When the merge diff
  carries an added or modified entry whose filemode is a symlink or gitlink,
  the merge fails with `Error::UnsafeEntry` and writes nothing. (A merge is
  incremental, so refusing the one hostile entry is clearer than silently
  dropping it.)

- **The merge writer never follows a symlink.** It removes any existing object
  at a path (without following it) and recreates the file with `O_EXCL`, and
  it builds parent directories one level at a time, refusing to descend
  through a component that is not a real directory. So even a symlink already
  on disk is replaced, never written through.

- **Checkout neutralizes symlinks.** `clone`, `fast_forward` and
  `init_and_commit` set `core.symlinks=false` on the vault repository, so
  libgit2 materializes a committed symlink blob as a regular file holding the
  link text rather than a traversable symlink. Checkout neutralizes (rather
  than refuses) because a clone or fast-forward is bulk and first-contact, and
  refusing the whole operation over one entry would let a git writer wedge
  every clone and pull.

This is not a format change: the on-disk format gains nothing, the golden
`format-v2` fixture is unchanged (it never held a symlink), and the read side
already ignores non-regular files (`entry_ids`, enrollment's `pending_files`).
It tightens how hostile input is handled.

## Consequences

- A merged file is always written fresh at `0o600`; an overwrite no longer
  keeps a file's prior mode.
- A vault whose remote commits a symlink/gitlink entry still clones and
  fast-forwards (the entry becomes an inert regular file), but a *merge* that
  would introduce one is refused until the offending commit is dropped.
- On Windows, libgit2 already defaults `core.symlinks=false`; the merge
  writer's no-follow behavior is what carries the guarantee cross-platform.
