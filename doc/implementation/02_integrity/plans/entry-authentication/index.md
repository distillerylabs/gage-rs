# Entry authentication (format v3)

Parent issue: [#198](https://github.com/distillerylabs/gage-rs/issues/198).

## Why

An entry is an `age` file encrypted to the vault's recipients. `age`
authenticates ciphertext integrity but not the writer: producing an entry
needs only the public recipient keys, which are committed in
`.age-recipients`. An entry's payload is also not bound to its filename id.
So the guarantee the security model names as a goal — that only a vault
member can write an entry the client trusts — is not enforced.

This phase adds a per-vault authentication secret so that **only a holder of
a recipient key can produce a valid entry**, and binds each entry to its id.

## Approach (resolved)

A **per-vault MAC key that only recipients hold**:

- A random per-vault MAC key, minted at `init`, stored sealed to every
  recipient (its own `age`-encrypted file). It is re-sealed to the current
  recipients whenever membership changes, reusing the re-encryption loop that
  already rewrites every entry.
- Each entry carries `auth = HMAC-SHA256(mac_key, uuid ‖ canonical
  entry-fields)` inside its encrypted payload. Every decrypt recomputes the
  MAC over the entry's fields and the *filename's* UUID and rejects a
  mismatch.

This authenticates "a recipient wrote this," which matches gage's trust model
(a vault is the unit of trust; recipients are mutually trusted). It is not
per-device provenance.

### Resolved decisions

- **MAC key, not signatures.** A shared secret sealed to recipients is
  proportionate and reuses existing machinery; per-device Ed25519 signatures
  were set aside as disproportionate (they would add a key type across
  enrollment, recovery and the mobile bindings).
- **Format v3; v2 refused outright.** No unauthenticated read path is kept; a
  v2 vault is rejected with a clear message. Viable pre-release with no other
  users.
- **The auth tag lives inside the encrypted payload** (a new field), so there
  is one file per entry and the merge model is unchanged.
- **Rotate the MAC key on recipient removal**, re-seal (no rotation) on
  addition.

## Milestones

| # | Doc | Model | Depends on | Status |
|---|---|---|---|---|
| M0 | [m0-library-mac-and-v3.md](m0-library-mac-and-v3.md) | Opus | — | Done, in review |
| M1 | [m1-membership-and-sync.md](m1-membership-and-sync.md) | Opus | M0 | Done, in review |
| M2 | [m2-frontend-and-docs.md](m2-frontend-and-docs.md) | Sonnet | M1 | Not started |

## Process notes

- This is a format change: per CLAUDE.md "Format stability", M0 lands the
  `format_version` bump, `doc/format/v3.md`, `tests/fixtures/format-v3/`, and
  an ADR together. The frozen `format-v2` fixture is not edited; the
  `format_v2_*` tests change from "loads" to "refused", which is legitimate
  under a deliberate version bump.
- Branches are a stack merging into `main`:
  `main` ← `198-entry-authentication` (these plan docs) ← M0 ← M1 ← M2.
- Each milestone is tests-first, ends green on `make lint` and `make test`,
  and stops for review. The milestone doc's test list is its definition of
  done.

## Out of scope

- Per-device provenance (a current recipient can still write another
  recipient's entry).
- The removed-member window before the MAC key is rotated (same semantics as
  their continued ability to read old git history).
- Rollback / freshness of the whole vault — separate work.
