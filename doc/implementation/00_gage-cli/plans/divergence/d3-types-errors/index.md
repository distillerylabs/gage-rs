# D3 Types and errors — implementation plan

[← divergence plan](../index.md) · Parent issue
[#131](https://github.com/distillerylabs/gage-rs/issues/131) · Roadmap
[#114](https://github.com/distillerylabs/gage-rs/issues/114)

D3 gives the library's types and error boundary a Rust shape instead of
Go's:

- vault ids become a validated newtype
- string constants become enums
- `""`-means-absent fields become `Option`
- library errors stop speaking for the CLI

D3 changes no on-disk bytes, no exit code and no help output. The golden
vault, the help snapshots and new byte-exact serialization tests prove
it. Error *wording* on stderr does change where a module prefix is
dropped, and ADR-0054 records why.

---

## Where things stand at the start of D3

Measured on `main` at `f36be0a`.

| What | Count | Where |
|---|---|---|
| `valid_id`/`check_vault_id` calls | 16 | `paths.rs`, `vaultconfig`, `enrollment/seal.rs`, `vault.rs` |
| `TYPE_GIT`/`METHOD_PASSPHRASE`/`ALLOWED_*` uses | 81 | both crates, help text included |
| `skip_serializing_if = "String::is_empty"` | 10 | `config.rs`, `vaultconfig`, `trustcache.rs` |
| `#[error("gage: …")]` | 48 | plus the hand-written `Display` impls in `remotesync.rs` and `syncerr.rs` |
| module-prefixed `#[error("config: …")]` etc. | 34 | `config`, `vaultconfig`, `atomicfile`, `vaultlock`, `recipients`, `xdgpaths`, `agekey` |
| `` `gage …` `` command hints in library text | 18 | `remoteauth`, `remotesync`, `trustcache`, `vault`, `enrollment` |
| `Prompter::warn` texts starting `gage:` | 7 | plus the `trustcache` summary and other `format!`s |
| `impl HasCode for` (library) | 39 | every error enum |
| `prefix_once` callers | 5 | plus 7 hand-written strips of `"gage: "` |

Regenerate with:

```
grep -rn --include='*.rs' -E 'valid_id\(|check_vault_id\(' crates/gage/src
grep -rn --include='*.rs' -E 'TYPE_GIT|METHOD_PASSPHRASE|ALLOWED_(TYPES|METHODS)' crates
grep -rn --include='*.rs' 'skip_serializing_if = "String::is_empty"' crates/gage/src
grep -rn --include='*.rs' -E '#\[error\("gage:' crates/gage/src | grep -v _tests.rs
grep -rn --include='*.rs' -E '#\[error\("[a-z_]+: ' crates/gage/src | grep -v -E '_tests.rs|error\("gage:'
grep -rn --include='*.rs' -E '`gage [a-z]' crates/gage/src | grep -v _tests.rs
grep -rn --include='*.rs' 'impl HasCode for' crates/gage/src
```

**One gap the plan has to close first.** No test re-serializes a config
file and compares bytes:

- `format_v2_schemas_round_trip` compares *values*.
- `format_v2_new_vault_writes_the_same_layout` compares *sorted key
  sets*.
- The only byte-exact serialization assertion is
  `toml::to_string(&Global::default()) == ""`.

So turning a `String` into an `Option` or an enum could reorder keys, or
change `key = ""` into an omission, and nothing would fail. D3a adds
byte-exact tests before it changes a type.

## Resolved decisions

See the [divergence plan](../index.md#scope-of-the-later-phases-refined-when-each-is-filed).
Specific to D3:

- **`VaultType` is strict, `UnlockMethod` is lenient.**
  - `VaultType` has one variant, `Git`, so an unknown `type` is refused
    at parse. [v2 §2.2](../../../../../format/v2.md) already says "git,
    the only type". Today an unknown type parses and silently loses git
    behavior.
  - `UnlockMethod` has `Passphrase` plus an unknown variant that keeps
    the string. A vault that names a method a later build adds still
    opens here, and is refused at unlock with `UnsupportedMethod`, as
    today. Strict parsing would make an older build refuse the whole
    vault config.
  - ADR-0053.
- **A bad vault id in the global config is refused at load.**
  - `[vaults.X].id` becomes `Option<VaultId>` and is validated as it is
    read. Today it is never validated, and a bad one fails later, at a
    path builder.
  - Only gage writes that file, so a bad id there is local corruption.
    The error names the file and the key.
  - ADR-0052 records `VaultId`, including this reader tightening.
- **CLI hints move to the CLI.**
  - A library error that today ends "run `gage sync`" carries a typed
    `Remedy` instead, and `gage-cli` renders the command text.
  - stderr is unchanged where only a hint moved.
  - A second frontend gets a value it can turn into a button, not a
    shell command.
- **Module prefixes are dropped.**
  - Library messages read as sentences, without `config:`,
    `vaultconfig:`, `atomicfile:` and so on.
  - Error wording is not in the CLI contract, which covers help, flags,
    exit codes and the duration grammar. Tests asserting the old strings
    are updated in the same change.
  - Exit codes and the single `gage:` prefix stay identical.
  - ADR-0054 records the error boundary.
- **The `Option` rule.**
  - A serde field becomes `Option` only where it is skipped when empty
    today:
    - `Global.current`
    - `VaultEntry.id`, `VaultEntry.pubkey`
    - `GitMeta.origin`
    - the four `Shell` fields
    - `Method.plugin`
    - `TrustRecord.mismatch_seen`
  - On read, `key = ""` reads as `None`, so an older or hand-edited file
    means what it meant before.
  - Required fields (`path`, `type`, `device`, `method`, every
    `Payload` key) stay non-`Option`, and are typed where an enum
    applies.
  - Non-serde `""`-as-absent values follow the same rule:
    `vault::ResolveError::Mismatch.registered_id` and
    `EnrollmentRequest.id`.
- **ADR-0037 already binds the exit-code half.** It says "Library errors
  map to codes in `gage-cli`, not in the library". The code contradicts
  it today, and D3b makes the code match. ADR-0054 covers the rest of
  the boundary: prefix, module names, remedies and warnings.
- **Commit messages stay.** `gage: initialize vault`, `gage: merge
  origin`, and the enrollment, recipient and recovery messages are
  on-disk format, pinned by `format_v2_commit_layout`. "No `gage:` in
  the library" means none in error or warning text.

## Milestones

| Milestone | Status | Model | Depends on |
|---|---|---|---|
| [D3a `VaultId`, enums, `Option` config](d3a-vaultid-enums.md) ([#132](https://github.com/distillerylabs/gage-rs/issues/132), PR [#136](https://github.com/distillerylabs/gage-rs/pull/136)) | `[x]` | **Opus** | parent branch |
| [D3b Error/frontend boundary](d3b-error-boundary.md) ([#133](https://github.com/distillerylabs/gage-rs/issues/133)) | `[ ]` | **Opus** | D3a |
| [D3c Small Go-isms](d3c-small-go-isms.md) ([#134](https://github.com/distillerylabs/gage-rs/issues/134)) | `[ ]` | Sonnet | D3b |

**Why Opus for D3a.**

- A vault id becomes a filesystem path component, and its validation is
  a security boundary (ADR-0006).
- Every serde change risks a byte change in a committed or local file.

The milestone moves validation to the type, so the reasoning that every
path into a path builder is validated has to be redone, not copied.

**Why Opus for D3b.** It rewrites the text of every library error, and it
changes the one seam every frontend renders errors through.

- The risk is in what gets lost: a security-relevant refusal whose
  message stops saying why, or an exit code that moves because a
  conversion changed.
- A secret must never reach an error message. `syncerr` redacts URL
  userinfo today, and that has to survive the rewrite.

**Why Sonnet for D3c.** Each item is a local refactor with the tests
already in place.

**Why this order.**

- D3b rewrites the messages of the error types D3a re-types (`paths`,
  `vaultconfig`, `config`, `CreateError`).
- D3c renames the load and save functions whose error types D3b settles.

## Acceptance

- Every milestone doc's test list is green.
- `git diff --stat main -- tests/fixtures` is empty. The help snapshots
  and the existing `format_v2_*` tests are green and unedited.
- Exit codes are unchanged. Every error reaches stderr with `gage:`
  exactly once.
- `crates/gage` has no `HasCode`, no `exitcode` module, no `gage:`
  outside commit messages, and no `` `gage …` `` command text.
- ADR-0052, ADR-0053 and ADR-0054 are linked from
  `doc/decisions/README.md`.
- CLAUDE.md's module list matches: `exitcode` has moved to `gage-cli`.
- `make lint`, `make test` and `make qa` are clean.
- CI is green on Linux, macOS and Windows.

## Process notes

- **Branches stack:** `main` <- `131-divergence-types-errors` (these
  docs) <- D3a <- D3b <- D3c. Each milestone branch is cut from the
  previous one when the user says to start it.
- **Byte-exact tests come first.** D3a's first task must be green on
  `main`'s code before any serde type changes. If one is red on `main`,
  stop and record it. Either the writer isn't deterministic or the
  fixture differs from what gage writes, and both are format questions,
  not D3 work.
- **Behavior proof.** The help snapshots and golden vault are not edited.
  A change that seems to need either is a format or CLI change, and needs
  its own ADR first.
- **Test text updates in D3b** are a reviewable list. Each assertion that
  changes moves from a prefixed string to the sentence, never to a looser
  `contains`. A test that pinned a security-relevant reason keeps pinning
  it.
- **If context runs low mid-milestone,** write a `*-sessionhandoff.md`
  here.
