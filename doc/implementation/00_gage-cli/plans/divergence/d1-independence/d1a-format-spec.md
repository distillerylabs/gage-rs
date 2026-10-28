# D1a — Format v2 spec and golden vault

[D1 plan](index.md) · [#116](https://github.com/distillerylabs/gage-rs/issues/116) · next: [D1b](d1b-retire-parity.md)

> **Recommended model: Opus.** The spec becomes the contract that D3/D4
> refactors are checked against. A fixture that is wrong, or that leaves
> out an artifact, lets a format drift through unnoticed. Deciding what a
> reader must reject and what it must preserve is security-adjacent
> judgement.

## Goal

The v2 on-disk format becomes gage-rs's own, written down and guarded by
tests:

- `doc/format/v2.md` specifies every on-disk artifact normatively, citing
  the code that defines each one.
- A golden v2 vault, written once by gage-rs, is committed under
  `tests/fixtures/format-v2/`. Tests read it back on all three OSes.
- CLAUDE.md gains a format-stability rule.

## Depends on

The parent branch `115-divergence-independence`. D1a only adds files, so
it merges on its own.

## Design references

- The TDDs' "On-disk layout" sections
  (`doc/implementation/00_gage-cli/tdds/gage-cli-design.md`,
  `gage-cli-init-design.md`).
- The archived register's schema decisions: Q-COMPAT (the schema as a
  contract), Q-YAML-CRATE (the entry emitter), Q-TRUST-HASH,
  Q-TRUST-CACHE-PATH and Q-AGE-CRATE (the scrypt factors and cap).
- The defining code (see [D1 index](index.md#where-things-stand-at-the-start-of-d1)).

## Sources

- **The artifact inventory** from the D1 audit:
  - `.gage/config.toml`
  - the global `config.toml`
  - `.age-recipients`
  - `.gitattributes` and `.gitignore`
  - `entries/<uuid>.age`, and the entry YAML payload inside the age layer
  - `.gage/pending/<id>-<expiry>.age`
  - the identity file and its `vault-name.txt` marker
  - `known-config.toml` and `trust.toml`
  - the lock and token paths
  - the reserved `recovery-paper-key` label
  - the commit author, branch, remote and every commit-message shape
  - the `[shell]` duration grammar
- **What is not format, and stays out of the spec:**
  - help text and error wording
  - exit codes (a CLI contract, owned by `doc/cli/exit-codes.md`)
  - the enrollment-code spelling, which a human types and which is never
    stored

## Decisions (resolved)

All six proposals were accepted as written. Where implementation needed
more than the proposal said, the addition is marked **Added**.

1. **How the fixture vault is stored.** A vault is a git repo, and a
   nested `.git` can't be committed.
   - Committed as a **bare repo**, `vault.git/`, which is plain files.
   - Each test first copies it into a tempdir and then clones the copy
     with `gitrepo::clone`. **Added:** the test reads the copy, never the
     committed tree. A pull on unlock or a stray push then can't touch
     the fixture, and Windows' read-only git object files never sit in a
     tempdir that has to be deleted.
   - `tests/fixtures/format-v2/** -text` is in `.gitattributes`. The
     repo-wide `* text=auto eol=lf` would otherwise risk rewriting the
     YAML and TOML inside it on Windows checkouts.
2. **The global config's absolute paths.**
   - Committed as `config/config.toml.in`, which the test substitutes.
     Everything else in it is literal.
   - **Added:** two placeholders, not one. `@VAULT_PATH@` stands for
     `path`, and `@ORIGIN@` for `[vaults.<name>.git].origin`, which is
     also an absolute path for a local remote.
3. **The identity.**
   - The test passphrase is fixed and documented:
     `format-v2 fixture passphrase`.
   - The identity is wrapped at `TEST_SCRYPT_WORK_FACTOR` (10). Reading
     has a cap (22) and no floor, so this exercises the real unwrap path
     cheaply.
   - The fixture's README says the passphrase is public and the vault
     holds no secrets.
   - **Added:** the recovery key's secret and the enrollment code are
     committed too, as `recovery-key.txt` and in the README. They are
     public for the same reason.
4. **What the fixture contains.** The smallest vault that touches every
   artifact:
   - two devices as recipients, one of them a recovery key:
     `laptop-1` and `recovery-paper-key`
   - three entries: one with `fields` and unknown YAML keys, one with a
     multi-line value, one minimal
   - a remote-sync merge commit. **Added:** it is made the way it happens
     in use. `laptop-1` writes while its remote is unreachable, the
     enrolling device publishes in the meantime, and `gage sync` merges
     the two.
   - an enrollment request from `phone-1` in `pending/`
   - a populated trust cache
   - a `[shell]` section with durations
5. **Where the tests and the generator live.**
   - Library-level schema tests go in
     `crates/gage/src/format_v2_tests.rs`.
   - End-to-end reads through `gage_cli::run` go in
     `crates/gage-cli/src/format_v2_tests.rs`, in process, per the
     testing conventions.
   - **Added:** the generator lives in the CLI file and drives
     `gage_cli::run`. `init`, `clone`, `identity enroll` and `sync` each
     compose several library calls, and the fixture should be what the
     shipped binary writes, not a re-composition of it.
   - The entry with unknown keys is the one exception: it is written with
     the library's `Vault::insert`. No command of this build writes keys
     it doesn't know. They arrive from a newer gage.
   - The fixture's constants and the install helper (copy, substitute,
     clone) live in `gage::testsupport::format_v2`, so both crates read
     one definition.
6. **The spec's normative language.**
   - RFC 2119 MUST/SHOULD for what a reader accepts and rejects.
   - Each section ends with "Defined in:" plus file paths.
7. **Added: the fixed clock.** A committed request's expiry is in the
   past for every later run, and the listing reads the wall clock.
   - Library: `Vault::pending_enrollments` is split into a `pub(crate)`
     `pending_enrollments_at(now)` that it calls with the real clock.
     This is a pure refactor. The test lists at a fixed instant inside
     the request's life, then opens the sealed payload at that instant
     with the committed code.
   - CLI: `gage_cli::run` has no wall-clock seam, and adding one is
     behavior D1a doesn't own. So the end-to-end test re-files the
     committed request in its own clone under a live expiry epoch. The
     id and the sealed bytes are unchanged, and only the filename's
     epoch moves. It then checks that `recipient pending` lists the
     committed id.

## Tasks

### 1. Spec

- [x] `doc/format/v2.md` covers every artifact in the inventory above:
      field names and types, what is optional, what a reader rejects
      (unknown `format_version`, invalid vault id or device name, an
      over-cap scrypt factor) and what it preserves (unknown entry YAML
      keys).
- [x] `doc/format/README.md` states the versioning rule. A change needs a
      `format_version` bump, a new `format-v<N>/` fixture, and a
      migration ADR, and v2 fixtures stay readable or migratable forever.

### 2. Generator and fixture

- [x] An `#[ignore]`d generator, `write_format_v2_fixture`, builds the
      decision-4 vault and writes `tests/fixtures/format-v2/`. It drives
      `gage_cli::run`, plus `Vault::insert` for the entry with unknown
      keys (decision 5).
- [x] The fixture is in the tree with its README (decision 3) and the
      `.gitattributes` rule (decision 1). It is 84 KB.

### 3. Tests, red first against a hand-broken copy

Library (`crates/gage/src/format_v2_tests.rs`) unless marked CLI
(`crates/gage-cli/src/format_v2_tests.rs`).

- [x] `format_v2_vault_config_parses`: `.gage/config.toml` loads, and
      `format_version`, id, recipients and method are as specified.
      `.age-recipients`, `.gitattributes`, `.gitignore` and the `entries/`
      names are checked too.
- [x] `format_v2_global_config_parses` (CLI): the templated global config
      loads, including the `[shell]` durations, through `shell::resolve`.
- [x] `format_v2_schemas_round_trip`: every TOML schema and every entry
      payload goes through parse, serialize and parse with equal values,
      and unknown entry keys survive. The schemas are the vault config,
      `known-config.toml`, `trust.toml`, the global config and the
      enrollment payload.
- [x] `format_v2_unlocks_and_lists` (CLI): an in-process `gage ls`
      against the cloned fixture lists the three titles.
- [x] `format_v2_shows_every_entry` (CLI): `gage show` (with and without
      `--field`) and `gage cat` return the committed values, including
      `fields`, the multi-line value and the unknown keys.
- [x] `format_v2_trust_cache_verifies` (CLI): `recipient verify` exits 0
      against the committed cache.
- [x] `format_v2_pending_request_lists` (CLI): `recipient pending` shows
      the committed request (decision 7). In the library,
      `format_v2_pending_request_opens_at_a_fixed_clock` lists and opens
      it as committed, at a fixed clock.
- [x] `format_v2_commit_layout`: the author, branch and message shapes
      in the fixture's history match the spec. It also checks that the
      constants this build commits with are the fixture's.
- [x] `format_v2_fixture_is_byte_stable`: a hash manifest (`SHA256SUMS`)
      of `tests/fixtures/format-v2/` fails on any edit to the fixture
      itself.
- [x] Each guard was proven to fail once by hand, then reverted. This
      was run on 2026-10-27 on macOS:

| Break | Went red |
|---|---|
| `#[serde(rename = "created_on")]` on `VaultMeta.created` | `format_v2_vault_config_parses`, `format_v2_new_vault_writes_the_same_layout` |
| entry YAML key `updated_by` renamed to `updatedBy` in both emitter and parser | `format_v2_schemas_round_trip`, `format_v2_new_vault_writes_the_same_layout` |
| `#[serde(rename = "recipient_hash")]` on `TrustRecord.recipients_hash` | `format_v2_schemas_round_trip`, `format_v2_new_vault_writes_the_same_layout`, CLI `format_v2_unlocks_and_lists` (the trust warning fired) |
| `[shell].idle_timeout` renamed to `idle` | CLI `format_v2_global_config_parses` |
| one byte of the fixture's `known-config.toml` | `format_v2_fixture_is_byte_stable`, `format_v2_schemas_round_trip` |
| `MERGE_COMMIT_MESSAGE` changed | `format_v2_commit_layout` |

Added beyond the list, and kept: `format_v2_reader_rejects_what_the_spec_rejects`
(format version, id, device name, over-cap scrypt factor),
`format_v2_identity_file_unwraps` (the identity file's plaintext and
marker), `format_v2_new_vault_writes_the_same_layout` (the writing side,
so one change to both reader and writer can't slip through), and
`format_v2_install_leaves_the_fixture_alone`.

### 4. Rule

- [x] CLAUDE.md, under "Key invariants": any change to an artifact in
      `doc/format/v2.md` needs a `format_version` bump, a new fixture
      and an ADR. The `format-v2` tests may not be edited to make a
      change pass.

## Definition of done

Met. The first CI run on PR [#120](https://github.com/distillerylabs/gage-rs/pull/120)
([test](https://github.com/distillerylabs/gage-rs/actions/runs/40642227331),
[lint](https://github.com/distillerylabs/gage-rs/actions/runs/40642227278))
was green on Linux, macOS and Windows. All 14 `format_v2_*` tests ran
and passed on each OS, and the generator was skipped as `#[ignore]`d.
No fixes were needed.


- Every box above is ticked.
- The `format_v2_*` tests are green on Linux, macOS and Windows CI.
- `make lint` and `make test` are clean.
- No existing test or behavior changed.
