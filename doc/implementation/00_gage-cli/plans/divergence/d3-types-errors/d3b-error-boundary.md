# D3b — Error/frontend boundary

[D3 plan](index.md) · [#133](https://github.com/distillerylabs/gage-rs/issues/133) · previous: [D3a](d3a-vaultid-enums.md) · next: [D3c](d3c-small-go-isms.md)

> **Recommended model: Opus.** This rewrites the text of every library
> error and the one seam every frontend renders errors through. The risks:
>
> - a refusal whose message stops saying why
> - an exit code that moves because a conversion changed
> - a secret that reaches a message because a redaction step was
>   bypassed

## Goal

A library error says what went wrong and nothing about who is showing it:

- no `gage:` prefix
- no module prefix
- no `gage …` command to run
- no exit number

A frontend decides all four. `gage-cli` maps an error's kind to exit
codes 0–7, adds `gage:` exactly once, and renders a typed remedy as the
command to run.

The help output is unchanged, and so are the exit codes. stderr is
unchanged except where a module prefix is dropped (ADR-0054).

## Depends on

D3a ([#132](https://github.com/distillerylabs/gage-rs/issues/132)). D3a re-types the errors in `paths`,
`vaultconfig`, `config` and `vault::CreateError`, and this milestone
rewrites their messages.

## Design references

- [D3 index](index.md): resolved decisions.
- [ADR-0037](../../../../../decisions/0037-exit-code-taxonomy.md): codes
  0–7, mapped in `gage-cli`, `gage:` exactly once. This milestone makes
  the code match it.
- [doc/cli/exit-codes.md](../../../../../cli/exit-codes.md): what maps to
  each code. Every row must still hold.
- [ADR-0017](../../../../../decisions/0017-sync-classify.md): sync failure
  classification, which `syncerr` implements.
- [ADR-0033](../../../../../decisions/0033-recipient-prompt.md): the
  recipient prompt, the largest `Prompter` user.
- CLAUDE.md, "Typed, distinguishable errors at library boundaries".

## Where things are

- **Codes.**
  - `crates/gage/src/exitcode.rs`: `Code` (0–7, with `as_i32`), `HasCode`,
    `code_of` and `all()`.
  - 39 library impls of `HasCode`, plus two in `gage-cli`
    (`commands/recovery.rs:170,190`).
  - `gage-cli/src/error.rs`: `CliError { code, err: anyhow::Error }`,
    `From<E: HasCode>` and `ERROR_PREFIX`.
  - `main.rs:76` calls `exit(code.as_i32())`.
- **Prefixes.**
  - 48 `#[error("gage: …")]` attributes, plus hand-written `Display` in
    `remotesync.rs:318-342` and `syncerr.rs:66-75`.
  - `git2::Error::from_str("gage: …")` at `gitrepo/remote.rs:409,445`.
  - 34 module-prefixed `#[error]` strings.
- **Hints.** 18 `` `gage …` `` command hints, in:
  - `remoteauth.rs:41,61,66,383`
  - `remotesync.rs:211-224,338`
  - `trustcache.rs:81,163`
  - `vault.rs:411,612`
  - `enrollment/mod.rs:212-291`
  - `enrollment/approve.rs:33`

  Flag hints (`--force`, `--reencrypt`, `--device`, `--yes`, `--name`,
  `--replaces`, `--to-vault`, `--field`) are spread across `recipient.rs`,
  `entry/crud.rs`, `recovery` and `enrollment`.
- **Warnings.**
  - `Prompter::warn(&str)`. Its contract (`prompter.rs:245`) says
    messages name gage themselves.
  - It has about 13 library callers, which pass `gage: …` text.
- **Strips.**
  - `prefix_once` (`gage-cli/src/error.rs:96`) has 5 callers.
  - Hand strips: `repl.rs:108,214`, `commands/init.rs:314`,
    `commands/recoverykey.rs:337`, `recipient.rs:191`,
    `trustcache.rs:153`, `remotesync.rs:654-660`.
  - `repl.rs:214`'s `script_error` rebuilds the error with
    `CliError::new`, which loses the typed chain.
- **Downcasts.**
  - `syncerr.rs:149` probes the chain for its own `Error`.
  - `syncerr.rs:152,188` probe for `git2::Error`.
  - `gage-cli/src/error.rs:44` has `CliError::downcast_ref`, used by
    tests.
- **`session::OpenError`** (`session.rs:52-85`) holds a code plus a
  `Box<dyn Error>`, and its `source()` skips the boxed error. It is built
  only by `gage-cli`'s `open_registered` (`commands/mod.rs:203`), from
  `config::Error`, `config::RegistryError` and `vault::ResolveError`.

## Decisions to make first

1. **Names.** Proposed:
   - The library has `gage::error::ErrorKind` with variants `Conflict`,
     `Usage`, `NotFound`, `Ambiguous`, `LockedOrAuth`, `Internal` and
     `Unreachable`. It has no `Success` and no discriminant values.
   - The trait is `HasKind { fn kind(&self) -> ErrorKind; }`.
   - `Usage` is kept as a kind. A library error such as `UnknownMethod`
     or a mutually exclusive `--force`/`--reencrypt` request is invalid
     input from any frontend.
   - `gage-cli` gets `exitcode.rs` with `ExitCode` (`Success` plus the
     seven), `From<ErrorKind>`, `as_i32` and `name`. The `0..=7` pinning
     test moves with it.
2. **The `Remedy` type.** The proposed shape is a non-exhaustive library
   enum with data, and an accessor `HasKind::remedy(&self) ->
   Option<Remedy>` that defaults to `None`.

   Settle the variant list by walking the 18 command hints and the flag
   hints. Starting point:
   - `Sync`
   - `RecipientVerify { repair: bool }`
   - `AuthLogin { host }`
   - `SetRemote`
   - `VaultRemoveOrClone { name }`
   - `IdentityEnroll`, `RecoveryEnroll`
   - `Push`
   - `Force`, `Reencrypt`, `Yes`
   - `ChooseDevice`, `ChooseName`, `Replaces`, `ToVault`, `Field`

   Record the final list and each variant's CLI rendering here. A hint
   whose text needs data the error doesn't carry yet gets that data as a
   field, not a string.
3. **Warnings.** Proposed: `Prompter::warn(&Warning)`, where `Warning` is
   `{ text, remedy: Option<Remedy> }`.
   - The terminal prompter renders `gage: {text}`, then the remedy line.
   - `FakePrompter::warnings()` keeps returning rendered strings, so the
     assertions on it change only where a prefix moves.

   Alternative: keep `warn(&str)` and have warnings stop naming commands.
   Decide by counting the warnings that carry a hint: the `trustcache`
   summary, `remotesync`'s offline notes, `recipient.rs:425` and
   `identityfile.rs:134`.
4. **What "exactly once" means for multi-line output.**
   `trustcache.rs:163` puts a second `gage:` mid-message, on its fix-it
   line. Proposed: the CLI prefixes the first line, and renders the
   remedy on its own line with `gage: fix this with …`. stderr stays
   byte-identical. Pin it with a test before the change.
5. **`git2` probes.** `syncerr`'s `git2::Error` downcasts classify a
   foreign error by class and code (ADR-0017). `gitrepo::Error` wraps
   `git2::Error` but doesn't expose it as a matchable variant everywhere.

   Proposed:
   - The self-probe for `syncerr::Error` goes. Callers that already hold
     a classified error pass it through typed.
   - The `git2` probe is replaced by a match on a `gitrepo::Error::Git`
     variant wherever the chain is a `gitrepo::Error`.
   - Any probe that must stay, because the error crosses a `dyn` boundary
     from `git2` callbacks, is listed here with that reason.
6. **`CliError::downcast_ref`.** It is used by tests only. Proposed: keep
   it in `gage-cli`, where `anyhow` is allowed, with a doc comment saying
   it's a test aid. The roadmap's `errors.Is` concern is about the
   library.

## Tasks

### 1. Pin today's stderr and exit codes

- [ ] Golden stderr tests first, through `gage_cli::run`, one for each:
  - a module-prefixed error (`vaultconfig` format version, `atomicfile`)
  - a hint-bearing error (`auth login`, `sync`, `recipient verify`)
  - the multi-line trust-cache refusal
  - a warning with a hint
  - a script-mode error (`commands.gage:3: …`)
  - one error per exit code 1–7

  These record today's bytes. In later tasks a test changes only where
  the module prefix is deliberately dropped, and the diff shows exactly
  that.

### 2. `ErrorKind` in the library, `ExitCode` in the CLI

- [ ] Tests first. In `gage-cli/src/exitcode.rs`:
  - every `ErrorKind` maps to its documented number
  - `Success` is 0
  - the names are unchanged

  The CLI's existing round-trip test now goes through `ErrorKind`.
- [ ] Rename `HasCode` → `HasKind` and `code` → `kind` across the 39
      library impls and the two CLI impls. `exitcode.rs` leaves the
      library, and `code_of` and `all()` move or go.
- [ ] `CliError` holds an `ExitCode`. `From<E: HasKind + Error>` maps it.
      `main.rs` is unchanged apart from the type.
- [ ] `doc/cli/exit-codes.md` and CLAUDE.md's module list name the new
      homes.

### 3. Typed `OpenError` and the `syncerr` downcasts

- [ ] Tests first:
  - `OpenError::Registry(NotRegistered)` is matchable
  - `source()` reaches the inner error
  - the CLI's exit code for each variant is unchanged
- [ ] `session::OpenError` becomes `enum { Config(config::Error),
      Registry(config::RegistryError), Resolve(vault::ResolveError) }`.
      `kind()` delegates to the inner error. `open_registered` and the
      test openers construct variants.
- [ ] `syncerr` per decision 5. The existing classification tests
      (`syncerr_tests.rs`, `remotesync_tests.rs`) stay green unedited.
      URL-userinfo redaction is still applied to every message that can
      carry a remote URL. Add a test that a remedy-rendered message with
      a `user:token@` remote is still redacted.

### 4. `Remedy`

- [ ] Settle decision 2's list. Tests first: each variant renders to
      exactly today's hint text in `gage-cli`.
- [ ] Each hint-bearing variant drops the command text from its
      `#[error]` and returns its `Remedy`. `CliError` renders `{message}`
      plus the remedy as today's text did, including the separator
      (`; run …`, `. Run …`, a new line).
- [ ] `enrollment/mod.rs:290`'s `msg.contains("gage auth login")` becomes a
      match on the typed remedy.
- [ ] Task 1's hint tests are green and unedited.

### 5. Prefixes out of the library

- [ ] `#[error]`: remove `gage:` and the module prefixes. A message that
      only made sense with its module name is reworded so it says what it
      is about, e.g. "unsupported format version 3 in .gage/config.toml".
- [ ] Hand-written `Display` (`remotesync.rs`, `syncerr.rs`) and
      `gitrepo/remote.rs`'s `from_str("gage: …")` follow.
- [ ] Library hand-strips (`recipient.rs:191`, `trustcache.rs:153`,
      `remotesync.rs:660`) go, now that nothing is prefixed.
- [ ] `Prompter::warn` per decision 3. The library warns without
      `gage:`, and the terminal prompter adds it. The `prompter.rs:245`
      contract comment says so.
- [ ] Commit messages are untouched: `format_v2_commit_layout` and the
      `merge_tests.rs` commit assertions stay green unedited.

### 6. `gage:` once, in the CLI

- [ ] Tests first: a library error, a CLI-made error, a context-wrapped
      error and a script error each render with exactly one leading
      `gage:`.
- [ ] `CliError`'s rendering adds `ERROR_PREFIX` once. `prefix_once` and
      the CLI hand-strips go: `repl.rs:108,214`, `init.rs:314`,
      `recoverykey.rs:337`.
- [ ] `script_error` wraps the typed error with context instead of
      rebuilding it from a string, so the exit code still comes from the
      original error. Test that a script-mode `NotFound` still exits 3.
- [ ] CLI-made errors (`CliError::new(code, "gage: …")`, about 167
      literals) drop their own `gage:`. Success output on stdout
      ("gage: inserted …") is not an error and stays as it is.

### 7. Update the text-dependent tests

- [ ] Every assertion that changes moves from the prefixed string to the
      new sentence, never to a looser `contains`. List the files touched
      in the PR description.
- [ ] The help snapshots and `format_v2_*` tests are unedited.

### 8. ADR and guard

- [ ] ADR-0054 *The library's errors are frontend-neutral*:
  - no binary or module name in text
  - `ErrorKind` instead of exit numbers
  - `Remedy` instead of command text
  - warnings follow the same rules
  - commit messages are format, not UI

  It cites ADR-0037 for the numbers. Link it from the README.
- [ ] A guard test in `crates/gage/tests/`, std-only, after
      `no_frontend_deps.rs`'s pattern. It fails on these, outside
      `_tests.rs` files and the commit-message constants it allowlists
      by name:
  - `#[error("gage:`
  - `` `gage `` in a string literal
  - `"gage: ` passed to `warn`

  Prove it fails by hand once, and record that here.

## Definition of done

- Every box above is ticked, and decisions 1–6 are recorded here as
  settled.
- Every row of `doc/cli/exit-codes.md` still holds, and task 1's
  exit-code tests are green unedited.
- `make lint`, `make test` and `make qa` are clean. The help snapshots and
  `format_v2_*` tests are green and unedited.
- CI is green on Linux, macOS and Windows.
