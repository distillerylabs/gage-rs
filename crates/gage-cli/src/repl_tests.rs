//! Session mode through the real CLI, in-process: bare `gage` with a
//! terminal on stdin, its lines and every prompt's answer read from one
//! in-memory script, exactly as a human types them in order (the line
//! after `use personal` is the passphrase). Ports Go's `repl_test.go`,
//! `session_test.go`, `sessionhelp_test.go` and the session row of
//! `usageerror_test.go`; each test names its Go row.
//!
//! Prompts go to stderr in gage-rs, where Go's REPL moved them to stdout,
//! so a few assertions read both streams where Go read stdout alone.

use std::cell::{Cell, RefCell};
use std::io::{Cursor, Read, Write};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use gage::config::Shell;
use gage::exitcode::Code;
use gage::locker::{Locker, MemLocker};
use gage::prompter::FakePrompter;
use gage::session::{Session, SessionConfig};

use crate::history::History;
use crate::lifecycle_tests::{CliResult, Env, PASSPHRASE};
use crate::lineread::{LineReader, PlainLineReader, ReadOutcome, map_readline};
use crate::prompter::{SecretInput, TerminalPrompter};
use crate::registry::{Availability, registry};
use crate::repl::{Repl, exec_line, split_line};
use crate::shell::{self, LOCKED_GLYPH, ShellSettings, UNLOCKED_GLYPH};
use crate::{BuildInfo, Io};

/// One stream shared by the REPL's line reader and the prompter, so a
/// script line goes to whichever asks next.
#[derive(Clone)]
pub(crate) struct SharedInput(pub(crate) Rc<RefCell<Cursor<Vec<u8>>>>);

impl Read for SharedInput {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().read(buf)
    }
}

/// An output buffer written by both the CLI and the prompter.
#[derive(Clone, Default)]
pub(crate) struct SharedOutput(Rc<RefCell<Vec<u8>>>);

impl SharedOutput {
    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.0.borrow().clone()).unwrap()
    }
}

impl Write for SharedOutput {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `lines` as stdin content, so tests read as transcripts.
pub(crate) fn script(lines: &[&str]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

/// How many times a transcript asked for a passphrase.
pub(crate) fn passphrase_prompts(r: &CliResult) -> usize {
    format!("{}{}", r.stdout, r.stderr)
        .matches("Enter passphrase for vault")
        .count()
}

/// A clock that advances `step` each time it is read: how a session
/// driven end to end gets simulated idle time with no hook between its
/// lines (Go's `stepClock`).
fn step_clock(step: Duration) -> impl Fn() -> Instant {
    let t = Cell::new(Instant::now());
    move || {
        t.set(t.get() + step);
        t.get()
    }
}

impl Env {
    /// Bare `gage` on a "terminal", fed `script` through the real
    /// terminal prompter (Go's `runSessionScript`).
    pub(crate) fn session(&self, script: &str) -> CliResult {
        self.session_with_clock(script, &Instant::now)
    }

    pub(crate) fn session_with_clock(&self, script: &str, now: &dyn Fn() -> Instant) -> CliResult {
        let input = SharedInput(Rc::new(RefCell::new(Cursor::new(
            script.as_bytes().to_vec(),
        ))));
        let stderr = SharedOutput::default();
        // Interactive, as `main` makes it on a terminal: a session on one
        // can ask which version of a conflicted entry to keep.
        let prompter = TerminalPrompter::new(input.clone(), stderr.clone(), SecretInput::Lines)
            .interactive(true);
        let mut stdout = Vec::new();
        let dirs = self.dirs.clone();
        let hostname = self.hostname.clone();
        let code = {
            let mut stdin = input.clone();
            let mut err = stderr.clone();
            let mut io = Io {
                stdin: &mut stdin,
                stdout: &mut stdout,
                stderr: &mut err,
                stdin_is_terminal: true,
                stderr_is_terminal: true,
                line_editor: false,
                build: BuildInfo::stamped(),
                dirs: &move || Ok(dirs.clone()),
                hostname: &move || Ok(hostname.clone()),
                editor: self.editor.lock().unwrap().clone(),
                passphrase_env: None,
                now,
                clipboard: &self.clip.keeper(),
                clipboard_wait: &|d| self.clip.wait(d),
                time_zone: self.time_zone.lock().unwrap().clone(),
            };
            crate::run(&[], &mut io, &prompter)
        };
        CliResult {
            code,
            stdout: String::from_utf8(stdout).unwrap(),
            stderr: stderr.text(),
        }
    }

    /// A session that must end cleanly.
    pub(crate) fn session_ok(&self, lines: &[&str]) -> CliResult {
        let r = self.session(&script(lines));
        assert_eq!(r.code, Code::Success, "session failed:\n{}", r.stderr);
        r
    }

    /// `name` gets an entry `title` holding `value`, through one-shot
    /// `insert`.
    pub(crate) fn add(&self, name: &str, title: &str, value: &str) {
        self.insert_ok(value, &["--use", name, title]);
    }

    fn history_text(&self) -> String {
        std::fs::read_to_string(self.dirs.state.join("history")).unwrap()
    }

    pub(crate) fn set_shell(&self, shell: Shell) {
        let mut g = self.global();
        g.shell = shell;
        self.save_global(&g);
    }
}

/// "The session survived to run its next command": the line after the
/// one under test was a `status` in a session holding nothing.
const NOTHING_UNLOCKED_YET: &str = "no vaults have been unlocked in this session yet";

// --- entering a session -----------------------------------------------------

// Go: TestBareGageOnTTYEntersSessionMode. Stdin is empty, so the loop
// starts, prompts once, reads end of input and exits cleanly.
#[test]
fn bare_gage_on_tty_enters_session_mode() {
    let env = Env::new();
    let r = env.session("");
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(!r.stdout.contains("Usage:"), "printed help:\n{}", r.stdout);
    assert!(r.stdout.contains("gage> "), "no prompt:\n{}", r.stdout);
}

// Go: TestSessionOnlyCommandsFailCleanlyOneShot
#[test]
fn session_only_commands_fail_cleanly_one_shot() {
    let env = Env::new();
    for name in ["use", "lock", "status", "exit", "whoami", "quit"] {
        let r = env.run_with(&[name], &FakePrompter::new());
        assert_eq!(r.code, Code::Usage, "{name}: {}", r.stderr);
        assert!(r.stderr.contains("session"), "{name}: {}", r.stderr);
    }
}

// Go: TestNoBareUnenumeratedExitCode. Every scenario maps to a defined
// code, and never a bare 1 (Conflict), which none of these produce.
#[test]
fn no_bare_unenumerated_exit_code() {
    let env = Env::new();
    let defined: Vec<i32> = gage::exitcode::all()
        .into_iter()
        .map(Code::as_i32)
        .collect();
    for args in [&["bogus"][..], &["help", "bogus"], &["use"]] {
        let r = env.run_with(args, &FakePrompter::new());
        assert!(defined.contains(&r.code.as_i32()), "{args:?}");
        assert_ne!(r.code, Code::Conflict, "{args:?}");
    }
}

// --- wiring -----------------------------------------------------------------

/// A session over `env`'s registered vaults, unlocking through `prompter`
/// and page-locking through `locker`.
fn direct_session<'a>(
    env: &'a Env,
    prompter: &'a FakePrompter,
    locker: Arc<dyn Locker>,
) -> Session<'a> {
    let dirs = env.dirs.clone();
    Session::new(SessionConfig {
        open: Box::new(move |name| {
            crate::commands::open_registered(&dirs, name).map(|v| v.with_locker(locker.clone()))
        }),
        prompter,
        idle_timeout: None,
        current: None,
        now: None,
    })
}

/// Runs `f` against an `Io` over `stdin`, returning what it wrote.
fn with_io<T>(env: &Env, stdin: &str, f: impl FnOnce(&mut Io) -> T) -> (T, String) {
    let mut out = SharedOutput::default();
    let mut err = out.clone();
    let mut stdin = stdin.as_bytes();
    let dirs = env.dirs.clone();
    let got = f(&mut Io {
        stdin: &mut stdin,
        stdout: &mut out,
        stderr: &mut err,
        stdin_is_terminal: true,
        stderr_is_terminal: true,
        line_editor: false,
        build: BuildInfo::stamped(),
        dirs: &move || Ok(dirs.clone()),
        hostname: &|| panic!("a session test read the hostname"),
        editor: None,
        passphrase_env: None,
        now: &Instant::now,
        clipboard: &crate::clipboard::fake::unused(),
        clipboard_wait: &|_| {},
        time_zone: jiff::tz::TimeZone::UTC,
    });
    let text = out.text();
    (got, text)
}

/// Counts page locks and unlocks: balanced means no key is held.
#[derive(Default)]
struct Tally {
    locks: AtomicUsize,
    unlocks: AtomicUsize,
}

impl Tally {
    fn held(&self) -> usize {
        self.locks.load(Ordering::SeqCst) - self.unlocks.load(Ordering::SeqCst)
    }
}

impl Locker for Tally {
    fn lock(&self, buf: &[u8]) -> std::io::Result<()> {
        self.locks.fetch_add(1, Ordering::SeqCst);
        MemLocker.lock(buf)
    }
    fn unlock(&self, buf: &[u8]) -> std::io::Result<()> {
        self.unlocks.fetch_add(1, Ordering::SeqCst);
        MemLocker.unlock(buf)
    }
}

// Go: TestREPLIsAThinWiringLayer. A typed line becomes the matching
// Session call and its result is rendered; Session's own behavior is
// tested in the library.
#[test]
fn repl_is_a_thin_wiring_layer() {
    let env = Env::new();
    env.init("personal", &[]);
    env.init("work", &[]);
    let p = FakePrompter::new().with_passphrase(PASSPHRASE);
    let mut s = direct_session(&env, &p, Arc::new(MemLocker));
    let reg = registry();

    let (quit, _) = with_io(&env, "", |io| exec_line(&reg, &mut s, io, "use work"));
    assert!(!quit.unwrap(), "`use` ended the session");
    assert_eq!(s.current(), Some("work"));

    let (quit, out) = with_io(&env, "", |io| exec_line(&reg, &mut s, io, "lock work"));
    assert!(!quit.unwrap(), "`lock` ended the session");
    let st = s.status();
    assert_eq!(st.len(), 1);
    assert!(!st[0].unlocked);
    assert!(out.contains("work locked."), "{out}");

    for exit in ["exit", "quit"] {
        let (quit, _) = with_io(&env, "", |io| exec_line(&reg, &mut s, io, exit));
        assert!(quit.unwrap(), "{exit} didn't end the session");
    }
}

// Go: TestSessionSkipsBlankLines
#[test]
fn session_skips_blank_lines() {
    let env = Env::new();
    let r = env.session_ok(&["", "   ", "\t", "exit"]);
    assert!(!r.stderr.contains("unknown command"), "{}", r.stderr);
}

// Go: TestUnknownSessionCommandReportsAndContinues. A typo doesn't end a
// session, and points at the surface listing what's available.
#[test]
fn unknown_session_command_reports_and_continues() {
    let env = Env::new();
    let r = env.session_ok(&["nope", "status", "exit"]);
    assert!(
        r.stderr.contains("unknown command \"nope\""),
        "{}",
        r.stderr
    );
    assert!(r.stderr.contains("help"), "{}", r.stderr);
    assert!(r.stdout.contains(NOTHING_UNLOCKED_YET), "{}", r.stdout);
}

// Go: TestOneShotOnlyCommandsReportPlainlyInSession. init creates a vault
// rather than operating on one, so it's one-shot only, and says so.
#[test]
fn one_shot_only_commands_report_plainly_in_session() {
    let env = Env::new();
    env.init("personal", &[]);
    let r = env.session_ok(&["init another", "status", "exit"]);
    assert!(r.stderr.contains("one-shot command"), "{}", r.stderr);
    assert!(r.stderr.contains("`gage init`"), "{}", r.stderr);
    assert!(!r.stderr.contains("unknown command"), "{}", r.stderr);
    assert!(r.stdout.contains(NOTHING_UNLOCKED_YET), "{}", r.stdout);
    assert!(!env.global().vaults.contains_key("another"));
}

// Go: TestStdinReadingInsertModesAreRefusedInASession. Both read stdin to
// EOF, and in a session that's the session's own input.
#[test]
fn stdin_reading_insert_modes_are_refused_in_a_session() {
    for flag in ["-m", "--value-stdin"] {
        let env = Env::new();
        env.init("personal", &[]);
        let r = env.session_ok(&[
            "use personal",
            PASSPHRASE,
            &format!("insert {flag} Notes"),
            "status",
            "exit",
        ]);
        assert!(
            r.stderr.contains("session's own input"),
            "{flag}: {}",
            r.stderr
        );
        assert!(r.stdout.contains("unlocked"), "{flag}: {}", r.stdout);
        assert!(env.entries("personal").is_empty(), "{flag} inserted anyway");
    }
}

// Go: TestSessionCommandArgumentCountsAreChecked (interactive half; the
// --stdin half is P6 · M12's). A usage error is reported and the session
// goes on.
#[test]
fn session_command_argument_counts_are_checked() {
    for (line, want) in [
        ("use", "usage: use <vault>"),
        ("use personal work", "usage: use <vault>"),
        ("lock personal work", "usage: lock [vault]"),
        ("status extra", "usage: status"),
    ] {
        let env = Env::new();
        env.init("personal", &[]);
        let r = env.session_ok(&[line, "status", "exit"]);
        assert!(r.stderr.contains(want), "{line}: {}", r.stderr);
        assert!(
            r.stdout.contains(NOTHING_UNLOCKED_YET),
            "{line}: {}",
            r.stdout
        );
    }
}

// Go: usageerror TestSessionWrongArgCountShowsCommandUsage, through the
// interactive session (Go drives it through --stdin, which is P6's). A
// clap rejection carries the command's own usage, not only the
// complaint, and the session goes on.
#[test]
fn session_wrong_arg_count_shows_command_usage() {
    let env = Env::new();
    env.init("personal", &[]);
    let r = env.session_ok(&["edit", "status", "exit"]);
    assert!(r.stderr.contains("<query>"), "{}", r.stderr);
    assert!(r.stderr.contains("Usage:\n  gage edit"), "{}", r.stderr);
    assert!(r.stdout.contains(NOTHING_UNLOCKED_YET), "{}", r.stdout);
}

// Go: TestNoCommandDoublesGagesOwnPrefix, the session's failures (the
// table's process rule: each milestone adds its own).
#[test]
fn no_session_failure_doubles_gages_own_prefix() {
    let env = Env::new();
    env.init("personal", &[]);
    env.add("personal", "AWS root account", "a");
    env.add("personal", "AWS IAM backup admin", "b");
    let r = env.session_ok(&[
        "nope",
        "init another",
        "use",
        "use not-registered",
        r#"show "unbalanced"#,
        "help nope",
        "lock not-held",
        "status extra",
        "edit",
        "use personal",
        PASSPHRASE,
        "insert -m Notes",
        "show nothing-matches",
        "show AWS",
    ]);
    // One report per failing line (the last follows the picker's prompt,
    // which end of input left unanswered).
    assert_eq!(r.stderr.matches("gage: ").count(), 12, "{}", r.stderr);
    let doubled = format!("{0}{0}", crate::error::ERROR_PREFIX);
    for line in r.stderr.lines() {
        assert!(!line.contains(&doubled), "{line}");
    }
}

// Go: TestSplitLineHandlesQuotesAndBackslashes. Titles have spaces, and
// Windows paths have backslashes, which survive: quotes are the only
// grouping.
#[test]
fn split_line_handles_quotes_and_backslashes() {
    let v = |args: &[&str]| args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    for (input, want) in [
        ("show ProtonMail", v(&["show", "ProtonMail"])),
        ("  show   ProtonMail  ", v(&["show", "ProtonMail"])),
        (
            r#"show "AWS root account""#,
            v(&["show", "AWS root account"]),
        ),
        ("show 'AWS root account'", v(&["show", "AWS root account"])),
        (
            r#"rename "old name" "new name""#,
            v(&["rename", "old name", "new name"]),
        ),
        (r"insert C:\keys\prod", v(&["insert", r"C:\keys\prod"])),
        (
            "show --use work Entry",
            v(&["show", "--use", "work", "Entry"]),
        ),
        (r#"show "it's""#, v(&["show", "it's"])),
        (r#"show a"b c"d"#, v(&["show", "ab cd"])),
        (r#"show """#, v(&["show", ""])),
        ("", v(&[])),
    ] {
        assert_eq!(split_line(input).unwrap(), want, "{input:?}");
    }
}

// Go: TestSplitLineRejectsAnUnbalancedQuote
#[test]
fn split_line_rejects_an_unbalanced_quote() {
    for input in [r#"show "unterminated"#, "show 'x"] {
        let err = split_line(input).unwrap_err();
        assert_eq!(err.code(), Code::Usage, "{input}");
    }
}

// --- unlock and lock ---------------------------------------------------------

// Go: TestSessionUnlocksOnceForManyCommands. The milestone's headline: one
// passphrase, several commands.
#[test]
fn session_unlocks_once_for_many_commands() {
    let env = Env::new();
    env.init("personal", &[]);
    env.add("personal", "ProtonMail", "correcthorsebatterystaple");
    env.add("personal", "Chase", "hunter2");

    let r = env.session_ok(&[
        "use personal",
        PASSPHRASE,
        "show ProtonMail",
        "show Chase",
        "exit",
    ]);
    assert!(
        r.stdout.contains("correcthorsebatterystaple"),
        "{}",
        r.stdout
    );
    assert!(r.stdout.contains("hunter2"), "{}", r.stdout);
    assert_eq!(passphrase_prompts(&r), 1, "{}", r.stderr);
}

// Go: TestSessionLockThenUseRePrompts
#[test]
fn session_lock_then_use_re_prompts() {
    let env = Env::new();
    env.init("personal", &[]);
    env.add("personal", "ProtonMail", "correcthorsebatterystaple");

    let r = env.session_ok(&[
        "use personal",
        PASSPHRASE,
        "lock personal",
        "use personal",
        PASSPHRASE,
        "show ProtonMail",
        "exit",
    ]);
    assert!(r.stdout.contains("personal locked."), "{}", r.stdout);
    assert_eq!(passphrase_prompts(&r), 2, "{}", r.stderr);
    assert!(
        r.stdout.contains("correcthorsebatterystaple"),
        "{}",
        r.stdout
    );
}

// Go: TestSessionBareLockLocksEverythingItHolds. It names each vault it
// locked: what was holding a key, not its arguments.
#[test]
fn session_bare_lock_locks_everything_it_holds() {
    let env = Env::new();
    env.init("personal", &[]);
    env.init("work", &[]);
    let r = env.session_ok(&[
        "use personal",
        PASSPHRASE,
        "use work",
        PASSPHRASE,
        "lock",
        "status",
        "lock",
        "exit",
    ]);
    for name in ["personal", "work"] {
        assert!(
            r.stdout.contains(&format!("{name} locked.")),
            "{}",
            r.stdout
        );
    }
    assert_eq!(
        r.stdout.matches("locked. run `use").count(),
        2,
        "the second bare lock had nothing left:\n{}",
        r.stdout
    );
    assert!(
        r.stdout.contains("no vaults are unlocked in this session"),
        "{}",
        r.stdout
    );
}

// Go: TestSessionLockWithNothingUnlockedSaysSo
#[test]
fn session_lock_with_nothing_unlocked_says_so() {
    let env = Env::new();
    env.init("personal", &[]);
    let r = env.session_ok(&["lock", "exit"]);
    assert!(
        r.stdout.contains("no vaults are unlocked in this session"),
        "{}",
        r.stdout
    );
    assert!(!r.stdout.contains("locked. run `use"), "{}", r.stdout);
}

// Go: TestSessionStatusListsTouchedVaults
#[test]
fn session_status_lists_touched_vaults() {
    let env = Env::new();
    env.init("personal", &[]);
    env.init("work", &[]);
    let r = env.session_ok(&[
        "use personal",
        PASSPHRASE,
        "use work",
        PASSPHRASE,
        "lock personal",
        "status",
        "exit",
    ]);
    // Output follows the prompt on its line, as a terminal shows it.
    let line = |name: &str| {
        r.stdout
            .lines()
            .map(|l| l.rsplit("gage> ").next().unwrap())
            .find(|l| l.contains(name) && l.contains("lock") && !l.contains("run `use"))
            .unwrap_or_else(|| panic!("no status line for {name}:\n{}", r.stdout))
            .to_string()
    };
    let (personal, work) = (line("personal"), line("work"));
    assert!(
        personal.contains("locked") && !personal.contains("unlocked"),
        "{personal}"
    );
    assert!(work.contains("unlocked"), "{work}");
    assert!(work.starts_with("* "), "work isn't marked current: {work}");
    assert!(work.contains("last used"), "{work}");
}

// Go: TestSessionAdHocUseFlagDoesNotChangeCurrentVault
#[test]
fn session_ad_hoc_use_flag_does_not_change_current_vault() {
    let env = Env::new();
    env.init("personal", &[]);
    env.init("work", &[]);
    env.add("personal", "Personal Entry", "personal-value");
    env.add("work", "Work Entry", "work-value");

    let r = env.session_ok(&[
        "use personal",
        PASSPHRASE,
        r#"show --use work "Work Entry""#,
        PASSPHRASE,
        r#"show "Personal Entry""#,
        "exit",
    ]);
    assert!(r.stdout.contains("work-value"), "{}", r.stdout);
    assert!(r.stdout.contains("personal-value"), "{}", r.stdout);
    assert_eq!(passphrase_prompts(&r), 2, "one per vault:\n{}", r.stderr);
}

// --- the picker ---------------------------------------------------------------

// Go: TestSessionAmbiguousQueryPrompts. The numbered picker, answered by a
// typed number, where one-shot fails with the same list.
#[test]
fn session_ambiguous_query_prompts() {
    let env = Env::new();
    env.init("personal", &[]);
    env.add("personal", "AWS root account", "root-secret");
    env.add("personal", "AWS IAM backup admin", "iam-secret");

    // Sorted by title, 2 is the root account.
    let r = env.session_ok(&["use personal", PASSPHRASE, "show AWS", "2", "exit"]);
    assert!(r.stderr.contains("Multiple entries match"), "{}", r.stderr);
    assert!(r.stderr.contains("Which one? [1-2]:"), "{}", r.stderr);
    assert!(r.stdout.contains("root-secret"), "{}", r.stdout);
    assert!(!r.stdout.contains("iam-secret"), "{}", r.stdout);
}

// Go: TestSessionRmResolvesAmbiguityThroughThePicker. rm resolves inside
// the library, so it needs its own proof that a session asks.
#[test]
fn session_rm_resolves_ambiguity_through_the_picker() {
    let env = Env::new();
    env.init("personal", &[]);
    env.add("personal", "AWS root account", "root-secret");
    env.add("personal", "AWS IAM backup admin", "iam-secret");

    let r = env.session_ok(&["use personal", PASSPHRASE, "rm AWS", "1", "exit"]);
    assert!(r.stderr.contains("Which one? [1-2]:"), "{}", r.stderr);
    let left = env.entries("personal");
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].1.title, "AWS root account", "1 was the IAM entry");
}

// rename, the other library-resolved write, asks too.
#[test]
fn session_rename_resolves_ambiguity_through_the_picker() {
    let env = Env::new();
    env.init("personal", &[]);
    env.add("personal", "AWS root account", "root-secret");
    env.add("personal", "AWS IAM backup admin", "iam-secret");

    env.session_ok(&[
        "use personal",
        PASSPHRASE,
        "rename AWS Renamed",
        "2",
        "exit",
    ]);
    let mut titles: Vec<String> = env
        .entries("personal")
        .into_iter()
        .map(|(_, e)| e.title)
        .collect();
    titles.sort();
    assert_eq!(titles, ["AWS IAM backup admin", "Renamed"]);
}

// Declining the picker (end of input) lists the candidates, as one-shot
// does, and the session still ends cleanly.
#[test]
fn declining_the_picker_lists_the_candidates() {
    let env = Env::new();
    env.init("personal", &[]);
    env.add("personal", "AWS root account", "root-secret");
    env.add("personal", "AWS IAM backup admin", "iam-secret");

    let r = env.session(&script(&["use personal", PASSPHRASE, "show AWS"]));
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stderr.contains("matches 2 entries"), "{}", r.stderr);
    assert!(!r.stdout.contains("secret"), "{}", r.stdout);
}

// --- exit ---------------------------------------------------------------------

fn run_repl(
    env: &Env,
    session: Session,
    reader: &mut dyn LineReader,
    stdin: &str,
) -> (Result<(), crate::error::CliError>, String) {
    let reg = registry();
    let session = std::cell::RefCell::new(session);
    let mut history = History::open(None).unwrap();
    let settings = ShellSettings {
        prompt: shell::DEFAULT_PROMPT.into(),
        idle_timeout: None,
        history_file: env.dirs.state.join("history"),
        clipboard_timeout: shell::DEFAULT_CLIPBOARD_TIMEOUT,
    };
    with_io(env, stdin, |io| {
        Repl {
            reg: &reg,
            session: &session,
            reader,
            history: &mut history,
            settings: &settings,
            dirs: &env.dirs,
        }
        .run(io)
    })
}

// Go: TestREPLClosesEveryHeldIdentityOnExit. Driven directly over a
// session the test holds, the only way to look at its keys afterwards.
#[test]
fn repl_closes_every_held_identity_on_exit() {
    let env = Env::new();
    env.init("personal", &[]);
    let p = FakePrompter::new().with_passphrase(PASSPHRASE);
    let tally = Arc::new(Tally::default());
    let mut s = direct_session(&env, &p, tally.clone());
    s.vault(Some("personal")).unwrap();
    assert_eq!(tally.held(), 1);

    let (result, _) = run_repl(&env, s, &mut PlainLineReader, "exit\n");
    result.unwrap();
    assert_eq!(tally.held(), 0, "exited still holding a key");
}

// Go: TestREPLEOFTerminatesLikeExit. Ctrl-D (end of input) is a clean
// exit, keys dropped the same way.
#[test]
fn repl_eof_terminates_like_exit() {
    let env = Env::new();
    env.init("personal", &[]);
    let p = FakePrompter::new().with_passphrase(PASSPHRASE);
    let tally = Arc::new(Tally::default());
    let mut s = direct_session(&env, &p, tally.clone());
    s.vault(Some("personal")).unwrap();

    let (result, _) = run_repl(&env, s, &mut PlainLineReader, "");
    result.unwrap();
    assert_eq!(tally.held(), 0);
}

/// A line reader whose first read is a Ctrl-C.
struct InterruptOnce(bool);

impl LineReader for InterruptOnce {
    fn read_line(&mut self, io: &mut Io, prompt: &str) -> std::io::Result<ReadOutcome> {
        if !self.0 {
            self.0 = true;
            return Ok(ReadOutcome::Interrupted);
        }
        PlainLineReader.read_line(io, prompt)
    }
}

// Go: TestCtrlCAbandonsTheLineNotTheSession
#[test]
fn ctrl_c_abandons_the_line_not_the_session() {
    let env = Env::new();
    env.init("personal", &[]);
    let p = FakePrompter::new().with_passphrase(PASSPHRASE);
    let tally = Arc::new(Tally::default());
    let mut s = direct_session(&env, &p, tally.clone());
    s.vault(Some("personal")).unwrap();

    let (result, out) = run_repl(&env, s, &mut InterruptOnce(false), "status\nexit\n");
    result.unwrap();
    assert!(
        out.contains("personal") && out.contains("unlocked"),
        "{out}"
    );
    assert_eq!(tally.held(), 0);
}

// Go: TestMapReadlineErrTranslatesCtrlC. Ctrl-C and Ctrl-D become their
// outcomes, and every other error passes through, so a real failure
// isn't read as a cancellation.
#[test]
fn map_readline_err_translates_ctrl_c() {
    use rustyline::error::ReadlineError;
    assert!(matches!(
        map_readline(Err(ReadlineError::Interrupted)),
        Ok(ReadOutcome::Interrupted)
    ));
    assert!(matches!(
        map_readline(Err(ReadlineError::Eof)),
        Ok(ReadOutcome::Eof)
    ));
    assert!(matches!(
        map_readline(Ok("ls".into())),
        Ok(ReadOutcome::Line(l)) if l == "ls"
    ));
    let err = map_readline(Err(ReadlineError::Io(std::io::Error::other(
        "the terminal went away",
    ))))
    .unwrap_err();
    assert_eq!(err.to_string(), "the terminal went away");
}

// --- the idle timeout from config ---------------------------------------------

// Go: TestConfiguredIdleTimeoutReachesTheSession. The [shell] value is
// wired to the re-lock: with an hour between reads of the clock, a 10m
// timeout re-locks between every command.
#[test]
fn configured_idle_timeout_reaches_the_session() {
    let env = Env::new();
    env.init("personal", &[]);
    env.add("personal", "ProtonMail", "correcthorsebatterystaple");
    env.set_shell(Shell {
        idle_timeout: "10m".into(),
        ..Shell::default()
    });

    let clock = step_clock(Duration::from_secs(3600));
    let r = env.session_with_clock(
        &script(&[
            "use personal",
            PASSPHRASE,
            "status",
            "show ProtonMail",
            PASSPHRASE,
            "exit",
        ]),
        &clock,
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    let status = r
        .stdout
        .lines()
        .find(|l| l.contains("personal") && l.contains("locked"))
        .unwrap_or_else(|| panic!("no status line:\n{}", r.stdout));
    assert!(!status.contains("unlocked"), "{status}");
    assert_eq!(passphrase_prompts(&r), 2, "{}", r.stderr);
    assert!(
        r.stdout.contains("correcthorsebatterystaple"),
        "{}",
        r.stdout
    );
}

// Go: TestConfiguredIdleTimeoutIsNotAppliedTooEagerly. The other half:
// with a second between reads, nothing ages out, so the test above can't
// be satisfied by a build that re-locks on every command.
#[test]
fn configured_idle_timeout_is_not_applied_too_eagerly() {
    let env = Env::new();
    env.init("personal", &[]);
    env.add("personal", "ProtonMail", "correcthorsebatterystaple");
    env.set_shell(Shell {
        idle_timeout: "10m".into(),
        ..Shell::default()
    });

    let clock = step_clock(Duration::from_secs(1));
    let r = env.session_with_clock(
        &script(&[
            "use personal",
            PASSPHRASE,
            "status",
            "show ProtonMail",
            "exit",
        ]),
        &clock,
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("unlocked"), "{}", r.stdout);
    assert_eq!(passphrase_prompts(&r), 1, "{}", r.stderr);
}

// A malformed idle_timeout refuses to start the session, rather than
// running one that never re-locks.
#[test]
fn a_malformed_idle_timeout_refuses_the_session() {
    let env = Env::new();
    env.init("personal", &[]);
    env.set_shell(Shell {
        idle_timeout: "10min".into(),
        ..Shell::default()
    });
    let r = env.session("exit\n");
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("idle_timeout"), "{}", r.stderr);
}

// --- the prompt -----------------------------------------------------------------

// Go: TestSessionPromptRendersTokensAndTracksLockState. The prompt comes
// from [shell].prompt, and its lock glyph follows the session's state.
#[test]
fn session_prompt_renders_tokens_and_tracks_lock_state() {
    let env = Env::new();
    env.init("personal", &[]);
    env.set_shell(Shell {
        prompt: "[{vault}{lock}{dirty}] gage> ".into(),
        ..Shell::default()
    });

    let r = env.session_ok(&["use personal", PASSPHRASE, "lock personal", "exit"]);
    // personal is current from init, so it shows from the first line:
    // locked, unlocked, locked again.
    let mut rest = r.stdout.as_str();
    for glyph in [LOCKED_GLYPH, UNLOCKED_GLYPH, LOCKED_GLYPH] {
        let want = format!("[personal{glyph}] gage> ");
        let at = rest
            .find(&want)
            .unwrap_or_else(|| panic!("{want:?} never appeared, in order, in:\n{}", r.stdout));
        rest = &rest[at + want.len()..];
    }
}

// --- history --------------------------------------------------------------------

// Go: TestSessionHistoryRecordsCommandsNeverValues. The typed query is
// fine (it was typed); a decrypted value or a passphrase never is.
#[test]
fn session_history_records_commands_never_values() {
    let env = Env::new();
    env.init("personal", &[]);
    env.add("personal", "ProtonMail", "correcthorsebatterystaple");
    env.add("personal", "AWS root account", "root-secret");
    env.add("personal", "AWS IAM backup admin", "iam-secret");

    let r = env.session_ok(&[
        "use personal",
        PASSPHRASE,
        "show ProtonMail",
        "show AWS",
        "2",
        "exit",
    ]);
    assert!(
        r.stdout.contains("correcthorsebatterystaple"),
        "vacuous:\n{}",
        r.stdout
    );

    let history = env.history_text();
    assert_eq!(
        history, "use personal\nshow ProtonMail\nshow AWS\nexit\n",
        "only typed command lines: no passphrase, value or picker answer"
    );
}

// Go: TestSessionHistoryFileIsCreated0600. The file a session creates by
// default; history.rs covers tightening one that already existed.
#[test]
fn session_history_file_is_created_0600() {
    let env = Env::new();
    env.init("personal", &[]);
    env.session_ok(&["status", "exit"]);
    let path = env.dirs.state.join("history");
    assert!(path.exists(), "no history file at {}", path.display());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

// A history file that can't be opened is reported once, and the session
// runs without it.
#[test]
fn an_unopenable_history_file_does_not_stop_the_session() {
    let env = Env::new();
    env.init("personal", &[]);
    // A directory where the file should be.
    let path = env.root.path().join("history-is-a-dir");
    std::fs::create_dir_all(&path).unwrap();
    env.set_shell(Shell {
        history_file: path.to_str().unwrap().into(),
        ..Shell::default()
    });
    let r = env.session_ok(&["status", "exit"]);
    assert!(
        r.stderr.contains("continuing without command history"),
        "{}",
        r.stderr
    );
    assert!(r.stdout.contains(NOTHING_UNLOCKED_YET), "{}", r.stdout);
}

// --- session help ---------------------------------------------------------------

/// The names a help listing line offers (canonical and aliases), argument
/// syntax dropped: `use <vault>` lists `use`, `vault list` stays whole.
fn listed_names(help: &str) -> Vec<String> {
    help.lines()
        .filter(|l| l.starts_with("  ") && !l.starts_with("   "))
        .filter_map(|l| l.trim().split("  ").next())
        .flat_map(|names| names.split(", "))
        .map(|n| {
            n.split_whitespace()
                .filter(|w| !w.starts_with('<') && !w.starts_with('['))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|n| !n.is_empty())
        .collect()
}

fn session_help(args: &str) -> CliResult {
    let env = Env::new();
    let r = env.session(&script(&[format!("help {args}").trim(), "exit"]));
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    r
}

// Go: TestSessionHelpMatchesRegistryExactly. Every session-available
// command and alias is listed, and nothing that isn't available leaks in.
#[test]
fn session_help_matches_registry_exactly() {
    let r = session_help("");
    let listed = listed_names(&r.stdout);
    for c in registry().commands {
        for n in std::iter::once(c.name).chain(c.aliases.iter().copied()) {
            let shown = listed.iter().any(|l| l == n);
            assert_eq!(
                shown,
                c.availability.session_visible(),
                "{n}:\n{}",
                r.stdout
            );
        }
    }
}

// Go: TestSessionHelpListsTheMetaVerbs. The design doc's own list, so the
// registry-derived check above can't be vacuous.
#[test]
fn session_help_lists_the_meta_verbs() {
    let listed = listed_names(&session_help("").stdout);
    for name in ["use", "lock", "status", "whoami", "exit", "quit", "help"] {
        assert!(listed.iter().any(|l| l == name), "{name}: {listed:?}");
    }
}

// Go: TestInitIsNotOfferedInSession
#[test]
fn init_is_not_offered_in_session() {
    let listed = listed_names(&session_help("").stdout);
    assert!(!listed.iter().any(|l| l == "init"), "{listed:?}");
}

// Go: TestBothHelpSurfacesRenderTheSameRegistry. A command available in
// both modes appears in both listings with the same description.
#[test]
fn both_help_surfaces_render_the_same_registry() {
    let env = Env::new();
    let one_shot = env.run_with(&["--help"], &FakePrompter::new()).stdout;
    let session = session_help("").stdout;
    let (one_listed, session_listed) = (listed_names(&one_shot), listed_names(&session));
    let both: Vec<_> = registry()
        .commands
        .into_iter()
        .filter(|c| c.availability == Availability::Both)
        .collect();
    assert!(!both.is_empty(), "vacuous");
    for c in both {
        assert!(one_listed.iter().any(|l| l == c.name), "{}", c.name);
        assert!(session_listed.iter().any(|l| l == c.name), "{}", c.name);
        assert!(one_shot.contains(c.short), "{}", c.name);
        assert!(session.contains(c.short), "{}", c.name);
    }
}

// Go: TestSessionOnlyCommandsNeverLeakIntoOneShotHelp
#[test]
fn session_only_commands_never_leak_into_one_shot_help() {
    let env = Env::new();
    let listed = listed_names(&env.run_with(&["--help"], &FakePrompter::new()).stdout);
    for c in registry().commands {
        if c.availability != Availability::SessionOnly {
            continue;
        }
        for n in std::iter::once(c.name).chain(c.aliases.iter().copied()) {
            assert!(!listed.iter().any(|l| l == n), "{n}: {listed:?}");
        }
    }
}

// Go: TestSessionHelpLeadsWithBareUse. Vault selection is `use <vault>`
// here; the -u/--use flag belongs in per-command help.
#[test]
fn session_help_leads_with_bare_use() {
    let out = session_help("").stdout;
    assert!(out.contains("use <vault>"), "{out}");
    assert!(!out.contains("--use"), "{out}");
}

// Go: TestSessionPerCommandHelpListsTheUseFlag
#[test]
fn session_per_command_help_lists_the_use_flag() {
    let out = session_help("show").stdout;
    assert!(out.contains("--use"), "{out}");
    assert!(out.contains("-u"), "{out}");
    assert!(out.contains("gage show"), "{out}");
    assert!(out.contains("<query>"), "{out}");
}

// Go: TestSessionHelpForAMetaVerbPrintsItsUsage
#[test]
fn session_help_for_a_meta_verb_prints_its_usage() {
    let out = session_help("use").stdout;
    assert!(out.contains("use <vault>"), "{out}");
    let out = session_help("whoami").stdout;
    assert!(out.contains("status"), "{out}");
}

// Go: TestSessionHelpUnknownTopicDoesNotEndTheSession
#[test]
fn session_help_unknown_topic_does_not_end_the_session() {
    let env = Env::new();
    let r = env.session_ok(&["help not-a-real-command", "status", "exit"]);
    assert!(r.stderr.contains("unknown help topic"), "{}", r.stderr);
    assert!(r.stdout.contains(NOTHING_UNLOCKED_YET), "{}", r.stdout);
}
