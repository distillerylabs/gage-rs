# RK3 — `init` gains the key; `recovery rotate`

[← RK2](rk2-recovery-swap-library.md) · [P8 plan](index.md) · [#77](https://github.com/distillerylabs/gage-rs/issues/77) · next: [RK4 — `recovery enroll`, `recovery verify`, hints, docs](rk4-enroll-verify-and-docs.md)

> **Recommended model: Opus ⚑.** This is the milestone where a private key
> that can read everything is shown to a human and written to disk. It has
> to happen in the right order: never before the vault is committed, never
> on a failure path, never into a log or a directory gage deletes from.
> Every bug Go's R1 review found was an ordering or destination bug, and
> each one was quiet. Run an **Opus review of the hygiene,
> show-after-commit, both-terminals, destination-guard and held-error rows**
> (task 8) before calling it done.

## Goal

### `gage init`

`gage init` generates the recovery key by default and registers it as
`recovery-paper-key`. It delivers the key once, after the vault exists, and
never stores it. The key is delivered in one of two ways:

- **On the screen:** text and a QR on stderr, the custody warnings, then a
  six-character re-type.
- **To a file:** a new 0600 file named by `--recovery-key-out`.

`--no-recovery-key` opts out.

### `gage recovery rotate`

`gage recovery rotate` replaces the key, or adds the first one, from an
unlocked device. It delivers the new key through the same path as `init`.

### Why they ship together

Rotate ships with `init` for two reasons:

- They share the delivery path.
- Every failure message on that path tells the user to run
  `gage recovery rotate` ([Q-P8-SPLIT](../open-questions.md#q-p8-split)).

### The order in `init`

Nothing irreversible happens before it must:

1. **Refusals that need nothing created, before any prompt:**
   - the flags' mutual exclusion
   - the terminal rule
   - the `--recovery-key-out` destination guard
   - the reserved device label, unconditionally
   - `validate_without_device_key`
2. **The new passphrase,** then the device identity.
3. **`new_recovery_key`,** then `Vault::create` with the key as a labelled
   extra.
   - If `create` fails, only the identity is rolled back.
   - The recovery key needs no rollback, and has not been shown.
4. **Global registration.**
5. **Delivery, with any error held:**
   - the screen path shows the key and asks for the re-type
   - the file path writes the file
6. **The summary,** including one line naming the recovery recipient's
   public key.
7. **`--remote`'s publish, still run,** then exit.
   - If delivery failed, the held delivery error sets the exit code, even
     if the push failed too.
   - A push failure is reported alongside it.

## Depends on

- **RK1:** `new_recovery_key`, `CreateSpec.extra_recipients`, and the
  reservation.
- **RK2:** `rotate_recovery_key`.
- **P6 · M12c:** `qr::render_qr` (returns `Zeroizing<String>`).
- **P1 · M0b:** `atomicfile` and `xdgpaths::Dirs`.
- **P4 · M8a2:** `init`'s `publish`, and the `--remote` path.
- **P2:** `Prompter::value`, the masked read the re-type goes through.

## Design references

- TDD "Recovery key at `init`", bullets 1–5. TDD `gage init`'s flag list.
- Go plan `recovery-key/index.md`: D1, D2, D3, D5, D7, D11.

## Go sources

- Go plan `recovery-key/r1-init-integration.md`, all of it. It settles:
  - N = 6, case-insensitive and trimmed, with 3 attempts
  - a refused confirmation keeps the vault, exits `Conflict` and doesn't
    reprint the key
  - both terminals are required
  - no clobber

  and three review findings:
  - the destination guard
  - the held delivery error
  - the writability probe

  It also records the accepted gap about page-locking.
- Go plan `recovery-key/r3-recovery-identity-library.md`, "Found by
  review", second bullet: a relative `--recovery-key-out` skipped the
  inside-a-vault guard.
- Go plan `recovery-key/r5-recovery-rotate-and-docs.md`:
  - tasks 1–2
  - "Settled while implementing": `plan_recovery_key` takes no opt-out
    flag for rotate, and messages name `recovery rotate`
  - "Found by review": the destination check moves ahead of the unlock,
    and `init`'s label check becomes unconditional
- Parity ledger:
  - §1 #9 (PR #14) and the CLI half of #18 (PR #22)
  - §4 `init`'s two flags, `recovery`, `recovery_rotate`
- Go sources at the pinned SHA:
  - `cmd/gage/recoverykey.go`
  - `cmd/gage/init.go`
  - `runRecoveryRotate` in `cmd/gage/recovery.go`
- Help fixtures: `init.txt`, `recovery.txt`, `recovery_rotate.txt`.
- Test inventory, 30 rows:
  - `cmd/gage/init_recoverykey_test.go` (21)
  - from `cmd/gage/recovery_enroll_test.go`: the 8 `TestRecoveryRotate*`
    rows and `TestNoCommandLetsADeviceTakeTheRecoveryLabel` (1). RK4 adds
    that last row's `recovery enroll` subtest.
- Inbound: `init`'s flags and help (Q-RECOVERY-INTERIM), the `recovery`
  parent's help, and `init_help_describes_this_milestone`'s flipped
  assertion (see the [index](index.md#inventory-accounting)).

## Decisions to make first

Settle these in this doc, and get them reviewed, before writing code.

1. **`Io.stderr_is_terminal`**
   ([Q-RECOVERY-TERMINAL](../open-questions.md#q-recovery-terminal)).
   - It is set in `main.rs` from `std::io::stderr().is_terminal()`.
   - Every `Io` constructor in tests sets it. The shared `Env` gains a way
     to run "with a terminal", which sets both flags.
   - Screen delivery needs both flags. There is no fallback from one to
     the other, as Go's `IsErrTerminal` had.
2. **How the 190 existing `init` calls keep working**
   ([Q-RECOVERY-TEST-INIT](../open-questions.md#q-recovery-test-init)).
   - `Env::init` and `sync_tests::init_with_remote` append
     `--no-recovery-key` unless the test asks for a key.
   - The roughly 70 literal `["init", …]` calls state the flag explicitly.
   - `tests/smoke.rs` and `tests/entries.rs` pipe stdin, so they state it
     too.
   - A shared helper, `init_vault_with_recovery_key_file(env, name)`, runs
     `init --recovery-key-out <tempdir>/rk` and returns the key, mirroring
     Go's. RK4 needs it heavily.
3. **`atomicfile::write_new`**
   ([Q-RECOVERY-NOCLOBBER](../open-questions.md#q-recovery-noclobber)).
   - It writes a temp file in the same directory, fsyncs it, chmods it to
     0600, then calls tempfile's `persist_noclobber`.
   - An existing destination gives its own variant,
     `atomicfile::Error::AlreadyExists`, whose `source` has
     `io::ErrorKind::AlreadyExists`.
   - It lives in the library's `atomicfile`. It is a file primitive, not a
     frontend concern.
   - The CLI keeps Go's up-front existence check. The atomic refusal is the
     backstop for a file appearing in between.
4. **Where the delivery code lives.** *Settled at the start of RK3, as
   proposed.*
   - `crates/gage-cli/src/commands/recoverykey.rs`, shared by `init`,
     rotate and RK4's enroll.
   - `plan_recovery_key(io, skip_flag: Option<&str>, no_key, out)` returns
     a `KeyPlan`: `None`, `Show` or `File(path)`.
     `check_recovery_key_out_path` returns the destination made absolute
     against `cwd`, and the caller puts that back into `File`, so the
     write lands where the check looked.
   - `skip_flag` is the opt-out flag named in the refusal:
     `--no-recovery-key` for `init`, `--no-new-recovery-key` for enroll,
     `None` for rotate.
5. **The guard's notion of "gage's own directories"**
   ([Q-RECOVERY-DATA-DIR](../open-questions.md#q-recovery-data-dir)).
   The guard refuses three places:
   - the vault being created
   - any ancestor directory holding `.gage/config.toml`
   - anything under `Dirs.data`

   Before checking:
   - the path is made absolute against a `cwd: &Path` parameter. Commands
     pass `std::env::current_dir()`; tests pass a directory of their own, so
     no test changes the process's working directory.
   - symlinks are resolved as far as the path exists (Go's
     `resolveExisting`)

   On macOS, `/var` and `/private/var` must compare equal. Windows paths
   compare per component; the guard must not use a string-prefix test.

## Tasks

Write each task's tests first. CLI rows go in
`crates/gage-cli/src/init_recoverykey_tests.rs` and
`crates/gage-cli/src/recovery_cli_tests.rs`.

### 1. Fixtures and the existing suite

- [x] `Io.stderr_is_terminal` (decision 1), and every `Io` constructor
  updated.
- [x] `Env::init` and `init_with_remote` default to `--no-recovery-key`. The
  literal `init` calls state it.
- [x] `init_vault_with_recovery_key_file` is added (decision 2).
- [x] `lifecycle_tests::init_help_describes_this_milestone` flips: the
  help now names the recovery key. `init.rs`'s "the recovery key is P8's"
  note goes.
- [x] The whole existing suite is green before any recovery behavior lands.
  This step is mechanical and wide, as Go's R1 found. It lands as its own
  first task, so the diff that follows shows only behavior.

### 2. `write_new` and the destination guard — 7 rows

- [x] `atomicfile::write_new` (decision 3), with library tests:
  - `write_new_refuses_an_existing_file`
  - `write_new_creates_a_0600_file` (Unix)
- [x] `check_recovery_key_out_path(path, cwd, vault_path, dirs)`. Order matters:
  1. the guard that refuses gage's own directories, first, since it is the
     only check that holds when nothing on the path exists yet
  2. the existence check
  3. the parent exists and is a directory
  4. the writability probe: create and remove a temp file
- [x] Rows:
  - `TestInitRefusesToWriteTheRecoveryKeyWhereGageWillDeleteIt`: `init_refuses_to_write_the_recovery_key_where_gage_will_delete_it`. Three cases: inside the new vault, inside another (unregistered) vault, and under the data dir. Each is `Usage` with nothing created.
  - `TestRecoveryKeyOutRejectsARelativePathInsideAVault`: `recovery_key_out_rejects_a_relative_path_inside_a_vault`. It passes a relative path and a `cwd` inside a vault (decision 5), never `set_current_dir` in a threaded test.
  - `TestRecoveryKeyOutRejectsAParentThatDoesNotExist`: `recovery_key_out_rejects_a_parent_that_does_not_exist`
  - `TestRecoveryKeyOutRejectsAParentThatIsARegularFile`: `recovery_key_out_rejects_a_parent_that_is_a_regular_file`. *Tightened, as Go's test asks:* Go's existence stat took `ENOTDIR` for an error and answered `Internal`, so its test leaves the code unpinned. Here `NotADirectory` counts as "does not exist", so the parent check answers with `Usage` and "is not a directory".
  - `TestInitChecksTheRecoveryKeyDestinationIsWritableBeforeCreating`: `init_checks_the_recovery_key_destination_is_writable_before_creating`. A read-only directory on Unix; on Windows, `#[cfg(unix)]` with a stated reason, as Go's test skips there.
  - `TestInitRecoveryKeyOutRefusesToClobberAnExistingFile`: `init_recovery_key_out_refuses_to_clobber_an_existing_file`. The file is untouched and nothing is created.
  - `TestPathIsInsideWithNoRootIsNotInside`: `path_is_inside_with_no_root_is_not_inside`
- [x] Beyond Go:
  - `path_is_inside_sees_through_a_symlinked_tempdir` (macOS `/var`, plus a real symlink on Unix)
  - `path_is_inside_is_not_fooled_by_a_shared_prefix` (`/a/vault2` is not inside `/a/vault`)
  - `an_accepted_destination_is_made_absolute_against_the_given_cwd`
  - `write_new_creates_the_file_and_leaves_no_temp_file` (library)

### 3. `init`'s planning and refusals — 6 rows

- [x] `--no-recovery-key` and `--recovery-key-out FILE`, with their help
  text from `init.txt`.
- [x] `--recipient`'s help no longer reads as the recovery mechanism, and
  the long help states the new default.
- [x] `plan_recovery_key`:
  - both flags together is `Usage`
  - with no terminal on both streams and neither flag, `Usage`, naming
    both flags
- [x] The label check is unconditional, ahead of the passphrase prompt.
- [x] Rows:
  - `TestInitWithoutATerminalRefusesRatherThanLoggingTheKey`: `init_without_a_terminal_refuses_rather_than_logging_the_key`. `Usage`, no prompt asked, no vault, no identity.
  - `TestInitWithRedirectedStderrRefusesToo`: `init_with_redirected_stderr_refuses_too`. stdin is a terminal and stderr is not.
  - `TestInitRejectsRecoveryKeyOutWithNoRecoveryKey`: `init_rejects_recovery_key_out_with_no_recovery_key`
  - `TestInitRejectsADeviceNamedLikeTheRecoveryLabel`: `init_rejects_a_device_named_like_the_recovery_label`. `Usage`, before the passphrase prompt, with `--no-recovery-key` too. This is R5's flipped version.
  - `TestInitNoRecoveryKeyOptsOut`: `init_no_recovery_key_opts_out`. One recipient, no key output.
  - `TestInitShowsNoRecoveryKeyWhenCreateFails`: `init_shows_no_recovery_key_when_create_fails`. No key on either stream, and no identity or vault left behind.

### 4. Generation, registration and delivery — 8 rows ⚑

- [x] `init` generates the key after the device identity, and passes it as
  a labelled extra.
- [x] `show_recovery_key` writes these to stderr, in order:
  1. a blank line
  2. "… now has a recovery key. This is the only time it will be shown."
  3. the key, indented
  4. the QR, from `render_qr`; a render failure only warns
  5. the custody warnings: unencrypted, full access, no copy kept, store
     offline on paper or an offline encrypted drive, never cloud notes, a
     password manager, email or photos, and clear the scrollback
- [x] `confirm_recovery_key_saved`:
  - `Prompter::value` asks for the last 6 characters
  - the comparison is case-insensitive and trims whitespace
  - up to 3 attempts, warning between them
  - after that, `Conflict`, with the message that the change is committed,
    the key is a recipient, and `gage recovery rotate` replaces it. The key
    is not reprinted.
- [x] `write_recovery_key_file`:
  - writes the key plus a newline through `write_new`
  - the buffer is a `Zeroizing<Vec<u8>>`
  - prints a stderr note: not encrypted, move it offline, delete this file
  - on failure, `Internal`, naming `gage recovery rotate`
- [x] Delivery errors are held, the summary and `publish` still run, and the
  held error wins over a push failure.
- [x] The summary gains a line naming the recovery recipient's pubkey.
- [x] Rows:
  - `TestInitRegistersARecoveryRecipientByDefault`: `init_registers_a_recovery_recipient_by_default`. `<device>` and `recovery-paper-key`, in both files.
  - `TestInitRecoveryKeyAloneDecryptsTheVault`: `init_recovery_key_alone_decrypts_the_vault`
  - `TestInitRecoveryKeyIsNeverStored`: `init_recovery_key_is_never_stored` ⚑.
    - The key is absent from every file under the three XDG roots and the vault tree, and from every git object read through the object database, so compression can't hide it.
    - It appears exactly once across stdout and stderr.
  - `TestInitRecoveryKeyIsShownWithItsWarningsAndAQR`: `init_recovery_key_is_shown_with_its_warnings_and_a_qr`. QR glyphs and each warning phrase are present, and all of it is on stderr.
  - `TestInitRecoveryKeyOutWritesAPrivateFileAndNothingToStdout`: `init_recovery_key_out_writes_a_private_file_and_nothing_to_stdout`. 0600 on Unix, no secret on stdout, no prompt for a re-type.
  - `TestInitRecoveryKeyConfirmationAsksForTheKeysOwnCharacters`: `init_recovery_key_confirmation_asks_for_the_keys_own_characters`
  - `TestInitRecoveryKeyWrongConfirmationRetriesThenFailsWithoutReshowing`: `init_recovery_key_wrong_confirmation_retries_then_fails_without_reshowing`. `Conflict`; the vault exists and is registered; the key appears once.
  - `TestInitStillPublishesWhenTheRecoveryConfirmationFails`: `init_still_publishes_when_the_recovery_confirmation_fails`. The `gittest` remote has the commit and the exit is `Conflict`.
- [x] Beyond Go:
  - `init_key_is_shown_only_after_the_commit_exists`. The prompter's re-type answer checks HEAD in the vault at the moment it is asked.
  - `init_recovery_key_confirmation_ignores_case_and_whitespace`
  - `gittest::all_object_data_finds_what_a_byte_search_of_dot_git_misses` (library): proves the hygiene walk's object read sees a committed string that a byte search of `.git` misses
  - `init_delivery_error_outranks_a_failed_push`. The exit code is the delivery error's, and both are reported. *As built:* both failures are real rather than faked. The file write fails because a file appears at the destination while the passphrase is being typed (a prompter hook), which `write_new`'s backstop refuses. The push fails because the remote is a missing local path. No new seam was needed, so none was added. A second half pins a refused re-type over a failed push the same way.

### 5. The `recovery` parent and `recovery rotate` — 8 rows

- [x] Registry entries:
  - a `recovery` `ParentInfo` with `recovery.txt`'s short text
  - `recovery rotate`: group Recipients, one-shot only, `--recovery-key-out`
    and `-u/--use`, long help from `recovery_rotate.txt`
- [x] `commands/recovery.rs`'s rotate runs in this order:
  1. `plan_recovery_key(skip_flag: None)`
  2. in file mode, resolve the vault without unlocking and check the
     destination, **before** the unlock (R5's review)
  3. unlock
  4. `new_recovery_key`
  5. `rotate_recovery_key`
  6. delivery, with any error held
  7. the summary: added a key (it had none), or replaced it and retired
     `<pubkey>`; the new pubkey; the count re-encrypted
- [x] Rows (`recovery_enroll_test.go`):
  - `TestRecoveryRotateReplacesTheKey`: `recovery_rotate_replaces_the_key`. One commit, the old key retired, the new one shown.
  - `TestRecoveryRotateAddsOneToAVaultWithout`: `recovery_rotate_adds_one_to_a_vault_without`
  - `TestRecoveryRotateWithoutATerminalRefusesBeforeAnyPrompt`: `recovery_rotate_without_a_terminal_refuses_before_any_prompt`. The refusal names `--recovery-key-out` only: rotate has no opt-out flag.
  - `TestRecoveryRotateChecksTheKeyDestinationBeforeTheUnlock`: `recovery_rotate_checks_the_key_destination_before_the_unlock`. No unlock request is recorded.
  - `TestRecoveryRotateKeyOutWritesAPrivateFile`: `recovery_rotate_key_out_writes_a_private_file`
  - `TestRecoveryRotateKeepsNoCopyOfEitherKey`: `recovery_rotate_keeps_no_copy_of_either_key`. The same walk as `init_recovery_key_is_never_stored`, for both the old key and the new one.
  - `TestRecoveryRotateStillPublishesWhenTheConfirmationFails`: `recovery_rotate_still_publishes_when_the_confirmation_fails`
  - `TestRecoveryRotateIsListedInHelp`: `recovery_rotate_is_listed_in_help`. In `gage help` and in `gage recovery --help`.

### 6. The reservation through the CLI — 1 row

- [x] Row:
  - `TestNoCommandLetsADeviceTakeTheRecoveryLabel`: `no_command_lets_a_device_take_the_recovery_label`. Subtests: `init`, even with `--no-recovery-key`, is `Usage` with no `.gage`; `identity add` is `Usage`; `recipient add` is non-zero. RK4 adds the `recovery enroll` subtest.
- [x] Beyond Go: `every_command_init_and_rotate_messages_name_is_registered`.
  - It collects every `gage <words>` that this milestone's messages can
    print: the terminal refusal, the delivery failures, the refused
    re-type.
  - Each must resolve in the registry.
  - This is `TestNoRecoveryKeyWarningNamesARealCommand`'s guard, applied
    to `init`'s and rotate's messages. That row itself is RK4's.

### 7. Help parity and smoke

- [x] `p8_init_and_rotate_have_their_go_fixture_flags_and_descriptions`, on
  `approve_cli_tests.rs`'s pattern:
  - `init`'s two new flags and their descriptions
  - `recovery rotate`'s flags, usage, group and availability
  - the `recovery` parent's child list, containing the children that exist
- [x] A real-binary smoke test (`assert_cmd`): `init` with piped stdin
  refuses with `Usage`, and the data dir stays empty.

### 8. Opus review ⚑

- [x] An Opus pass over tasks 2–5's tests, recorded here as a table: what
  it checked, and what it found.
  - Does the hygiene walk really reach decompressed objects?
  - Is the key shown only after the commit exists?
  - Does stderr-not-a-terminal refuse alone?
  - Does the guard run before any file is created, including for relative
    paths and symlinked temp dirs?
  - Do the held error and publish behave together as Go's R1 review
    settled?
  - Is every secret buffer `Zeroizing`?

#### Review record

Done in-session by the implementing model (Opus), as a separate pass over
tasks 2–5 once they were green. It was not a separate agent. Where a
question could be answered by breaking the code, it was: each mutation
below was applied, its test watched fail, and the change reverted.

| Question | What was checked | Found |
|---|---|---|
| Does the hygiene walk reach decompressed objects? | `assert_never_stored` reads every object through `git2`'s object database (`gittest::all_object_data`); a guard test proves a committed string is found that way and *not* by a byte search of `.git`. Mutation: commit the key into the vault and remove it again, so it lives only in history. | Caught (`init_recovery_key_is_never_stored` fails with "is in a git object"). The walk also covers the three XDG roots and the vault tree, case-insensitively. |
| Is the key shown only after the commit exists? | `deliver_recovery_key` runs after `Vault::create` and `config::save`; `init_key_is_shown_only_after_the_commit_exists` asserts one commit, a clean tree and the registration at the moment of the re-type. The file path writes from the same point. | Holds. |
| Does stderr-not-a-terminal refuse alone? | `plan_recovery_key` requires both flags. Mutation: require only `stdin_is_terminal`. | Caught (`init_with_redirected_stderr_refuses_too`). The real binary refuses a piped run too (smoke test). |
| Does the guard run before any file is created, relative paths and symlinked temp dirs included? | Every refusal row runs with an unscripted prompter (a passphrase request panics) and compares the data, state and config trees before and after. Paths are made absolute against `cwd`, folded lexically, and resolved as far as they exist before comparison; the ancestor walk uses the resolved path. | Holds. **Found:** `--recovery-key-out ""` named the working directory and was refused as "already exists". Now refused as empty ([Q-RECOVERY-EMPTY-OUT](../open-questions.md#q-recovery-empty-out), `an_empty_recovery_key_out_is_refused_as_empty`). |
| Do the held error and publish behave together as Go's R1 review settled? | Delivery's result is held; the summary and `publish` run; the delivery error wins over a failed push, which is printed beside it. Tested for a refused re-type (with a working and a missing remote) and a failed file write. | Holds. Rotate holds its delivery error the same way, and its swap has already pushed by then. |
| Is every secret buffer `Zeroizing`? | `RecoveryKey.secret`; the key file's `Zeroizing<Vec<u8>>`, allocated at its final size so it never reallocates; `render_qr`'s output; the re-type answer from `Prompter::value`. The secret reaches stderr through `write_all` straight from its own buffer, never through `format!`. | **Found:** the `qrcode` crate's own buffers (data and module grid) encode the secret and are dropped without being zeroed. They aren't reachable to wipe, and the gap predates P8 (`show -q`). Recorded under [Q-RECOVERY-SECRET-MEMORY](../open-questions.md#q-recovery-secret-memory), not fixed. |

### 9. Parity and ledger

- [x] All 30 rows ported, or struck with a reason in this doc.
- [x] Ledger §1 #9, #16 and #18 `[x]` (#18's docs are RK4's, so note them), and §4 `init`'s flags, `recovery`
  (rotate) and `recovery_rotate` updated.
- [x] `Q-RECOVERY-INTERIM` marked superseded.
- [x] `make lint` and `make test` (1,632 tests); CI on Linux, macOS and Windows (PR [#82](https://github.com/distillerylabs/gage-rs/pull/82), commit 1dccbef: lint 39101227859, test 39101227844 and audit 39101227839 all pass).

## Definition of done

- Every box above is checked and all 30 claimed rows are ported.
- RK1's, RK2's and P1–P7's suites are still green.
- `make lint` and `make test` are clean locally and on all three CI
  platforms.
- A recorded real-terminal walkthrough of the real binary in a scratch XDG
  root confirms four things, as Go's R1 did:
  - a piped `init` refuses
  - `init --recovery-key-out` writes a 0600 file whose key appears nowhere
    under any XDG root
  - an entry inserted afterwards decrypts with the recovery key alone
    (`age -d -i`)
  - a terminal run shows the key, the QR, the warnings and the re-type
    prompt, in that order

## Walkthrough record

Run against the real binary (`target/debug/gage`, shipped scrypt cost) in
a scratch XDG root, on macOS, while closing RK3:

- **A piped `init` refuses:** exit 2, the message names both flags, and
  the data dir is empty.
- **`init --recovery-key-out` writes a 0600 file** (`-rw-------`). Its key
  is in no file under any XDG root, and in no object of the vault's
  repository (`git cat-file --batch-all-objects --batch`).
- **An entry inserted afterwards decrypts with the recovery key alone:**
  `age -d -i <key file> entries/<uuid>.age` prints the entry's YAML with
  its value.
- **A terminal run** (driven by `expect` on a pty) shows, in order: "This
  is the only time it will be shown", the key, the QR, the warnings, the
  `Value for "the last 6 characters…"` prompt, then the summary with the
  recovery recipient's public key. The key appears exactly once in the
  transcript, and the re-type typed from the screen is accepted (exit 0).
