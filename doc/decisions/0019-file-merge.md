# ADR-0019: gage's own file-level merge

- Status: Accepted
- Origin: Q-FILE-MERGE ([entry](../archive/rust-port/open-questions.md#q-file-merge))

## Context

`git2::merge_trees` can build the merge commit, but it merges text files
line by line. `.age-recipients` is text, so two devices that each add a
recipient would merge into a union neither wrote ([ADR-0021](0021-sync-conflict-resolution.md)).
The committed `.gitattributes` (`-merge`) binds a human running stock `git`
in the vault, not gage, so the guarantee cannot rest on it.

## Decision

gage computes a file-level three-way merge from the merge base and the tree
diffs base→ours and base→theirs. Never `merge_trees`.

- A path changed on one side takes that side's version.
- A path changed on both sides is a conflict, unless both hold the same blob.
- A deletion on both sides is taken.
- A deletion on one side with a modification on the other is a
  delete/modify conflict.
- Nothing is written to the working tree until commit; a conflict writes
  nothing. A partial failure rolls back and always reaches the hard reset.
- Tree paths come from whoever can push to the remote, so they go through
  `safe_join` before any filesystem write.
- `merge_remote` is `prepare_merge` followed by `PendingMerge::commit` with no
  choices, so the automatic and the resolving path cannot drift apart.
