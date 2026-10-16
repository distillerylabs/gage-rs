# M12c — `-c/--clip`, `-q/--qr`

[← M12b](m12b-display-cleanup.md) · [P6 plan](index.md) · [#55](https://github.com/distillerylabs/gage-rs/issues/55) · next: [M12d](m12d-script-mode.md)

> **Recommended model: Sonnet\*.** The behavior is fully specified, and
> every test runs against fakes. What's borderline is the platform layer:
> two new crates, a signal handler, and a timer thread on three CI
> platforms. Switch to Opus at the milestone boundary if that turns
> awkward.

## Goal

The design's answer to principle 6 ("plaintext should be surprising to
produce"): two ways for `show` and `generate` to hand over a secret
*instead of* printing it.

- **One place a decrypted value leaves gage.** Go's `emitSecret`: the
  default prints, `-q` renders a QR code instead, `-c` copies instead.
  Nothing prints the value alongside either one. `-c` and `-q` together
  are a Usage error, and `generate` checks that **before** it inserts
  anything, so a refused run never leaves a new secret in the vault.
- **`-c/--clip`:**
  - copy, then write `gage: copied to clipboard; clears in 45s — Ctrl-C
    to clear now.` to **stderr**; nothing reaches stdout
  - **one-shot:** block until the timeout, clear, exit 0. Ctrl-C during
    the wait clears at once and also exits 0 ("done pasting", not an
    abort)
  - **session:** schedule the clear on a timer and return to the prompt.
    Session exit (`exit`, Ctrl-D, end of input) clears anything pending
  - **clear only if unchanged:** the keeper holds a SHA-256 of what it
    wrote, never the value. At clear time it reads the clipboard and
    clears only on a match. A clipboard it can't read is left alone
  - a second copy supersedes the first's pending clear
  - a failed scheduled clear reaches stderr, because "your secret is
    still on the clipboard" is the one thing here a human must hear
  - a SIGKILL leaves the value on the clipboard. That is an accepted,
    documented limit, as in Go
- **`[shell] clipboard_timeout`:** 45s by default, the same duration
  syntax as `idle_timeout`, and below 1s is a config error. There is no
  `0`/disable value.
- **`-q/--qr`:** a terminal QR code in half-block characters with a
  4-module quiet zone, at error correction level M. With `show --field
  NAME -q` it encodes only that field. An empty value, or one too long to
  encode, is a Usage error.

## Depends on

- **P3 · M5:** `show` (with `--field`) and `generate`. **M6:** the session
  and its exit path (`repl.rs`), and `[shell]` resolution (`shell.rs`).
  The config field `clipboard_timeout` already exists, unused
  (`crates/gage/src/config.rs`).

## Design references

- Design doc, "Notes on `show`": `--field`, `-c`, `-q`, and why `--field`
  scopes the QR.
- Design doc, principles 5 (no background process outlives gage) and 6.

## Go sources

- Go plan `gage-cli-design/m12-polish.md`: the clipboard clear decision
  (A17), the timeout decision (A18), and the clip and QR tests.
- Parity ledger: §3 M12 (`-c`, `-q`); §4 `show`, `generate` (the `-c`/`-q`
  flags M5 left).
- Go sources at the pinned SHA:
  - `cmd/gage/clipboard.go`: `clipboardPort`, `clipboardKeeper`
  - `cmd/gage/secretout.go`: `emitSecret`, `checkSecretOutput`,
    `copySecret`, `waitOrInterrupt`, `waitForClipboard`
  - `cmd/gage/qrcode.go`: `renderQR`
  - `cmd/gage/shell.go`: `clipboard_timeout`, `clipboardTimeout`
  - `cmd/gage/repl.go`: the clear on session exit
- Test inventory, 27 rows: `cmd/gage/clip_test.go` (20),
  `cmd/gage/qr_test.go` (7).
- Inbound, 4 rows from P3: `generate` `TestGenerateClipAndQRAreMutuallyExclusive`,
  `TestGenerateClipCopiesTheValueAndPrintsNone`,
  `TestGenerateQRRendersWithoutPrintingTheValue`; `show`
  `TestShowClipCopiesTheNamedFieldNotTheValue`.

## Decisions to make first

[Q-CLIPBOARD](../open-questions.md#q-clipboard) and
[Q-QR-CRATE](../open-questions.md#q-qr-crate) settle the crates. What's
left to confirm against the crates before writing code:

- **`arboard` on each platform.** Add it to `gage-cli` with
  `default-features = false, features = ["wayland-data-control"]`. Then
  confirm:
  - `cargo tree` and `make audit` pass, and any new license is recorded
    in Q-CLIPBOARD (`deny.toml` too)
  - `no_frontend_deps` still passes (it should: `crates/gage` doesn't
    change)
  - whether `arboard::Clipboard` is `Send`. The session's timer thread
    needs to reach the clipboard. If it isn't `Send` on some platform,
    the timer thread signals the session thread instead of touching the
    clipboard itself; record which.
  - on Linux, contents last only while a `Clipboard` instance lives. The
    keeper holds one for the one-shot wait and for the session's life,
    which matches Go's "the clear happens in the process that wrote it".
- **Ctrl-C.** `ctrlc::set_handler` is process-wide and can be set once,
  which fits: it is installed only on the one-shot `-c` path, just
  before the wait, and the process clears and exits right after. The
  wait is `recv_timeout` on a channel the handler sends to. Tests never
  install it: they replace the wait through the same seam Go's
  `ClipboardWait` is. Rustyline handles Ctrl-C itself in a session.
- **The seams.** In `gage-cli`, all behind traits, as Go's are:
  - `ClipboardPort { write, read }`, with the `arboard` implementation and
    a fake (in-memory, with failure switches)
  - a timer seam that a test fires on demand
  - a wait seam (one-shot)
  - a report sink for a scheduled clear's failure that is `Send`
    (production: the process's stderr; tests: a shared buffer), since the
    timer fires on another thread and `Io.stderr` is borrowed

  They reach commands through `Io`, as `editor` and `now` do.
- **`qrcode`.** Use `default-features = false` (no `image`), `EcLevel::M`,
  and render from `QrCode::to_colors()` with gage's own half-block loop,
  as Go's `renderQR`. The round-trip rows compare the rendered grid with
  the crate's own module grid for the same value, as Go's tests compare
  with `rsc.io/qr`'s. That leaves "does the crate encode correctly" to the
  crate. No decoder dependency.
- **Where the timeout comes from.** A session has `ShellSettings`. A
  one-shot `-c` resolves `[shell]` from global config at that point, as
  Go's `clipboardTimeout` does, so commands that never copy never read it.

## Tasks

Write each task's tests first, at task granularity.

### 1. QR renderer — 5 rows

- [ ] `render_qr(value) -> Result<String, CliError>` in `gage-cli`.
- [ ] Rows (`qr_test`):
  - `TestRenderQRRoundTripsToTheExactValue`
  - `TestRenderQRDistinguishesValues`
  - `TestRenderQRHasAQuietZone`
  - `TestRenderQRIsSquareIshAndRectangular`
  - `TestRenderQRRejectsAnEmptyValue`

### 2. `clipboard_timeout` — 3 rows

- [ ] `ShellSettings.clipboard_timeout`, default 45s, below 1s refused.
- [ ] Rows (`clip_test`):
  - `TestResolveShellSettingsClipboardTimeout`
  - `TestClipboardTimeoutIsConfigurable`
  - `TestClipboardTimeoutBelowTheMinimumIsRejected`

### 3. The clipboard keeper — 7 rows

- [ ] `ClipboardKeeper` over the port and timer seams: `copy`,
      `schedule_clear`, `clear`, hash-guarded and idempotent.
- [ ] Rows (`clip_test`):
  - `TestClipboardKeeperClearIsIdempotentAndSkipsWhenNothingPending`
  - `TestClipboardKeeperLeavesAnUnreadableClipboardAlone`
  - `TestClipboardClearWithNothingPendingIsANoOp`
  - `TestClipboardScheduledClearWithNothingPendingSchedulesNothing`
  - `TestClipboardCopyReportsAWriteFailure`
  - `TestSecondCopySupersedesTheFirstsPendingClear`
  - `TestScheduledClearReportsItsFailure`
- [ ] Beyond Go's rows: the keeper's state holds no copy of the value
      (assert on what the fake recorded and on the keeper's fields).

### 4. One-shot `-c` and `-q` on `show` and `generate` — 8 rows + 4 inbound

- [ ] `-c/--clip` and `-q/--qr` on `show` and `generate`; the
      one-place emitter; the pre-insert exclusivity check on `generate`.
- [ ] Rows (`clip_test`):
  - `TestClipCopiesAndClearsAfterTheTimeout`
  - `TestClipBlocksUntilTheTimeoutThenExitsZero`
  - `TestClipInterruptDuringTheWaitClearsAndExitsZero`
  - `TestClipDoesNotWipeAClipboardSomeoneElseChanged`
  - `TestClipPrintsNoPlaintextAndNoticesOnStderr`
  - `TestClipAndQRAreMutuallyExclusive`
- [ ] Rows (`qr_test`):
  - `TestShowQRPrintsNoPlaintextAlongsideTheCode`
  - `TestShowFieldQREncodesOnlyThatField`
- [ ] Inbound (from P3):
  - `TestShowClipCopiesTheNamedFieldNotTheValue`
  - `TestGenerateClipCopiesTheValueAndPrintsNone`
  - `TestGenerateQRRendersWithoutPrintingTheValue`
  - `TestGenerateClipAndQRAreMutuallyExclusive`: nothing is inserted or
    committed.

### 5. Session `-c` — 4 rows

- [ ] The session holds one keeper for its life; `-c` schedules; the
      REPL's exit path clears, whichever way the session ends.
- [ ] Rows (`clip_test`):
  - `TestSessionClipSchedulesRatherThanBlocking`
  - `TestSessionClipTimerClearsWhenItFires`
  - `TestSessionExitClearsAPendingCopy`
  - `TestSessionScheduledClearFailureReachesStderr`
  - plus the session halves of the copy rows wherever Go's test runs
    both modes.

(Rows: 5 + 3 + 7 + 8 + 4 = 27, plus the 4 inbound in task 4.)

### 6. Platform check, parity, ledger

- [ ] **Manual check on each OS available** (not CI, which has no
      clipboard): `show -c` copies, clears after a short configured
      timeout, Ctrl-C clears early, and a clipboard changed in the
      meantime survives. Record which platforms were checked here.
- [ ] Extend the fixture-parity tests: `show` and `generate` now carry
      every fixture flag, so update M5's
      `m5_commands_have_their_go_fixture_flags_and_descriptions` (it
      exempted `-c`/`-q`) instead of adding a parallel test.
- [ ] Port-plan crates table: `arboard`, `qrcode` and `ctrlc` marked
      decided, first used M12c.
- [ ] Parity ledger: §3 M12 (`-c`, `-q`), §4 `show` and `generate`.
- [ ] Mark M12c `[x]` in the [P6 index](index.md) after CI.

## Definition of done

Every box above is checked. All 27 claimed rows and the 4 inbound ones
are ported or excused in writing, and the platform check is recorded.
`make lint`, `make test` and `make audit` are clean locally, and lint
and test are clean on Linux, macOS and Windows CI.

The milestone is done when:
- no `-c` or `-q` path writes the value to stdout
- a copied secret is cleared by the process that copied it, and a
  clipboard someone else has since changed is never wiped
