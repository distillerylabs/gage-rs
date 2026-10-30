//! `-c/--clip`: the keeper against fakes, then `show`/`generate` through
//! the real CLI, one-shot and in a session. Every test runs over
//! `clipboard::fake`: a CI runner may have no clipboard at all, and a
//! developer's must not be clobbered by `make test`. One-shot runs record
//! the blocking wait instead of sleeping through it, which is how "blocks
//! until the timeout" is asserted without a 45-second test.

use std::time::Duration;

use crate::exitcode::ExitCode;
use gage::storage::config::Shell;
use sha2::{Digest, Sha256};

use crate::clipboard::fake::{FakeClipboard, FakeTimer, SharedSink, keeper};
use crate::clipboard::{ClipboardKeeper, wait_or_interrupt};
use crate::entry_tests::env_with_vault;
use crate::lifecycle_tests::{Env, PASSPHRASE};
use crate::repl_tests::script;
use crate::shell::DEFAULT_CLIPBOARD_TIMEOUT;

fn with_github() -> Env {
    let env = env_with_vault();
    env.insert_ok("hunter2", &["GitHub"]);
    env
}

fn set_clipboard_timeout(env: &Env, raw: &str) {
    env.set_shell(Shell {
        clipboard_timeout: Some(raw.into()),
        ..Shell::default()
    });
}

// --- the keeper ---------------------------------------------------------------

// The
// session's exit path calls clear unconditionally, so with nothing
// outstanding it must not wipe whatever is on the clipboard.
#[test]
fn clipboard_keeper_clear_is_idempotent_and_skips_when_nothing_pending() {
    let cb = FakeClipboard::holding("someone else's copy");
    let k = keeper(&cb, &FakeTimer::default());

    k.clear().unwrap();
    assert_eq!(cb.get(), "someone else's copy");

    k.copy("secret").unwrap();
    k.clear().unwrap();
    cb.set("something new");
    k.clear().unwrap();
    assert_eq!(
        cb.get(),
        "something new",
        "a second clear wiped a value gage did not write"
    );
}

// With no way to
// tell whose value is there, the safe answer is not to destroy a
// stranger's, and the human is told.
#[test]
fn clipboard_keeper_leaves_an_unreadable_clipboard_alone() {
    let cb = FakeClipboard::default();
    let k = keeper(&cb, &FakeTimer::default());
    k.copy("secret").unwrap();
    cb.fail_reads("no clipboard here");

    let err = k.clear().unwrap_err();
    assert_eq!(err.code(), ExitCode::Internal);
    assert_eq!(
        cb.writes(),
        ["secret"],
        "a blind clear must not happen: only the original copy"
    );
}

// "Nothing was ever
// copied" is silent and successful, and writes nothing.
#[test]
fn clipboard_clear_with_nothing_pending_is_a_no_op() {
    let cb = FakeClipboard::holding("something the human copied themselves");
    let k = keeper(&cb, &FakeTimer::default());
    for _ in 0..2 {
        k.clear().unwrap();
    }
    assert_eq!(cb.get(), "something the human copied themselves");
    assert!(cb.writes().is_empty(), "{:?}", cb.writes());
}

// A
// timer armed with nothing copied would later race whatever the human
// copies next.
#[test]
fn clipboard_scheduled_clear_with_nothing_pending_schedules_nothing() {
    let timer = FakeTimer::default();
    let k = keeper(&FakeClipboard::default(), &timer);
    k.schedule_clear(Duration::from_secs(1));
    assert_eq!(timer.scheduled(), 0);
}

// A refused write fails the
// command, says what was being attempted, never carries the secret, and
// leaves nothing pending.
#[test]
fn clipboard_copy_reports_a_write_failure() {
    let cb = FakeClipboard::default();
    cb.fail_writes("no clipboard on this display");
    let k = keeper(&cb, &FakeTimer::default());

    let err = k.copy("hunter2").unwrap_err();
    assert_eq!(err.code(), ExitCode::Internal);
    let msg = err.to_string();
    assert!(msg.contains("copying to the clipboard"), "{msg}");
    assert!(
        !msg.contains("hunter2"),
        "the failure carries the secret: {msg}"
    );

    k.clear().unwrap();
    assert!(cb.writes().is_empty(), "{:?}", cb.writes());
    assert_eq!(k.pending_digest(), None);
}

// The clipboard holds one
// value; the first copy's timer must not later wipe the second.
#[test]
fn second_copy_supersedes_the_firsts_pending_clear() {
    let cb = FakeClipboard::default();
    let first = FakeTimer::default();
    let k = keeper(&cb, &first);

    k.copy("one").unwrap();
    k.schedule_clear(DEFAULT_CLIPBOARD_TIMEOUT);
    k.copy("two").unwrap();
    assert!(
        first.stopped(),
        "the first copy's pending clear was not cancelled"
    );
    assert_eq!(cb.get(), "two");
    k.clear().unwrap();
    assert_eq!(cb.get(), "");
}

// A first timer that had already fired, and reaches the
// keeper only after a second copy landed, is a no-op rather than a wipe of
// the second copy ahead of its own timeout.
#[test]
fn a_stale_timer_firing_after_a_second_copy_does_nothing() {
    let cb = FakeClipboard::default();
    let first = FakeTimer::default();
    let k = keeper(&cb, &first);

    k.copy("one").unwrap();
    k.schedule_clear(DEFAULT_CLIPBOARD_TIMEOUT);
    k.copy("two").unwrap();
    first.fire();
    assert_eq!(cb.get(), "two", "the first copy's timer wiped the second");
    assert_eq!(k.pending_digest(), Some(sha256("two")));
}

// A timer firing has no caller to
// return to, so its failure says so itself: silence would leave the human
// believing the secret was wiped when it is still there.
#[test]
fn scheduled_clear_reports_its_failure() {
    let (cb, timer, sink) = (
        FakeClipboard::default(),
        FakeTimer::default(),
        SharedSink::default(),
    );
    let k = ClipboardKeeper::new(
        Box::new(cb.clone()),
        Box::new(timer.clone()),
        Some(Box::new(sink.clone())),
    );
    k.copy("hunter2").unwrap();
    k.schedule_clear(DEFAULT_CLIPBOARD_TIMEOUT);
    // The clipboard goes away between the copy and the timeout.
    cb.fail_writes("clipboard unavailable");

    timer.fire();

    let reported = sink.text();
    assert_eq!(reported.lines().count(), 1, "{reported}");
    assert!(reported.contains("clearing the clipboard"), "{reported}");
    assert!(reported.starts_with("gage: ") && !reported.contains("gage: gage: "));
    assert!(!reported.contains("hunter2"), "{reported}");
    assert_eq!(cb.get(), "hunter2", "the value really is still there");
}

// The session is still running
// when the timeout arrives.
#[test]
fn session_clip_timer_clears_when_it_fires() {
    let (cb, timer) = (FakeClipboard::default(), FakeTimer::default());
    let k = keeper(&cb, &timer);
    k.copy("hunter2").unwrap();
    k.schedule_clear(DEFAULT_CLIPBOARD_TIMEOUT);
    assert_eq!(cb.get(), "hunter2");
    timer.fire();
    assert_eq!(cb.get(), "");
}

fn sha256(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

// The keeper holds a digest of what it wrote, never the
// value. The value reached the port once, and what the keeper remembers is
// its SHA-256.
#[test]
fn the_keeper_holds_a_digest_never_the_value() {
    let cb = FakeClipboard::default();
    let k = keeper(&cb, &FakeTimer::default());
    k.copy("hunter2").unwrap();
    assert_eq!(cb.writes(), ["hunter2"]);
    assert_eq!(k.pending_digest(), Some(sha256("hunter2")));
}

// --- clipboard_timeout -----------------------------------------------------------

// [shell].clipboard_timeout
// overrides the default and reaches the wait.
#[test]
fn clipboard_timeout_is_configurable() {
    let env = with_github();
    set_clipboard_timeout(&env, "5s");
    env.cmd_ok(&["show", "GitHub", "-c"]);
    assert_eq!(env.clip.waits(), [Duration::from_secs(5)]);
}

// Fails before anything
// reaches the clipboard, naming the setting at fault.
#[test]
fn clipboard_timeout_below_the_minimum_is_rejected() {
    let env = with_github();
    set_clipboard_timeout(&env, "200ms");
    let r = env.cmd(&["show", "GitHub", "-c"]);
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert!(
        env.clip.cb.writes().is_empty(),
        "{:?}",
        env.clip.cb.writes()
    );
    assert!(r.stderr.contains("clipboard_timeout"), "{}", r.stderr);
}

// --- one-shot -c --------------------------------------------------------------------

// The value reaches the
// clipboard, and it does not stay there.
#[test]
fn clip_copies_and_clears_after_the_timeout() {
    let env = with_github();
    env.cmd_ok(&["show", "GitHub", "-c"]);
    assert_eq!(env.clip.cb.writes(), ["hunter2", ""]);
    assert_eq!(env.clip.cb.get(), "");
}

// A one-shot waits in the
// foreground rather than leaving a clearer behind (principle 5).
#[test]
fn clip_blocks_until_the_timeout_then_exits_zero() {
    let env = with_github();
    let r = env.cmd(&["show", "GitHub", "-c"]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert_eq!(env.clip.waits(), [DEFAULT_CLIPBOARD_TIMEOUT]);
    // Nothing was scheduled: the one-shot clears in the foreground.
    assert_eq!(env.clip.timer.scheduled(), 0);
}

// Ctrl-C is "I've
// finished pasting": the wait returns as it would at the timeout, so the
// clear runs and the command succeeds either way.
#[test]
fn clip_interrupt_during_the_wait_clears_and_exits_zero() {
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(()).unwrap();
    let started = std::time::Instant::now();
    // A timeout that would outlast the test: only the interrupt ends it.
    wait_or_interrupt(&rx, Duration::from_secs(3600));
    assert!(started.elapsed() < Duration::from_secs(60));

    // The CLI half: whatever ends the wait, the clear runs and exit is 0.
    let env = with_github();
    let r = env.cmd(&["show", "GitHub", "-c"]);
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert_eq!(env.clip.cb.get(), "");
}

// Someone copies
// something else while gage waits out the timeout, the window the
// conditional clear exists for.
#[test]
fn clip_does_not_wipe_a_clipboard_someone_else_changed() {
    let env = with_github();
    env.clip.during_wait(|cb| cb.set("someone else's copy"));
    env.cmd_ok(&["show", "GitHub", "-c"]);
    assert_eq!(env.clip.cb.get(), "someone else's copy");
}

// -c copies instead of
// printing, and the notice goes to stderr so a redirected stdout carries
// nothing.
#[test]
fn clip_prints_no_plaintext_and_notices_on_stderr() {
    let env = with_github();
    let r = env.cmd_ok(&["show", "GitHub", "-c"]);
    assert_eq!(r.stdout, "", "-c must print nothing to stdout");
    assert!(
        r.stderr
            .contains("gage: copied to clipboard; clears in 45s — Ctrl-C to clear now."),
        "{}",
        r.stderr
    );
    assert!(!r.stderr.contains("hunter2"), "{}", r.stderr);
}

// Two answers to one question, so
// both is a usage error, and the clipboard is untouched.
#[test]
fn clip_and_qr_are_mutually_exclusive() {
    let env = with_github();
    let r = env.cmd(&["show", "GitHub", "-c", "-q"]);
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("mutually exclusive"), "{}", r.stderr);
    assert!(
        env.clip.cb.writes().is_empty(),
        "{:?}",
        env.clip.cb.writes()
    );
    assert!(!r.stdout.contains("hunter2"), "{}", r.stdout);
}

// --field chooses
// which value; -c chooses where it goes.
#[test]
fn show_clip_copies_the_named_field_not_the_value() {
    let env = env_with_vault();
    env.insert_ok(
        "the-password",
        &["Proton", "--field", "username=me@example.com"],
    );
    env.cmd_ok(&["show", "Proton", "--field", "username", "-c"]);
    assert_eq!(
        env.clip.cb.writes().first().map(String::as_str),
        Some("me@example.com")
    );
}

// Plain
// generate never prints the value; -c is how it gets out without landing
// in the scrollback. The confirmation still prints, before the wait.
#[test]
fn generate_clip_copies_the_value_and_prints_none() {
    let env = env_with_vault();
    let r = env.cmd_ok(&["generate", "GitHub", "-c"]);
    let stored = env.only_entry("personal").1.value;
    assert_eq!(env.clip.cb.writes().first(), Some(&stored));
    assert!(!r.stdout.contains(&stored), "{}", r.stdout);
    assert!(!r.stderr.contains(&stored), "{}", r.stderr);
    assert!(r.stdout.contains("generated"), "{}", r.stdout);
    assert_eq!(env.clip.waits(), [DEFAULT_CLIPBOARD_TIMEOUT]);
    assert_eq!(env.clip.cb.get(), "");
}

// Refused before
// anything is generated or committed, so the vault is untouched.
#[test]
fn generate_clip_and_qr_are_mutually_exclusive() {
    let env = env_with_vault();
    let vault = env.entry("personal").path;
    let commits = gage::sync::gitrepo::commit_count(&vault).unwrap();
    // Refused before the unlock, so the prompter is never asked.
    let r = env.run_with(
        &["generate", "Both", "-c", "-q"],
        &gage::prompter::FakePrompter::new(),
    );
    assert_eq!(r.code, ExitCode::Usage, "{}", r.stderr);
    assert!(env.entries("personal").is_empty());
    assert_eq!(
        gage::sync::gitrepo::commit_count(&vault).unwrap(),
        commits,
        "a commit was made"
    );
    assert!(env.clip.cb.writes().is_empty());
}

// --- session -c ----------------------------------------------------------------------

fn session_show_clip(env: &Env) -> crate::lifecycle_tests::CliResult {
    let r = env.session(&script(&[
        "use personal",
        PASSPHRASE,
        "show GitHub -c",
        "status",
        "exit",
    ]));
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    r
}

// A session hands the
// prompt straight back and clears on a timer.
#[test]
fn session_clip_schedules_rather_than_blocking() {
    let env = with_github();
    let r = session_show_clip(&env);
    assert!(
        env.clip.waits().is_empty(),
        "a session's show -c took the one-shot wait"
    );
    assert_eq!(
        env.clip.cb.writes().first().map(String::as_str),
        Some("hunter2")
    );
    assert_eq!(env.clip.timer.after(), Some(DEFAULT_CLIPBOARD_TIMEOUT));
    assert!(
        r.stderr
            .contains("gage: copied to clipboard; clears in 45s."),
        "{}",
        r.stderr
    );
    // The prompt came back: the next line ran.
    assert!(r.stdout.contains("unlocked"), "{}", r.stdout);
    assert!(!r.stdout.contains("hunter2"), "{}", r.stdout);
    assert!(!r.stderr.contains("hunter2"), "{}", r.stderr);
}

// `exit` a second after `show -c`
// does not leave the secret for the remaining 44: the timer never fired,
// and the value is gone anyway.
#[test]
fn session_exit_clears_a_pending_copy() {
    let env = with_github();
    session_show_clip(&env);
    assert_eq!(env.clip.cb.get(), "");
    assert!(
        env.clip.timer.stopped(),
        "the pending clear's timer was left running"
    );
}

// The other ways a session ends clear too: end of input with no `exit`.
#[test]
fn session_end_of_input_clears_a_pending_copy() {
    let env = with_github();
    let r = env.session(&script(&["use personal", PASSPHRASE, "show GitHub -c"]));
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert_eq!(env.clip.cb.get(), "");
}

// The wiring half: a
// session's keeper reports a failed scheduled clear to the human, not to
// nowhere. The timer fires mid-session, from the session's own clock read
// between two lines, which is outside any command, as a real timer's
// thread is. (`main` gives the real keeper the process's stderr.)
/// The real clock, firing `timer` on every read: a session reads its
/// clock between lines, so that is where the fake timer goes off.
struct FiresTimer<'t>(&'t FakeTimer);

impl gage::Clock for FiresTimer<'_> {
    fn now(&self) -> std::time::Instant {
        self.0.fire_if_scheduled();
        std::time::Instant::now()
    }
}

#[test]
fn session_scheduled_clear_failure_reaches_stderr() {
    let env = with_github();
    env.clip.cb.fail_clears("clipboard unavailable");
    let clock = FiresTimer(&env.clip.timer);
    let r = env.session_with_clock(
        &script(&[
            "use personal",
            PASSPHRASE,
            "show GitHub -c",
            "status",
            "exit",
        ]),
        &clock,
    );
    assert_eq!(r.code, ExitCode::Success, "{}", r.stderr);
    assert_eq!(env.clip.timer.scheduled(), 1);
    let reported = env.clip.reports.text();
    assert!(
        reported.contains("gage: clearing the clipboard: clipboard unavailable"),
        "a failed scheduled clear said nothing: {reported:?}"
    );
    assert!(!reported.contains("hunter2"), "{reported}");
    assert_eq!(
        env.clip.cb.get(),
        "hunter2",
        "the value really is still there"
    );
}

// A clear that fails at the session's end is reported, since the value may
// still be on the clipboard, and the session still ends as it would have.
#[test]
fn a_failed_clear_at_session_exit_reaches_stderr() {
    let env = with_github();
    env.clip.cb.fail_clears("clipboard unavailable");
    let r = session_show_clip(&env);
    assert!(
        r.stderr
            .contains("gage: clearing the clipboard: clipboard unavailable"),
        "{}",
        r.stderr
    );
}

// The session half of the copy rows: a failed copy in a session is
// reported and the session goes on.
#[test]
fn a_failed_copy_in_a_session_is_reported_and_the_session_continues() {
    let env = with_github();
    env.clip.cb.fail_writes("no clipboard on this display");
    let r = session_show_clip(&env);
    assert!(
        r.stderr.contains("copying to the clipboard"),
        "{}",
        r.stderr
    );
    assert!(r.stdout.contains("unlocked"), "{}", r.stdout);
    assert_eq!(env.clip.timer.scheduled(), 0);
}
