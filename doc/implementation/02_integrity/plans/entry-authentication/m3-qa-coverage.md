[index](index.md) · [M0](m0-library-mac-and-v3.md) · [M1](m1-membership-and-sync.md) · [M2](m2-frontend-and-docs.md) · **M3**

# M3 — QA coverage of entry authentication and membership changes

> Recommended model: **Sonnet**. Pattern-following shell scenarios against
> behavior a probe has already pinned down, and docs.

Sub-issue: [#206](https://github.com/distillerylabs/gage-rs/issues/206).

## Goal

`make qa` proves entry authentication through the real binary, as several
people sharing a vault through git, with the most weight on what changes
when a recipient is added or removed. A regression that stopped rotating the
key, or stopped verifying a tag, fails QA.

## Why this milestone exists

M0–M2 are covered in-process, but `scripts/qa/` has no scenario aimed at any
of it. `make qa` is green on the M2 branch (644 assertions) and also would be
if authentication were switched off, because no assertion can tell the
difference:

| Shipped behavior | `scripts/qa/` today |
|---|---|
| A forged, altered or relocated entry is refused, exit `1` | None |
| One bad entry fails `ls`, `search`, sessions, writes | None (`10` only covers a non-age file) |
| Missing, corrupt or replaced `vault-key.age` | None |
| v3 written, v2 refused | None (`10` only tries `format_version = 999`) |
| `auth` is a reserved key | None |
| **Add:** key re-sealed, not rotated | Incidental: the new person can read (`03`, `04`) |
| **Remove:** key rotated, old key and old blobs refused | None: `05` shows only that the removed person can't decrypt, which rotation doesn't affect |
| **Recovery** `rotate` / `enroll --replaces` rotate the key | None: `08` shows only that old keys stop decrypting |
| `history` reads revisions across a rotation | None: `05` runs it only as the removed person |
| Every write path stamps a tag the others accept | Partial, and never after a rotation |
| Merge straddling a key change, re-stamped by `sync` | None |
| Removed device can't complete that merge | None |
| A remote key that doesn't continue this one is refused | None |
| Half-applied rotation refuses reads, next write repairs | None |

`make qa` only runs in CI on PRs into `main`, so this stack hasn't had it.
Run it locally for this milestone.

## Depends on

M2 (the CLI surfaces the failure). Branch `206-entry-auth-qa-coverage` off
`201-entry-auth-frontend-docs`.

## Design references

- `scripts/qa/README.md` and `qa-env.sh`, the harness. `run-all.sh` picks up
  `scenarios/[0-9]*.sh` on its own.
- ADR-0065 and ADR-0066, the guarantees being tested.
- `doc/cli/entries.md` "When an entry fails to verify", the documented way
  out.
- `doc/design/security-model.md` "Limits".

## How the scenarios make their inputs

No new tool dependencies. CI's QA job has `expect` and `python3`, not `age`.

- **A forged entry.** `mallory` runs her own gage vault and adds the team's
  public keys (from the committed `.age-recipients`) as recipients. Her
  entries are well-formed, decrypt for the team, and carry a tag under the
  wrong key. Plain `git` in a throwaway clone puts them on the team's remote,
  as a stranger with push access would.
- **Proving a rotation.** By contrast, not by parsing the key file. Save an
  entry's `.age` file, change membership, put the saved file back:
  after an add it still verifies, after a removal it's refused. The saved
  file is still encrypted to the people reading it, so a refusal there can
  only come from the tag.
- **A removed member's write.** Bob writes while offline, is removed, then
  rebases onto the rotated head and pushes with plain `git`. The entry
  decrypts for everyone left and carries a tag under the retired key.

The cases below were run against the M2 binary with a throwaway script;
"expect" is what they did. Assert exit codes and the stable part of a message.

## Decisions to make first

1. **Resolved: a vault replaced wholesale reads as genuine; M3 pins it and
   [#207](https://github.com/distillerylabs/gage-rs/issues/207) fixes it.**
   Someone with only
   push access replaces `.gage/vault-key.age` with one sealed to the public
   recipient list, and replaces every entry with their own tagged under it.
   `show` prints their value and exits `0`, with one line on stderr saying
   the key file was replaced. `ls` and `--stdin` sessions do the same. After
   one `--yes` write the warning stops too. This is the consequence of
   "unlock and sync only warn" (`doc/cli/identities-and-recipients.md`), but
   it isn't listed under the security model's limits, and it falls short of
   #198's first acceptance criterion. Replacing the key but not every entry
   is caught, since the remaining genuine entries fail.
   M3 stays reviewable and mergeable by pinning this: the scenario asserts
   the warning and exit `0`, with a comment naming #207, and #207 flips the
   assertion. The fix is separate because it is security work with its own
   ADR. Accepting this as a documented limit was set aside.
2. **Resolved: don't pin the straddling merge's wording.** `pull`, `push`
   and `sync` report "N path(s) changed on both sides" and list entries that
   only one side wrote. Scenarios assert the exit code and `run .gage sync`,
   so correcting that text later doesn't break them.

## Tasks (tests first)

Each scenario is written and run against the existing binary. A failure is a
product bug to report, not an assertion to loosen. Shared helpers (entry id
to file, the mallory vault, a stranger's push) go in `qa-env.sh`.

### `13-entry-auth-forgery.sh`: forgery and failure surface

- [x] **Format.** A new vault has `format_version = 3`, a tracked
      `.gage/vault-key.age`, and `.gage/vault-key.age -merge` in
      `.gitattributes`.
- [x] **Stranger's forged entry.** Pushed as above: `show`, `ls`, `search`
      and `--stdin` each exit `1` with `failed authentication` and the id.
      The forged value appears in neither stream.
- [x] **Bound to the id.** Two entries' files swapped, and an entry copied to
      a new id, each exit `1`.
- [x] **Everything stops.** With a bad entry present, a write exits `1` too.
- [x] **The documented way out.** `git rm`, commit and push the named file:
      `ls` exits `0` and the real entries are intact.
- [x] **Key file states.** Uncommitted delete: read exits `1` (`is missing`),
      the next write discards the change and reads work. Committed delete:
      read and write both exit `1`. Garbage bytes: read exits `1` (`corrupt`).
- [x] **Key file replaced, entries genuine.** Unlock warns `was replaced on
      its own`, and the read exits `1`. A piped write exits `1` (`nobody is at
      a terminal`). On a pty the review says `vault-key.age was replaced`, and
      declining writes nothing.
- [x] **Vault replaced wholesale.** Every entry and the key file replaced:
      `show`, `ls` and `--stdin` exit `0` and show the forged value, unlock
      warns `was replaced on its own`, and after one `--yes` write the warning
      stops. Pins decision 1; a comment names #207.
- [x] **Reserved key.** `edit` adding a top-level `auth:` exits `2` and the
      entry is unchanged. `--field auth=x` is a user field and round-trips.
- [x] **v2 refused.** A *copy* of `tests/fixtures/format-v2/vault.git`:
      `clone` exits `1` (`older format`), registering nothing and leaving no
      directory. An existing vault edited to `format_version = 2` exits `1`
      on `show`. A copy of the `format-v3` fixture clones with exit `0`.

### `14-entry-auth-membership.sh`: adding and removing recipients

Adding:

- [x] **Re-seal on `recipient add`.** The add's commit touches
      `.gage/vault-key.age`. The new recipient reads every existing entry,
      including one another recipient wrote.
- [x] **New recipient writes.** Every other recipient reads it.
- [x] **Add doesn't rotate.** A file saved before the add still verifies when
      restored.
- [x] **Enrollment.** `recipient approve`: the same three, for the approved
      device.
- [x] **First recovery key.** `recovery rotate` on a vault that had none only
      grows the list: a saved file still verifies.
- [x] **New member's history.** `history --decrypt` of a revision from before
      they joined exits `5` (encrypted to the old list), and one from after
      exits `0`.

Removing:

- [x] **One commit.** `recipient remove --reencrypt` puts entries, key file
      and recipient files in a single commit.
- [x] **Those left keep working.** The remover and a remaining recipient read
      every entry. Each reads what the other writes afterwards.
- [x] **A saved file is refused.** A file saved before the removal, restored,
      exits `1` with `failed authentication` for someone who can still decrypt
      it.
- [x] **`history` across rotations.** A remaining recipient's `history
      --decrypt` exits `0` after one removal and after two.
- [x] **Removed member's forged write.** Pushed as above: remaining
      recipients exit `1` naming the id, and the documented way out clears it.
- [x] **Removed member through gage.** `sync` exits `5` with HEAD unmoved and
      nothing pushed. `pull` and `push` exit `1`.
- [x] **Remove, then re-add.** The returning member reads current entries and
      writes one the others accept.
- [x] **Every write path after a rotation.** `insert`, `generate`, `edit`,
      `rename`, and `cp`/`mv` from another vault: a remaining recipient's
      `ls` and `show` exit `0`.
- [x] **Recovery rotates.** `recovery rotate` replacing a key, and `recovery
      enroll --replaces`, each change the key file and refuse a saved file. The
      recovering device reads every entry and writes one a remaining recipient
      accepts.

### `15-entry-auth-sync.sh`: merges across a key change

- [x] **Offline writer across a removal.** `pull` and `push` exit `1` and
      don't merge; a further write commits locally and warns. Piped `sync`
      exits `0`. Remaining recipients read the entry, and the removed one
      exits `5` on it.
- [x] **Offline writer across an add.** `sync` exits `0`, and the *new*
      recipient reads the offline entry (the gap ADR-0066 closes).
- [x] **The rotator was offline.** A writer stays online during it: the
      rotator's `sync` exits `0` and its reader verifies the entry.
- [x] **Real conflict across a rotation.** Piped: exit `1`, neither value in
      either stream. On a pty, `l`, `r` and `b` each leave a vault everyone
      remaining can `ls` and `show`.
- [x] **A key that doesn't continue.** A replaced key on the remote while a
      device has a local write: `sync` exits `1` (`does not continue`), HEAD
      unmoved, nothing pushed.
- [x] **Half-applied rotation.** Post-rotation entries with the pre-rotation
      key file in the working tree: read exits `1`, the next write discards it
      and the vault reads. This simulates the state; the crash itself stays
      in-process.

### Docs and harness

- [x] `doc/cli/sync.md`: an entry written across a key change needs `gage
      sync`, and a removed device can't complete it. It says neither today.
- [x] `doc/cli/identities-and-recipients.md` "What counts as a change": add
      the key file.
- [x] `doc/cli/configuration.md` shows `format_version = 2`, and
      `doc/cli/vaults.md` explains only format 2's change. Bring both to v3.
- [x] `scripts/qa/README.md`: its example uses scenario number `13`, so
      renumber it; describe how scenarios forge input.

### Left to the in-process suite

Named so the gap is deliberate: the FFI (the harness drives only the binary);
an entry with no `auth` line at all (needs raw `age`); MAC construction and
known-answer vectors; a real crash mid-rotation (needs an injected failure);
`init` refusing a plugin recipient.

## Definition of done

- Every item above is green.
- `make qa` passes locally with no skipped pty steps. `make lint` and `make
  test` are clean.
- Reverting the rotation (or the tag check) makes the new scenarios fail.
  Confirm this once by hand and say so in the PR.
- Stop for review.
