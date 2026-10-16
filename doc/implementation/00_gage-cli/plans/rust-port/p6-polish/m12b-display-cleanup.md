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

## Decisions to make first

- **Where `cat`'s alignment parses.** Go walks yaml.v3's node tree for
  each key's line. Here the parser (`saphyr`) lives in `crates/gage`,
  and [Q-YAML-CRATE](../open-questions.md#q-yaml-crate) chose it partly
  because it records the spans this needs. Lean: the library exposes the
  key layout of an entry's YAML (each key's line, depth and whether it is
  quoted, or `None` for a non-mapping), and `gage-cli` does the padding
  and the id line. That keeps one YAML parser in the tree and the
  rendering in the frontend. Confirm `saphyr`'s spans give a key's line
  and column for both plain and quoted keys before writing the tests.
- **Where the orphan check lives**
  ([Q-ORPHAN-CLEANUP](../open-questions.md#q-orphan-cleanup)). Go has it
  all in `cmd/gage`. Here the library returns a typed value from the
  registration's id, device, pubkey and path:
  `OrphanCheck::{Nothing, Keep { path, reason: KeepReason }, Offer { path, device }}`.
  `KeepReason` carries the recipient label or the underlying error where
  Go's message names one. `gage-cli` renders `Keep` on stderr, and asks
  `confirm` on `Offer` with a prompt that names the file and says the
  deletion can't be undone. Deleting goes through the library's existing
  identity removal. The library never asks.
- **Order inside `vault remove`.** Unchanged from Go: forget the
  registration, write global config, delete the trust cache, *then* the
  orphan check. The check reads the store, which may be gone, and
  `vault remove` must succeed regardless.
- **The keep message for a still-listed key.** Go's says "Remove it as a
  recipient first (from another device, with --reencrypt)". Check what
  `recipient remove` takes in gage-rs (P5 · M9b) and name its real flags.
- **`ls -H` in the library?** No. `render_ls` is `gage-cli`'s, and the
  header is a second shape of the same renderer, not a library concept.

## Tasks

Write each task's tests first, at task granularity.

### 1. `cat` — 12 rows (10 + 2 inbound)

- [ ] The library's key-layout reader, then the CLI's id line, padding
      and dedent, used by `cat` in both modes.
- [ ] Rows (`cat_test`):
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
- [ ] Inbound (`entry` CLI, from P2):
  - `TestAlignCatOutputLeavesNonMappingInputAlone`
  - `TestPadKeyLinesWithNoKeysIsANoOp`

### 2. `ls -H/--header` — 4 inbound rows

- [ ] `-H/--header` on `ls`; the table shape from Go's `lsTable`.
- [ ] Rows (`entry` CLI, from P2):
  - `TestLsDefaultOutputUnchangedByHeaderFeature`: no `|`, one line per
    entry, and the same bytes as before this milestone.
  - `TestLsHeaderPrintsLabelledPipeDelimitedTable`: `-H` and `--header`.
  - `TestLsHeaderEmptyVaultNoOutput`
  - `TestLsHeaderRendersIdenticallyInBothModes`

### 3. Orphan check in the library

- [ ] `OrphanCheck` and the function that produces it, with in-crate
      unit tests for each variant before any CLI test. These are not
      Go rows; they are where each `KeepReason` is proven without a CLI
      in the way.

### 4. `vault remove` cleanup — 12 inbound rows

- [ ] Wire the check into `vault remove` after the trust-cache deletion.
- [ ] Rows (`vault_lifecycle`, from P1):
  - `TestVaultRemoveDeletesAnOrphanedIdentityFile`: confirmed, the file
    is gone.
  - `TestVaultRemoveKeepsIdentityFileStillListedAsRecipient` ⚑
  - `TestVaultRemoveKeepsIdentityFileWhenRecipientsUnreadable` ⚑: the
    store deleted; kept, and stderr says it "could not confirm".
- [ ] Rows (`vaultid_keying`, from P1):
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

- [ ] Extend the fixture-parity tests for `ls` (`-H/--header`) and check
      `cat` and `vault remove` against theirs.
- [ ] **Opus review (⚑)** of the rows marked ⚑ in task 4. For each, name
      the mutation it kills and record the table here:
  - comparing device names instead of public keys
  - deleting without asking when the key is absent
  - a failed recipient read treated as "not a recipient"
  - skipping the registration-versus-vault check
  - `confirm`'s error treated as yes
- [ ] Parity ledger: §2 #39, #55, #54; §4 `cat`, `ls`, `vault_remove`;
      the P1 · M1 and P2 inbound rows.
- [ ] Mark M12b `[x]` in the [P6 index](index.md) after CI.

## Definition of done

Every box above is checked. All 10 claimed rows and the 18 inbound ones
are ported or excused in writing, and the review table is recorded.
`make lint` and `make test` are clean locally and on Linux, macOS and
Windows CI.

The milestone is done when:
- plain `ls` output hasn't changed by a byte
- no path through `vault remove` deletes an identity file without a yes
  from a human, and every kept file says why
