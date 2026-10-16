# M12b — `cat` display, `ls -H`, `vault remove` cleanup

[← M12a](m12a-log-history.md) · [P6 plan](index.md) · [#54](https://github.com/distillerylabs/gage-rs/issues/54) · next: [M12c](m12c-clip-qr.md)

> **Recommended model: Sonnet ⚑.** The display work is pattern-following,
> and the cleanup is fully specified by Go's code after E0. Run an **Opus
> review of the never-delete rows** (task 4) before calling it done:
> deleting the wrong identity file destroys the only copy of a private
> key, and Go's first version (comparing device names) was wrong in both
> directions.

## Goal

Three display and housekeeping items that Go recorded under M12:

- **`cat` (#55).** The output is still the entry's full plaintext YAML,
  for scripting and piping, with three display-only changes:
  - an `id: <short id>` line right after `title`, which is not part of
    the stored entry
  - top-level keys aligned in one column, and each `fields` mapping's
    keys aligned independently
  - multi-line block values printed flush left instead of indented
  Input that isn't a mapping is printed unchanged.
- **`ls -H/--header` (#54).** An opt-in labelled table: a header row, a
  dash row broken with `+` at each column boundary, and `" | "`-delimited
  rows. The last column is never padded, and its dashes are sized from
  its label. An empty vault still prints nothing. **Plain `ls` is
  byte-for-byte unchanged**, and both shapes render identically in a
  session and one-shot.
- **`vault remove` orphan cleanup (#39 as revised by E0).** After forgetting
  the registration, offer to delete this device's identity file for that
  vault, but only when the vault's recipient list, read fresh from disk,
  no longer holds this device's **public key**. Always behind `confirm`,
  which defaults to no. Every other case keeps the file and says why on
  stderr:
  - no identity file, or no device or id recorded: nothing to say
  - no public key recorded for this device
  - the registration points at a different vault than the one on disk
  - the recipient list can't be read
  - the key is still a recipient, under any label
  - the human didn't confirm

## Depends on

- **P1 · M1:** `vault remove` (registration and trust cache) and the
  registration-versus-vault id check.
- **P2 · M3/M4:** the entry YAML writer, `cat`, `ls` and `render_ls`.
- **P3 · M7:** `ls` renders identically in both modes
  (`TestLsRendersIdenticallyInBothModes`).
- **P5 · M9a:** identity files, `identity add`, the recorded `pubkey`,
  and reading `.age-recipients`. **M10:** `vault remove` deleting the
  trust cache, which stays where it is in the order.

## Design references

- Design doc, "Vault lifecycle": `vault remove` forgets registration and
  never touches the store.
- Design doc, "Command reference": `cat` is "always full raw plaintext,
  for scripting/piping".
- Go `gage-cli-init-design` open questions: Q-ORPHAN-BY-NAME (why keys,
  not names).

## Go sources

- Go plan `gage-cli-design/m12-polish.md`: the `ls --header` decision and
  its four tests.
- Parity ledger rows:
  - §2 `a0920b0`, `0a1beeb` (#39)
  - §2 `f0e17db`, `f1e391f` (#55)
  - §2 `fb2f1e9` (#54)
  - §4 `cat`, `ls`, `vault_remove`
- Go sources at the pinned SHA:
  - `cmd/gage/entry.go`: `alignCatOutput`, `padKeyLines`, `writeLsRows`,
    `lsTable`, `lsColumns`
  - `cmd/gage/vault.go`: `newVaultRemoveCommand`, `removeOrphanedIdentity`,
    `checkRegistrationPointsAtThisVault`
- Test inventory, 10 rows: `cmd/gage/cat_test.go`.
- Inbound, 18 rows: 12 from P1 (`vault_lifecycle` 3, `vaultid_keying`
  9) and 6 from P2 (`entry` CLI: 2 `cat`, 4 `ls -H`). They are listed in
  the [P6 plan](index.md#inventory-accounting).

## Decisions (resolved)

- **Where `cat`'s alignment parses.** `crates/gage` exposes
  `entry::key_layout(&[u8]) -> Option<Vec<KeyLine>>`, built on
  `saphyr-parser`'s event spans (the crate Q-YAML-CRATE chose partly for
  this). Each `KeyLine` carries the key, its 0-based line, whether it is
  written bare, the line where whatever follows begins, the content of a
  multi-line `|` value, and, for the top-level `fields` key, its children.
  `None` means not a mapping with scalar keys, which `cat` prints
  unchanged. Spans were confirmed to give the right line for plain and
  quoted keys. `gage-cli` (`commands/catfmt.rs`) does the padding, the id
  line and the dedent. Two small departures from Go, both in its favour:
  the id line goes after the *first key's whole value* rather than after
  line 0, so a multi-line title can't split; and a key whose line doesn't
  hold its text where the parse says is left unpadded.
- **Where the orphan check lives** ([Q-ORPHAN-CLEANUP](../open-questions.md#q-orphan-cleanup)).
  `gage::orphan::assess(dirs, name, &VaultEntry) -> OrphanCheck` with
  `Nothing`, `Keep { path, reason: KeepReason }` and
  `Offer { path, device }`. `KeepReason` is `NoPublicKeyRecorded`,
  `RegistrationDoesNotMatch`, `RecipientsUnreadable` or `StillARecipient`,
  carrying the label or the underlying error where Go's message names one.
  `gage-cli` renders `Keep` on stderr, asks `confirm` on `Offer` (naming
  the file, saying it can't be undone) and deletes through
  `identityfile::remove`. A `confirm` that fails is a no.
- **Order inside `vault remove`.** As Go: forget the registration, write
  global config, delete the trust cache, then the orphan check. Nothing in
  the check can fail the command.
- **The still-listed keep message** names the real command:
  ``gage recipient remove <label> --reencrypt`` (`--reencrypt` is
  required by `recipient remove`, P5 · M9b).
- **`ls -H` is not in the library.** `render_ls_table` sits beside
  `render_ls` in `gage-cli`, over one shared `ls_cells`.

## Tasks

Write each task's tests first, at task granularity.

### 1. `cat` — 12 rows (10 + 2 inbound)

- [x] The library's key-layout reader, then the CLI's id line, padding
      and dedent, used by `cat` in both modes.
- [x] Rows (`cat_test`):
  - `TestAlignCatOutputInsertsIDLineAfterTitle`
  - `TestAlignCatOutputAlignsTopLevelColumn`
  - `TestAlignCatOutputAlignsFieldsIndependently`
  - `TestAlignCatOutputRoundTrips`: stripping the id line gives YAML
    that parses back to the same entry.
  - `TestAlignCatOutputDedentsMultilineValue`
  - `TestAlignCatOutputDedentsFieldsSubValue`
  - `TestAlignCatOutputDedentsValueFollowedByMoreKeys`
  - `TestAlignCatOutputSkipsAlignmentForQuotedKeys`
  - `TestAlignCatOutputLeavesMultilineValueContentAlone`
  - `TestCatCommandPrintsIDLineThroughTheCLI`
- [x] Inbound (`entry` CLI, from P2):
  - `TestAlignCatOutputLeavesNonMappingInputAlone`
  - `TestPadKeyLinesWithNoKeysIsANoOp`

### 2. `ls -H/--header` — 4 inbound rows

- [x] `-H/--header` on `ls`; the table shape from Go's `lsTable`.
- [x] Rows (`entry` CLI, from P2):
  - `TestLsDefaultOutputUnchangedByHeaderFeature`: no `|`, one line per
    entry, and the same bytes as before this milestone.
  - `TestLsHeaderPrintsLabelledPipeDelimitedTable`: `-H` and `--header`.
  - `TestLsHeaderEmptyVaultNoOutput`
  - `TestLsHeaderRendersIdenticallyInBothModes`

### 3. Orphan check in the library

- [x] `OrphanCheck` and the function that produces it, with in-crate
      unit tests for each variant before any CLI test. These are not
      Go rows; they are where each `KeepReason` is proven without a CLI
      in the way.

### 4. `vault remove` cleanup — 12 inbound rows

- [x] Wire the check into `vault remove` after the trust-cache deletion.
- [x] Rows (`vault_lifecycle`, from P1):
  - `TestVaultRemoveDeletesAnOrphanedIdentityFile`: confirmed, the file
    is gone.
  - `TestVaultRemoveKeepsIdentityFileStillListedAsRecipient` ⚑
  - `TestVaultRemoveKeepsIdentityFileWhenRecipientsUnreadable` ⚑: the
    store deleted; kept, and stderr says it "could not confirm".
- [x] Rows (`vaultid_keying`, from P1):
  - `TestNonInteractiveVaultRemoveNeverDeletesAnIdentity` ⚑: through the
    real terminal prompter with no terminal, the answer is no.
  - `TestVaultRemoveNeverDeletesWithoutConfirming` ⚑
  - `TestVaultRemoveIdentifiesAStaleKeyWhoseNameNowLabelsAnother`: the
    device's label now belongs to another key, so the offer is made.
  - `TestVaultRemoveKeepsARelabelledRecipientsKey` ⚑: the key is listed
    under a different label, so it is kept.
  - `TestVaultRemoveKeepsTheFileWhenNoPubkeyIsRecorded` ⚑
  - `TestVaultRemoveNeverDeletesAKeyBelongingToADifferentVault` ⚑: the
    registration points at another vault.
  - `TestVaultRemoveOnAPendingEnrollmentAsksBeforeDeleting` ⚑: built with
    `identity add` (a local key the vault doesn't list yet), as Go's
    test is. Answering no keeps it, and the prompt names the file.
  - `TestVaultRemoveLeavesTheVaultsOwnFilesUntouched`
  - `TestOneRepositoryRegisteredTwiceSharesOneIdentitiesDirectory` ⚑:
    removing either registration keeps a key the other still needs.

### 5. Parity, review, ledger

- [x] Extend the fixture-parity tests for `ls` (`-H/--header`) and check
      `cat` and `vault remove` against theirs.
- [x] **Opus review (⚑)** of the rows marked ⚑ in task 4. For each, name
      the mutation it kills and record the table here:
  - comparing device names instead of public keys
  - deleting without asking when the key is absent
  - a failed recipient read treated as "not a recipient"
  - skipping the registration-versus-vault check
  - `confirm`'s error treated as yes

      **Method.** The Opus pass re-ran every mutation itself rather than
      relying on the implementing session's table. Each one is a
      one-line edit to the real code, followed by
      `cargo test -p gage --lib orphan` and
      `cargo test -p gage-cli --lib vault_remove`, then the file is
      restored. M6 and M7 are additions on the CLI side. The plan's
      "deleting without asking when the key is absent" was split into
      M2a and M2b, because the implementing session had tested only the
      first reading. The code was also read against Go's
      `removeOrphanedIdentity` and `checkRegistrationPointsAtThisVault`
      at the pinned SHA, and it matches step for step, including letting
      an unreadable vault config through the registration check.

      | # | Mutation (where) |
      |---|---|
      | M1 | Match recipients by `device`, not `pubkey` (`orphan::assess`) |
      | M2a | Offer when no public key is recorded (drop the `pubkey.is_empty()` keep) |
      | M2b | Delete on `Offer` without asking (`confirmed = true`) |
      | M3 | A failed recipient read treated as an empty list |
      | M4 | Skip the registration-versus-vault check (`from_registration` never mismatches) |
      | M5 | `confirm`'s error treated as yes (`unwrap_or(true)`) |
      | M6 | Delete after a no (the `return` dropped after "not confirmed") |
      | M7 | A `Keep` still asks, and deletes on a yes |

      All eight are killed. The table below lists the ⚑ rows and the
      extra CLI rows. Every library row is also killed by its
      `orphan::tests` counterpart: M1 by three, M2a, M3 and M4 by one
      each. M2b, M5, M6 and M7 are CLI-only and have no library row.

      | Row | Kills |
      |---|---|
      | `KeepsIdentityFileStillListedAsRecipient` ⚑ | M7 |
      | `KeepsIdentityFileWhenRecipientsUnreadable` ⚑ | M3, M7 |
      | `NonInteractiveVaultRemoveNeverDeletesAnIdentity` ⚑ | M2b, M5, M6 |
      | `NeverDeletesWithoutConfirming` ⚑ | M1, M2b, M6 |
      | `KeepsARelabelledRecipientsKey` ⚑ | M1, M7 |
      | `KeepsTheFileWhenNoPubkeyIsRecorded` ⚑ | M2a, M7 |
      | `NeverDeletesAKeyBelongingToADifferentVault` ⚑ | M7 only (see finding 1) |
      | `OnAPendingEnrollmentAsksBeforeDeleting` ⚑ | M2b, M6 |
      | `OneRepositoryRegisteredTwiceSharesOneIdentitiesDirectory` ⚑ | M1, M2b, M6, M7 |
      | extra: `…keeps_the_file_when_the_registration_points_at_another_vault` | M4, M7 |
      | extra: `…treats_a_failed_confirmation_as_no` | M1, M2b, M5, M6 |

      Each ⚑ row kills at least one mutation, and each mutation is
      killed by at least one ⚑ row or extra row.

      Findings:
      1. **Confirmed:** Go's `…NeverDeletesAKeyBelongingToADifferentVault`
         doesn't reach the registration check. B's removal looks under
         B's id, finds no identity, and returns `Nothing`, so M4 survives
         that row. The row still guards id-keying (P1), which is where
         its A20 regression lived. M4 is killed only by
         `orphan::tests::a_registration_for_a_different_vault_keeps_the_file`
         and the extra CLI row. Keep both.
      2. **Mutation-list wording:** the implementing session read item 2
         as M2a. M2b is the more natural reading, and it is killed by
         five ⚑ rows. Both are now recorded.
      3. **Not a gap:** in this file, the stub prompter panics on an
         unscripted `confirm`. So every `FakePrompter::new()` call here
         (for example the first `vault remove` in the different-vault
         row) also asserts that nothing was asked.
      4. **Residual, accepted:** `--yes` is global. It is safe on
         `vault remove` only because `AssumeYes::confirm` passes through.
         Making that method answer yes is killed by M9b's
         `yes_does_not_answer_the_self_removal_confirmation`, and by
         nothing under `vault_remove`. That is adequate, because the
         wrapper is shared. A `vault remove --yes` row that answers no
         and keeps the file would pin this down locally, if wanted.
      5. **Not a test issue:** `assess` and the delete run without the
         vault lock. That matches Go, since reads don't take the lock,
         and the store isn't written here. `identityfile::remove` deletes
         the one file and prunes only a directory that holds no
         identities.

- [x] Parity ledger: §2 #39, #55, #54; §4 `cat`, `ls`, `vault_remove`;
      the P1 · M1 and P2 inbound rows.
- [x] Mark M12b `[x]` in the [P6 index](index.md) after CI.

## Definition of done

Every box above is checked. All 10 claimed rows and the 18 inbound ones
are ported or excused in writing, and the review table is recorded.
`make lint` and `make test` are clean locally and on Linux, macOS and
Windows CI.

The milestone is done when:
- plain `ls` output hasn't changed by a byte
- no path through `vault remove` deletes an identity file without a yes
  from a human, and every kept file says why
