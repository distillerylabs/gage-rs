# ADR-0021: Conflict resolution

- Status: Accepted
- Origin: Q-SYNC-CONFLICT, A16 (original project's register); Q-CONFLICT-PROMPTER ([entry](../archive/rust-port/open-questions.md#q-conflict-prompter))

## Context

"The entry conflicts" is one of five situations:

| Situation | Handling |
|---|---|
| Different entries changed | Merges cleanly; nothing to ask |
| Same entry changed | Conflict (binary); decrypt both, ask |
| Delete vs. modify | Conflict; decrypt the survivor, ask |
| **Recipient files changed** | Would merge cleanly; forced to conflict |
| Same title, different UUIDs | Not a conflict; the ambiguous-query resolver handles it |

The recipient-file row is a security hole: `.age-recipients` is one key per
line, so two devices adding different recipients merge cleanly into a list
neither wrote, every later write is encrypted to it, and the trust cache
sees nothing because no person made the change.

## Decision

1. `.gitattributes`, written by `init`, marks `.age-recipients` and
   `.gage/config.toml` `-merge`; gage's own merge ([ADR-0019](0019-file-merge.md))
   does not depend on it.
2. Menu: keep local / keep remote / keep both, plus skip and abort. `keep
   both` writes the losing version as a new entry under a fresh UUID, so
   resolving never destroys a secret.
3. History is a true merge commit with two parents (local first). No rebase:
   that would re-prompt per local commit. No squash: that would discard the
   per-write audit trail.
4. Unlock is lazy. Fast-forwards and merges touching different entries need
   no identity. `sync` unlocks only on reaching a conflict that needs
   plaintext, which is why the trust-cache check hooks `sync` directly.
5. Same title with different UUIDs is not a conflict; detecting it would
   force an unlock on every sync.
6. A frontend opts in to resolving through a default trait method,
   `Prompter::resolve_conflict`, returning `PromptError::NotInteractive`.
   An implementation that says nothing refuses. The terminal prompter
   refuses when not interactive, which is the `--script`, `--stdin` and CI
   path.

## Consequences

A real conflict is always sent to a human; divergence is never
auto-resolved.
