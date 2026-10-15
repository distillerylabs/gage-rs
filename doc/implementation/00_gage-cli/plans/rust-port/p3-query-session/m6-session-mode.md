# M6 — Session mode

[← M5](m5-query-resolution.md) · [P3 plan](index.md) · [#23](https://github.com/distillerylabs/gage-rs/issues/23) · next: [M7 — Metadata index](m7-metadata-index.md)

> **Recommended model: Sonnet\*.** The `Session` type is well specified
> and the borrow checker settles most of Go's lifetime reasoning. The
> borderline part is the REPL wiring: `rustyline` behind a `LineReader`
> trait, the prompter and the line editor sharing one terminal, and the
> one pty test. Switch to Opus at a milestone boundary if that turns
> awkward.

## Goal

The `Session` library type: one or more vaults' `Identity` values held
for longer than one command, with `use`, `lock`, `status` and an idle
timeout. The REPL is `gage-cli`'s thin terminal rendering of it, reached
by bare `gage` on a terminal. **`Session` adds no memory-protection
mechanics and no `Vault` method gains a session-only variant**: `use`
calls the same `Vault::unlock`, `lock` and the idle timeout drop the same
`Identity`, and commands call the same `Vault` methods they call one-shot.
An ambiguous query in a session asks the human through
`Prompter::choose`. No metadata index yet (M7): a session still decrypts
on demand.

## Depends on

- **M5**: the resolver's `CandidateList`, which the session picker shows;
  every entry command, which the session dispatches.
- **P2 · M2**: `Vault::unlock`, `Identity` (`Drop` wipes and unlocks), the
  terminal prompter, `LockedBuf`'s one-page-per-key allocation.
- **P1 · M0b**: the registry (`Availability`, `session_visible`), the
  usage-error-with-help mechanism, `Outcome::Session` from `cli::parse`.

## Design references

- Design doc "Session model" (the two invocation modes, the REPL
  transcript, no daemon), "Why an idle timeout still matters despite
  process-scoped keys", "Session-only commands", "A few decisions worth
  calling out" (history holds typed queries only).
- Go's Q-ROOT-CMD, Q-HELP-SURFACES, Q-CMD-AVAILABILITY (inherited, in the
  Go register).
- [Q-LINE-EDITOR](../open-questions.md#q-line-editor),
  [Q-SESSION-SHAPE](../open-questions.md#q-session-shape).

## Go sources

- Go plan `gage-cli-design/m6-session-mode.md`; the post-plan #27
  session half (usage rejection inside a session).
- Parity ledger rows: §2 `db2731f` (session half); §3 M6; §4 "Session-mode
  `help` and session-only commands"; §6 `session-mode.md`.
- Go sources at the pinned SHA: `internal/gage/session.go` (less the
  index and `EntryTitleCandidates`, which are M7's and M13's),
  `cmd/gage/repl.go`, `cmd/gage/session.go`, `cmd/gage/shell.go`,
  `cmd/gage/lineread.go`, `cmd/gage/help.go` (session help),
  `cmd/gage/prompter.go` (`Choose`).
- Test inventory: `internal/gage/session_test.go` (20 of 24),
  `cmd/gage/repl_test.go` (27), `cmd/gage/session_test.go` (4),
  `session_pty` (1), `sessionhelp` (9), `shell` (9): 70 rows. Plus 3
  deferred rows (task 7).

## Decisions to make first

Recommended answers; confirm them when this plan is reviewed.

- **`Session` shape** (Q-SESSION-SHAPE): `Session::new(SessionConfig)`
  with `open: Box<dyn Fn(&str) -> Result<Vault, _>>` (resolves a name
  through global config, as Go's `VaultOpener`), the prompter (held by
  reference), `idle_timeout: Option<Duration>`, `current: Option<String>`
  seeded from global config's `current`, and `now: Box<dyn Fn() -> Instant>`
  defaulting to `Instant::now`. Each touched vault is a `HeldVault`
  `{ vault, identity: Option<Identity>, last_used }`; `None` is "locked",
  and a locked vault stays in the map so `status` and the prompt still
  show it.
- **Borrowing instead of Go's closed-identity contract.** Methods that
  need a key return `(&Vault, &Identity)` borrowed from `&mut self`, so a
  command can't hold a key across a `lock`. Go's `ErrIdentityClosed`
  path has no Rust equivalent and its tests are excused where they test
  only that path (none of the 20 claimed rows do).
- **Idle timeout**: evaluated on entering the outermost public call and
  nowhere else (Go's depth rule), plus a `Session::begin_command()` guard
  the REPL takes around one typed line, so a re-lock happens between
  commands and never inside one. A zero or absent timeout disables it.
  Expiry drops the `Identity` through the same function `lock` uses.
  `status` applies expiry first, so the prompt shows a timed-out vault as
  locked without a command running.
- **`[shell]` settings**: `prompt` defaults to `{vault}{lock} gage> `
  with leading spaces trimmed after rendering (Go's
  `defaultPromptTemplate`); `{lock}` renders 🔓/🔒 only when a vault is
  current; `{dirty}` runs `gitrepo::is_clean` only if the template uses
  it, best-effort (any error renders clean). `idle_timeout` defaults to
  10 minutes; parse with `humantime` (already a dependency), accepting at
  least Go's documented forms (`10m`, `90s`, `1h`, `0`); a malformed or
  negative value is a Usage error. Any accepted-form difference from Go's
  `time.ParseDuration` (e.g. `1.5h`) is recorded here when found.
  `history_file` defaults to `$GAGE_STATE/history`, expands `~` and the
  `$GAGE_*` roots, and an empty value disables history.
  `clipboard_timeout` is P6's.
- **Line editing** (Q-LINE-EDITOR): a `LineReader` trait in `gage-cli`
  with `read_line(prompt)`; `PlainLineReader` reads from `Io.stdin`
  (in-process tests, piped input) and `RustylineReader` wraps
  `rustyline::Editor` when stdin is a terminal. Only submitted, non-blank
  lines are added to history, explicitly. The terminal prompter keeps
  reading stdin itself between lines, since `rustyline` releases the
  terminal when `readline()` returns. Ctrl-C abandons the line, not the
  session (`ReadlineError::Interrupted`); Ctrl-D/EOF is `exit`.
- **History file** is created 0600 before `rustyline` first saves to it
  (its own `fix_perm` also covers Unix); an unreadable history file is
  ignored, not fatal (`TestHistoryIgnoresBlankLinesAndUnreadableFiles`).
- **Line splitting**: Go's `splitLine` rules (single and double quotes,
  backslash escapes, an unbalanced quote is a usage error) in a small
  hand-written function; no `shell-words` dependency.
- **Session dispatch** goes through the same clap tree as one-shot, built
  from the registry. Session-only verbs (`use`, `lock`, `status`, `exit`,
  `help`) are handled before clap. `init` (and later `clone`) report
  "one-shot only" plainly. `insert -m` and `--value-stdin` are refused
  in a session, since stdin is the REPL's
  (`TestStdinReadingInsertModesAreRefusedInASession`). A usage rejection
  prints the command's own usage and the session continues.
- **`Prompter::choose(&CandidateList) -> Result<Uuid, PromptError>`**:
  the CLI renders Go's numbered list and `[1-N]` prompt, retries a bad
  answer, and gives up after Go's attempt count. An empty list is
  Internal. `Session` refuses an id that wasn't in the list it offered.
- **`TestSessionAddsNoDuplicateVaultMethods`** becomes a source scan over
  `crates/gage/src/session.rs`: it may call `Vault` methods but defines
  no `impl Vault` and no method duplicating one by name.

### Settled at the start of implementation

Reading Go's sources at the pinned SHA corrected or sharpened some of
the above. These are the decisions the code follows.

- **Line splitting has no backslash escapes.** Go's `splitLine` takes
  quotes as the only grouping, on purpose: a session is where someone
  types `insert C:\keys\prod`, and an escape rule would eat the
  separators on Windows. `TestSplitLineHandlesQuotesAndBackslashes`
  asserts backslashes survive verbatim. The bullet above is wrong on
  this point; Go's rule is ported.
- **`idle_timeout` uses a hand-written parser for Go's
  `time.ParseDuration` grammar**, not `humantime`: `humantime` accepts
  `10min`, which Go and `TestResolveShellSettingsRejectsAMalformedIdleTimeout`
  reject. Accepted: an optional sign, then one or more `<decimal><unit>`
  terms with units `ns`, `us`/`µs`, `ms`, `s`, `m`, `h` (`1.5h`, `1h30m`),
  or a bare `0`. A negative value is its own Usage error.
- **gage owns the history file, as Go's `history.go` does**: created
  0600 (tightened if it already exists), append-only, one typed line per
  submit. `rustyline` keeps only its in-memory recall list, preloaded
  from the file, with default features off (no `with-file-history`).
  `FileHistory::save` rewrites the file wholesale, and a session must
  start even when the file can't be read
  (`TestHistoryIgnoresBlankLinesAndUnreadableFiles`). Amends
  Q-LINE-EDITOR.
- **`Prompter::choose` outcomes.** A pick returns its id. `Cancelled`
  (end of input, Ctrl-C, a GUI's closed picker) is the human declining,
  and `Session::resolve` returns the candidate list as one-shot does:
  Go's nil-prompter case. Running out of attempts is a new
  `PromptError::NoChoice` (Ambiguous); an empty list is a new
  `PromptError::EmptyCandidates` (Internal).
- **The opener's error** is a library `OpenError` that wraps whatever
  coded library error the frontend's lookup failed with, keeping its
  exit code and message.
- **The clock is monotonic `Instant`**, so `VaultState` reports
  `idle: Option<Duration>` (since the vault last served a call) instead
  of Go's wall-clock `LastUsed`; `status` renders it as "last used … ago".
- **Prompts stay on stderr in a session.** Go's REPL points its
  prompter at stdout while a session runs; gage-rs doesn't redirect, so
  the session tests assert on stdout and stderr together where Go
  asserted on stdout alone. Layout, not behavior.
- **Not in M6:** Go's `use` also pulls when the vault is already
  unlocked (`syncOnUnlock`): P4 · M8a. Go's `use` adds a hint naming
  `gage identity enroll`/`gage recovery enroll` to a no-local-identity
  failure: P7, with those commands.
- **`history_file = ""` takes the default path**, as Go's
  `resolveHistoryPath` does; the bullet above saying an empty value
  disables history is wrong. Only an unopenable file disables it (Go's
  `openHistory("")`), reported once.
- **`rustyline` brings BSL-1.0 on Windows** (`clipboard-win`,
  `error-code`). Added to `deny.toml` and recorded under Q-LINE-EDITOR.
- **`usageerror` `TestSessionWrongArgCountShowsCommandUsage`** is driven
  through `--stdin` in Go. It is ported through the interactive session
  (`edit` with no query prints edit's own usage and the session goes
  on); the `--stdin` spelling arrives with P6 · M12.

## Tasks

Tests first, at task granularity.

### 1. `Session` library — 20 of 24 `session_test` rows

- [x] Use and reuse: `TestSessionUseUnlocksOnceThenReusesTheKey`,
      `TestSessionUseWithEmptyNameSaysSo`,
      `TestSessionWithNoCurrentVaultSaysSo`,
      `TestSessionAdHocVaultDoesNotChangeCurrent`.
- [x] Same `Vault` methods: `TestSessionCallsTheSameVaultMethodsAsOneShot`,
      `TestSessionAddsNoDuplicateVaultMethods`.
- [x] Lock and status: `TestSessionLockDropsOneVaultAndLeavesOthersAlone`,
      `TestSessionLockWithNoVaultLocksAll`,
      `TestSessionLockUnknownVaultReports`,
      `TestSessionStatusReportsMultipleVaults`,
      `TestSessionCloseClosesEveryHeldIdentity` (counting `Locker`).
- [x] Idle timeout: `TestSessionIdleTimeoutRelocksAndRePrompts`,
      `TestIdleTimeoutClosesTheIdentityLikeAnExplicitLock`,
      `TestIdleTimeoutDisabledWhenZero`,
      `TestIdleTimeoutIsVisibleToStatusWithoutACommand`,
      `TestIdleTimeoutNeverInterruptsAnInFlightCommand`.
- [x] Resolve: `TestSessionResolveAsksThePrompterOnAnAmbiguousQuery`,
      `TestSessionResolveRefusesAnIDThatWasNeverOffered`,
      `TestResolveAmbiguousWithNoPrompterReturnsCandidates` (a prompter
      whose `choose` declines returns the candidate list, as one-shot).
- [x] `TestConcurrentlyHeldIdentitiesDoNotSharePages`: two identities held
      by one session; dropping the first leaves the second's pages locked
      (counting `Locker`). M2's `LockedBuf` already prevents Go's bug; this
      proves it at the level where Go found it.
- Moved: the 4 `TestEntryTitleCandidates*` rows (M13).
- Beyond Go's rows: an opener failure keeps its code and never prompts; a
  seeded current vault unlocks on first use; a unique query never asks;
  a picker that gives up is Ambiguous; dropping a session drops its keys.
  The page-lock rows use a `PageLedger` locker that keeps the set of
  locked pages as the OS does, so "no key held" and "no shared page" are
  checked on every platform.

### 2. Prompter `choose` — 2 rows deferred by P2

- [x] `Prompter::choose` on the trait, the fake (scripted answers,
      records lists) and the terminal prompter.
- [x] `prompter` `TestChooseOnAnEmptyCandidateListIsInternal`,
      `TestChooseRetriesThenGivesUp`.

### 3. Shell settings — `shell` (9 rows)

- [x] `TestResolveShellSettingsDefaults`,
      `TestResolveShellSettingsParsesIdleTimeout`,
      `TestResolveShellSettingsRejectsAMalformedIdleTimeout`,
      `TestZeroIdleTimeoutDisablesTheRelock`.
- [x] `TestHistoryDisabledWithoutAPath`,
      `TestHistoryFileHonoursAnAbsolutePath`,
      `TestHistoryPathExpandsGageRoots`, `TestExpandGagePathExpandsTilde`.
- [x] `TestVaultIsDirtyIsBestEffort`.

### 4. Entering a session — `cmd/gage/session_test` (4 rows)

- [x] Replace `Outcome::Session`'s "not implemented yet" with the REPL.
- [x] `TestBareGageOnTTYEntersSessionMode`,
      `TestBareGageWithPipedStdinPrintsHelp` (P1's
      `bare_gage_with_redirected_stdin_prints_root_help`),
      `TestSessionOnlyCommandsFailCleanlyOneShot`,
      `TestNoBareUnenumeratedExitCode`.

### 5. The REPL — `repl` (27 rows, the `--stdin` half of one moved)

- [x] Wiring: `TestREPLIsAThinWiringLayer`, `TestSessionSkipsBlankLines`,
      `TestUnknownSessionCommandReportsAndContinues`,
      `TestOneShotOnlyCommandsReportPlainlyInSession`,
      `TestStdinReadingInsertModesAreRefusedInASession`,
      `TestSessionCommandArgumentCountsAreChecked` (interactive half; the
      `--stdin` half moves to P6 · M12 with `--stdin`).
- [x] Line splitting: `TestSplitLineHandlesQuotesAndBackslashes`,
      `TestSplitLineRejectsAnUnbalancedQuote`.
- [x] Unlock and lock: `TestSessionUnlocksOnceForManyCommands`,
      `TestSessionLockThenUseRePrompts`,
      `TestSessionBareLockLocksEverythingItHolds`,
      `TestSessionLockWithNothingUnlockedSaysSo`,
      `TestSessionStatusListsTouchedVaults`,
      `TestSessionAdHocUseFlagDoesNotChangeCurrentVault`.
- [x] Picker: `TestSessionAmbiguousQueryPrompts`,
      `TestSessionRmResolvesAmbiguityThroughThePicker`.
- [x] Exit: `TestREPLEOFTerminatesLikeExit`,
      `TestREPLClosesEveryHeldIdentityOnExit`,
      `TestCtrlCAbandonsTheLineNotTheSession`,
      `TestMapReadlineErrTranslatesCtrlC`.
- [x] Idle timeout from config: `TestConfiguredIdleTimeoutReachesTheSession`,
      `TestConfiguredIdleTimeoutIsNotAppliedTooEagerly` (the clock injected
      at the CLI layer too, as Go's `App.Now`, so neither depends on
      Windows' coarse clock).
- [x] Prompt: `TestRenderPromptTokens`,
      `TestSessionPromptRendersTokensAndTracksLockState`.
- [x] History: `TestSessionHistoryRecordsCommandsNeverValues`,
      `TestSessionHistoryFileIsCreated0600` (Unix mode check),
      `TestHistoryIgnoresBlankLinesAndUnreadableFiles` (Go overflows
      `bufio.Scanner`; the unreadable file here is one that isn't UTF-8).
- [x] Beyond Go's rows: the session's failures join the prefix-doubling
      table (`no_session_failure_doubles_gages_own_prefix`); a malformed
      `idle_timeout` refuses the session; an unopenable history file is
      reported and the session runs; `rename` asks through the picker;
      declining the picker lists the candidates.

### 6. Session help — `sessionhelp` (9 rows)

- [x] `TestSessionHelpListsTheMetaVerbs`, `TestSessionHelpMatchesRegistryExactly`,
      `TestBothHelpSurfacesRenderTheSameRegistry`,
      `TestSessionOnlyCommandsNeverLeakIntoOneShotHelp`,
      `TestInitIsNotOfferedInSession`, `TestSessionHelpLeadsWithBareUse`,
      `TestSessionPerCommandHelpListsTheUseFlag`,
      `TestSessionHelpForAMetaVerbPrintsItsUsage`,
      `TestSessionHelpUnknownTopicDoesNotEndTheSession`.

### 7. Rows deferred by P1, and the real terminal

- [x] `usageerror` `TestSessionWrongArgCountShowsCommandUsage` (P1 · M0b).
- [x] `session_pty` `TestSessionOverARealTerminalDrivesTheLineEditor`
      (Unix only, `portable-pty`, as Go's is `linux || darwin`): bare
      `gage` on a pty reaches `rustyline`, a passphrase prompt in between
      lines is masked, and `exit` ends the process 0. It also recalls a
      line with Up and abandons one with Ctrl-C, which only the editor's
      raw mode does: forcing the plain reader fails the test at the
      recall.
- [x] Add `rustyline` to `gage-cli`; confirm `make audit` (cargo-deny)
      passes (after allowing BSL-1.0, above), and the frontend-deps check
      still keeps it out of `crates/gage`.

### 8. User docs and ledger

- [x] Port Go's `doc/cli/session-mode.md` for the shipped surface (no
      index or completion yet: M7 and M13 extend it).
- [x] Parity ledger: §2 `db2731f` session half; §4 session help row.
- [x] P3 index status column.

## Definition of done

Every box above checked; all 70 claimed rows ported, or moved with the
destination named here; the 3 deferred rows in tasks 2 and 7 done;
`make lint` and `make test` clean locally, and the **unmodified** suite
green on Linux, macOS and Windows CI. Both invocation modes call the same
`Vault` methods.
