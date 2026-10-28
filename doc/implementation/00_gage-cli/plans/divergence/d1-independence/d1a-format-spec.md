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

## Decisions to make first

1. **How the fixture vault is stored.** A vault is a git repo, and a
   nested `.git` can't be committed.
   - Proposed: commit it as a **bare repo** (`vault.git/`, which is plain
     files), and have each test clone it into a tempdir with the existing
     `gittest`/`gitrepo` clone path.
   - Add `tests/fixtures/format-v2/** -text` to `.gitattributes`. The
     repo-wide `* text=auto eol=lf` would otherwise risk rewriting the
     YAML and TOML inside it on Windows checkouts.
2. **The global config's absolute `path`.**
   - Proposed: commit `config.toml.in` with a `@VAULT_PATH@` placeholder
     that the test substitutes. Everything else in it is literal.
3. **The identity.**
   - Proposed: a fixed, documented test passphrase.
   - The identity is wrapped at `TEST_SCRYPT_WORK_FACTOR` (10). Reading
     has a cap (22) and no floor, so this exercises the real unwrap path
     cheaply.
   - The fixture's README says the passphrase is public and the vault
     holds no secrets.
4. **What the fixture contains.** Proposed, the smallest vault that
   touches every artifact:
   - two devices as recipients, one of them a recovery key
   - three entries: one with `fields` and unknown YAML keys, one with a
     multi-line value, one minimal
   - a remote-sync merge commit
   - an enrollment request in `pending/`
   - a populated trust cache
   - a `[shell]` section with durations
5. **Where the tests live.**
   - Proposed: library-level schema tests in
     `crates/gage/src/format_v2_tests.rs`.
   - End-to-end reads through `gage_cli::run` in
     `crates/gage-cli/src/format_v2_tests.rs`, in process, per the
     testing conventions.
6. **The spec's normative language.**
   - Proposed: RFC 2119 MUST/SHOULD for what a reader accepts and
     rejects.
   - Each section ends with "Defined in:" plus file paths.

## Tasks

### 1. Spec

- [ ] `doc/format/v2.md` covers every artifact in the inventory above:
      field names and types, what is optional, what a reader rejects
      (unknown `format_version`, invalid vault id or device name, an
      over-cap scrypt factor) and what it preserves (unknown entry YAML
      keys).
- [ ] `doc/format/README.md` states the versioning rule. A change needs a
      `format_version` bump, a new `format-v<N>/` fixture, and a
      migration ADR, and v2 fixtures stay readable or migratable forever.

### 2. Generator and fixture

- [ ] An `#[ignore]`d generator, `write_format_v2_fixture`, builds the
      decision-4 vault through the public library API and writes
      `tests/fixtures/format-v2/`.
- [ ] The fixture is committed with its README (decision 3) and the
      `.gitattributes` rule (decision 1).

### 3. Tests, red first against a hand-broken copy

- [ ] `format_v2_vault_config_parses`: `.gage/config.toml` loads, and
      `format_version`, id, recipients and method are as specified.
- [ ] `format_v2_global_config_parses`: the templated global config
      loads, including the `[shell]` durations.
- [ ] `format_v2_schemas_round_trip`: every TOML schema and every entry
      payload goes through parse, serialize and parse with equal values,
      and unknown entry keys survive.
- [ ] `format_v2_unlocks_and_lists`: an in-process `gage ls` against the
      cloned fixture lists the three titles.
- [ ] `format_v2_shows_every_entry`: `gage cat` and `gage show` return
      the committed values, including `fields` and the multi-line value.
- [ ] `format_v2_trust_cache_verifies`: `recipient verify` exits 0
      against the committed cache.
- [ ] `format_v2_pending_request_lists`: `recipient pending` shows the
      committed request (with a fixed clock, so it hasn't expired).
- [ ] `format_v2_commit_layout`: the author, branch and message shapes
      in the fixture's history match the spec.
- [ ] `format_v2_fixture_is_byte_stable`: a hash manifest of
      `tests/fixtures/format-v2/` fails on any edit to the fixture
      itself.
- [ ] Each guard is proven to fail once by hand (rename a serde field,
      edit one fixture byte, then revert). Record that here.

### 4. Rule

- [ ] CLAUDE.md, under "Key invariants": any change to an artifact in
      `doc/format/v2.md` needs a `format_version` bump, a new fixture
      and an ADR. The `format-v2` tests may not be edited to make a
      change pass.

## Definition of done

- Every box above is ticked.
- The `format_v2_*` tests are green on Linux, macOS and Windows CI.
- `make lint` and `make test` are clean.
- No existing test or behavior changed.
