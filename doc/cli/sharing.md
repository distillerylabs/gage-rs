# Sharing entries between vaults

`gage` has no per-directory or per-entry sharing inside a vault: recipients
are always vault-wide. If you want a subset of entries visible to a
different set of people than the rest of a vault, the answer is a
**different vault**, and `mv` and `cp` are how an entry gets from one to
the other.

```
gage mv <query> --to-vault <name> [--use NAME]   # move
gage cp <query> --to-vault <name> [--use NAME]   # copy, keeping the original
```

Both decrypt the entry from its source vault (which has to be unlocked) and
re-encrypt it to the destination vault's recipients. The destination itself
does **not** need to be unlocked: encrypting *to* a set of public keys
never requires holding any of the matching private keys, so the destination
never asks for a passphrase, in a session or out of one. `mv` then removes
the entry from the source; `cp` leaves the original in place, so the same
secret ends up readable by both vaults' recipient sets.

```
$ gage init shared-family --method passphrase --recipient <family-member-pubkey>
$ gage mv family-wifi --to-vault shared-family
gage: moved "family-wifi" to shared-family (0c1d...)
```

- **`--to-vault` is required**, and names a registered vault. A name that
  isn't registered (exit `3`), or that is the source vault (exit `2`), is
  refused before anything is unlocked. A second name registered for the
  same vault counts as the source too.
- **The result is a new entry.** It gets a fresh id, fresh `created` and
  `updated` dates, and `updated_by` set to this device. Title, description,
  value and fields carry over. A title the destination already has is
  allowed (there is no `--force`): refusing would make sharing fail on the
  entry most likely to be shared twice.
- **The destination's recipient list is checked, not the source's.** The
  [recipient-change confirmation](identities-and-recipients.md#the-recipient-change-confirmation)
  runs against the *destination*, since that's the list the entry is about
  to be encrypted to. Declining (exit `1`) stops the whole operation with
  nothing written to either vault. The global `--yes` answers it, as on
  other writes.
- **One commit in each vault**, each named by that vault's entry id, and
  each pushed after it commits, like any write.
- **The order is fixed, so a crash can't lose an entry.** `mv` writes,
  commits and pushes in the destination first, and only then removes the
  entry from the source. If the process dies in between, the entry is in
  both vaults: a duplicate to delete, never a loss. Any uncommitted
  leftovers in either working tree are discarded (with a warning) before
  the next write, as for every write.
- **Locks.** Both commands take both vaults' write locks and hold them for
  the whole operation, acquired in the order of the vaults' ids, so two
  simultaneous moves between the same pair of vaults in opposite
  directions can't deadlock. A lock that stays contended for ten seconds
  fails with exit `1`.
- **In a session**, `mv` and `cp` use the vault you've unlocked (or
  `--use`) as the source. An ambiguous query goes through the picker, as for
  `rm`. The session's cached listing is updated for both vaults without
  re-decrypting anything.

## Why not finer-grained sharing?

An earlier design let recipients be overridden per subtree, so one vault
could mix a household `shared/` tree with a private `personal/` tree. It
works, but a separate vault per trust boundary removes the whole class of
problem instead of mitigating it: with per-directory recipients, *which
directory an entry sits in* becomes a privilege boundary in itself,
exactly the kind of thing someone with git write access could exploit by
moving a file into a directory they happen to hold keys for. With one
recipient list per vault, there's no finer-grained boundary to attack:
"does this person have write access to this vault's repository" is the
entire question, and it's one your git host already answers.

What you give up: if one vault needs three different partial-access
groups, that's three vaults instead of one vault with three subtrees. In
practice that's usually the right shape anyway: separate vaults get
separate remotes, separate access lists on the git host, and separate
audit trails, which is what you'd want for genuinely different trust
groups regardless of how `gage` stored things internally.
