//! `--script FILE` and `--stdin` through the real CLI, in-process: one
//! session driven by a list of commands instead of a human at a prompt, and
//! where its unlocks get a passphrase from. Ports Go's `script_test.go`
//! (the wrapper rows are `scriptprompt`'s own tests); each test names its
//! Go row.
//!
//! Most runs inject a [`FakePrompter`], which answers without reading
//! anything, so a prompt that reaches it is an unscripted-question panic.
//! The tests about a prompt eating the command stream use the real
//! [`TerminalPrompter`] over the same input the commands arrive on, since
//! only that shape shows what a prompt does to a script.

use std::cell::RefCell;
use std::io::Cursor;
use std::rc::Rc;

use gage::exitcode::Code;
use gage::prompter::{FakePrompter, Prompter};

use crate::entry_tests::unlocks;
use crate::lifecycle_tests::{CliResult, Env, PASSPHRASE};
use crate::prompter::{SecretInput, TerminalPrompter};
use crate::repl_tests::{SharedInput, SharedOutput, script};
use crate::scriptprompt::ENV_VAR;
use crate::{BuildInfo, Io};

/// What a run is given besides its arguments.
#[derive(Default)]
struct Run<'a> {
    /// The command stream (`--stdin`) or whatever a prompt could read.
    stdin: &'a str,
    /// Whether stdin is a terminal.
    tty: bool,
    /// `$GAGE_PASSPHRASE`.
    passphrase_env: Option<&'a str>,
}

impl Env {
    /// A script file of `lines`, returning its path.
    fn script_file(&self, lines: &[&str]) -> String {
        let path = self.root.path().join("commands.gage");
        std::fs::write(&path, script(lines)).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn history_file(&self) -> std::path::PathBuf {
        self.dirs.state.join("history")
    }

    /// Runs `args` with an injected `prompter`.
    fn script_run(&self, args: &[&str], run: Run, prompter: &dyn Prompter) -> CliResult {
        let input = SharedInput(Rc::new(RefCell::new(Cursor::new(
            run.stdin.as_bytes().to_vec(),
        ))));
        self.script_run_over(args, run, input, SharedOutput::default(), prompter)
    }

    /// Runs `args` with the real terminal prompter reading the same input
    /// the commands arrive on, as `main` wires it.
    fn script_run_real_prompter(&self, args: &[&str], run: Run) -> CliResult {
        let input = SharedInput(Rc::new(RefCell::new(Cursor::new(
            run.stdin.as_bytes().to_vec(),
        ))));
        let stderr = SharedOutput::default();
        let prompter = TerminalPrompter::new(input.clone(), stderr.clone(), SecretInput::Lines)
            .interactive(run.tty);
        self.script_run_over(args, run, input, stderr, &prompter)
    }

    fn script_run_over(
        &self,
        args: &[&str],
        run: Run,
        mut stdin: SharedInput,
        mut stderr: SharedOutput,
        prompter: &dyn Prompter,
    ) -> CliResult {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let mut stdout = Vec::new();
        let dirs = self.dirs.clone();
        let hostname = self.hostname.clone();
        let code = {
            let mut io = Io {
                stdin: &mut stdin,
                stdout: &mut stdout,
                stderr: &mut stderr,
                stdin_is_terminal: run.tty,
                line_editor: false,
                build: BuildInfo::stamped(),
                dirs: &move || Ok(dirs.clone()),
                hostname: &move || Ok(hostname.clone()),
                editor: None,
                passphrase_env: run
                    .passphrase_env
                    .map(|p| zeroize::Zeroizing::new(p.into())),
                now: &std::time::Instant::now,
                clipboard: &self.clip.keeper(),
                clipboard_wait: &|d| self.clip.wait(d),
            };
            crate::run(&args, &mut io, prompter)
        };
        CliResult {
            code,
            stdout: String::from_utf8(stdout).unwrap(),
            stderr: stderr.text(),
        }
    }

    /// `gage --script FILE` in the CI shape: no terminal, the variable set.
    fn run_script(&self, lines: &[&str]) -> CliResult {
        let file = self.script_file(lines);
        self.script_run(
            &["--script", &file],
            Run {
                passphrase_env: Some(PASSPHRASE),
                ..Run::default()
            },
            &FakePrompter::new(),
        )
    }

    /// `gage --stdin` with `lines` piped in and the variable set.
    fn run_stdin_script(&self, lines: &[&str]) -> CliResult {
        self.script_run(
            &["--stdin"],
            Run {
                stdin: &script(lines),
                passphrase_env: Some(PASSPHRASE),
                ..Run::default()
            },
            &FakePrompter::new(),
        )
    }
}

fn with_github() -> Env {
    let env = Env::new();
    env.init("personal", &[]);
    env.add("personal", "GitHub", "hunter2");
    env
}

fn ok(r: CliResult) -> CliResult {
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    r
}

// --- the mode itself ---------------------------------------------------------

// Go: TestScriptRunsCommandsNonInteractively. A list of lines runs against
// one session, with no prompt anywhere.
#[test]
fn script_runs_commands_non_interactively() {
    let env = with_github();
    let r = ok(env.run_script(&["use personal", "show GitHub", "ls"]));
    assert!(
        r.stdout.contains("hunter2"),
        "show did not run:\n{}",
        r.stdout
    );
    assert!(r.stdout.contains("GitHub"), "ls did not run:\n{}", r.stdout);
    assert!(
        !r.stdout.contains("gage> "),
        "a prompt was rendered:\n{}",
        r.stdout
    );
}

// Go: TestStdinRunsCommandsNonInteractively. The same, piped.
#[test]
fn stdin_runs_commands_non_interactively() {
    let env = with_github();
    let r = ok(env.run_stdin_script(&["use personal", "show GitHub"]));
    assert!(r.stdout.contains("hunter2"), "{}", r.stdout);
    assert!(!r.stdout.contains("gage> "), "{}", r.stdout);
}

// Go: TestScriptUnlocksEachVaultAtMostOnce. What makes the mode worth
// having over a shell loop of one-shot runs: the key is held across the
// whole run, not re-derived per command.
#[test]
fn script_unlocks_each_vault_at_most_once() {
    let env = with_github();
    env.add("personal", "Proton", "hunter3");
    let lines = ["use personal", "show GitHub", "show Proton", "ls"];

    // The variable answers the unlock, so the prompter underneath sees
    // nothing at all.
    let file = env.script_file(&lines);
    let inner = FakePrompter::new();
    ok(env.script_run(
        &["--script", &file],
        Run {
            passphrase_env: Some(PASSPHRASE),
            ..Run::default()
        },
        &inner,
    ));
    assert!(inner.unlock_requests().is_empty());

    // Through a terminal prompter that can answer once: asked once for
    // four commands. A second ask is an unscripted-unlock panic.
    let inner = unlocks();
    ok(env.script_run(
        &["--script", &file],
        Run {
            tty: true,
            ..Run::default()
        },
        &inner,
    ));
    assert_eq!(
        inner.unlock_requests().len(),
        1,
        "{:?}",
        inner.unlock_requests()
    );
}

// Go: TestScriptAbortsOnTheFirstFailingCommand. A script has nobody to
// notice a line that failed halfway down: later lines don't run, the exit
// code is the failing command's, and the message says where.
#[test]
fn script_aborts_on_the_first_failing_command() {
    let env = with_github();
    let r = env.run_script(&["use personal", "show NoSuchEntry", "show GitHub"]);
    assert_eq!(r.code, Code::NotFound, "{}", r.stderr);
    assert!(
        !r.stdout.contains("hunter2"),
        "ran past a failing line:\n{}",
        r.stdout
    );
    assert!(
        r.stderr.contains("commands.gage:2: show NoSuchEntry"),
        "no source:line: text in {:?}",
        r.stderr
    );
    assert_eq!(r.stderr.matches("gage: ").count(), 1, "{}", r.stderr);
}

// `--stdin` names its stream, and an ambiguous query's error carries the
// candidates a human would have been shown.
#[test]
fn a_failed_stdin_line_names_the_stream_and_keeps_an_ambiguous_list() {
    let env = with_github();
    env.add("personal", "GitHub backup", "hunter3");
    // Exact titles win, so ambiguity takes a substring both share.
    let r = env.run_stdin_script(&["use personal", "show Git"]);
    assert_eq!(r.code, Code::Ambiguous, "{}", r.stderr);
    assert!(r.stderr.contains("<stdin>:2: show Git"), "{}", r.stderr);
    assert!(r.stderr.contains("GitHub backup"), "{}", r.stderr);
}

// Beyond Go's rows (M12d's ambiguity decision): the picker is a prompt too.
// With the real prompter over the command stream, an ambiguous query must
// fail with its list and leave the next line unread, not take it for the
// pick and ask again.
#[test]
fn an_ambiguous_query_under_stdin_does_not_eat_the_command_stream() {
    let env = with_github();
    env.add("personal", "GitHub backup", "hunter3");
    let input = SharedInput(Rc::new(RefCell::new(Cursor::new(
        script(&["use personal", "show Git", "show GitHub"]).into_bytes(),
    ))));
    let stderr = SharedOutput::default();
    let prompter = TerminalPrompter::new(input.clone(), stderr.clone(), SecretInput::Lines);
    let r = env.script_run_over(
        &["--stdin"],
        Run {
            passphrase_env: Some(PASSPHRASE),
            ..Run::default()
        },
        input.clone(),
        stderr,
        &prompter,
    );
    assert_eq!(r.code, Code::Ambiguous, "{}", r.stderr);
    assert!(
        !r.stderr.contains("Which one"),
        "a picker was shown: {}",
        r.stderr
    );
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut input.clone(), &mut rest).unwrap();
    assert_eq!(rest, "show GitHub\n", "a command line was taken for a pick");
}

// And `--script FILE` on a terminal still lets the human pick, as an
// interactive session does.
#[test]
fn script_on_a_terminal_still_lets_a_human_pick() {
    let env = with_github();
    env.add("personal", "GitHub backup", "hunter3");
    let file = env.script_file(&["use personal", "show Git"]);
    let inner = FakePrompter::new().with_choice(0);
    let r = env.script_run(
        &["--script", &file],
        Run {
            tty: true,
            passphrase_env: Some(PASSPHRASE),
            ..Run::default()
        },
        &inner,
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_eq!(inner.choose_lists().len(), 1);
}

// Go: TestScriptWritesNothingToTheHistoryFile. The history file records
// what a human typed at a prompt; a script's lines were never typed.
#[test]
fn script_writes_nothing_to_the_history_file() {
    let env = with_github();
    ok(env.run_script(&["use personal", "show GitHub"]));
    ok(env.run_stdin_script(&["use personal", "show GitHub"]));
    assert!(
        !env.history_file().exists(),
        "a script opened the history file"
    );
}

// Go: TestScriptSkipsBlankLinesAndComments. So a checked-in file can be
// commented like any other.
#[test]
fn script_skips_blank_lines_and_comments() {
    let env = with_github();
    let r = ok(env.run_script(&[
        "# unlock the vault",
        "",
        "use personal",
        "   ",
        "# and read one secret",
        "  # indented too",
        "show GitHub",
    ]));
    assert!(r.stdout.contains("hunter2"), "{}", r.stdout);
}

// Go: TestScriptExitEndsTheRunEarly. Remaining lines unrun, success.
#[test]
fn script_exit_ends_the_run_early() {
    let env = with_github();
    let r = ok(env.run_script(&["use personal", "exit", "show GitHub"]));
    assert!(!r.stdout.contains("hunter2"), "{}", r.stdout);
}

// A line that is only a typed-prompt habit (`quit`) ends it the same way.
#[test]
fn script_quit_is_exit() {
    let env = with_github();
    let r = ok(env.run_script(&["use personal", "quit", "show GitHub"]));
    assert!(!r.stdout.contains("hunter2"), "{}", r.stdout);
}

// The session verbs work in a script, and an error in one stops it.
#[test]
fn script_runs_the_session_verbs_and_stops_on_a_usage_error() {
    let env = with_github();
    let r = env.run_script(&["use personal", "status", "lock", "status"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(r.stdout.contains("personal locked."), "{}", r.stdout);

    let r = env.run_script(&["status extra", "status"]);
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(
        r.stderr.contains("commands.gage:1: status extra"),
        "{}",
        r.stderr
    );
}

// --- the flags ---------------------------------------------------------------

// Go: TestScriptAndStdinAreMutuallyExclusive. Two command streams have no
// meaning together.
#[test]
fn script_and_stdin_are_mutually_exclusive() {
    let env = Env::new();
    let file = env.script_file(&["ls"]);
    let r = env.script_run(
        &["--script", &file, "--stdin"],
        Run::default(),
        &FakePrompter::new(),
    );
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("mutually exclusive"), "{}", r.stderr);
}

// Go: TestScriptFlagsRejectASubcommand. They choose where a whole
// session's lines come from, so combining one with a one-shot command asks
// for two different things at once.
#[test]
fn script_flags_reject_a_subcommand() {
    let env = with_github();
    let file = env.script_file(&["ls"]);
    for args in [
        vec!["--stdin", "ls"],
        vec!["--script", &file, "show", "GitHub"],
        vec!["ls", "--stdin"],
    ] {
        let r = env.script_run(
            &args,
            Run {
                passphrase_env: Some(PASSPHRASE),
                ..Run::default()
            },
            &FakePrompter::new(),
        );
        assert_eq!(r.code, Code::Usage, "{args:?}: {}", r.stderr);
        assert!(
            r.stderr.contains("can't be combined"),
            "{args:?}: {}",
            r.stderr
        );
        assert!(!r.stdout.contains("GitHub"), "{args:?} ran the command");
    }
}

// Go: TestScriptFlagsAreRefusedInsideASession. They *start* a session, so
// asking for one from inside a session is a category error. Unreachable
// from the prompt as a bare flag (a line of only flags is an unknown
// command), so the contract is pinned at the runner a parsed line reaches.
#[test]
fn script_flags_are_refused_inside_a_session() {
    use crate::cli::{self, Outcome};
    use crate::registry::registry;

    let reg = registry();
    for flag in [vec!["--stdin"], vec!["--script", "session.gage"]] {
        let args: Vec<String> = flag.iter().map(|s| s.to_string()).collect();
        let parsed = cli::parse(&reg, &BuildInfo::stamped(), &args, true).unwrap();
        assert!(matches!(parsed, Outcome::Script(_)), "{flag:?}: {parsed:?}");

        let env = Env::new();
        let r = env.session(&script(&[&flag.join(" "), "status", "exit"]));
        assert_eq!(r.code, Code::Success, "{}", r.stderr);
        assert!(
            r.stderr.contains("unknown command") || r.stderr.contains("aren't available inside"),
            "{flag:?} was accepted in a session: {}",
            r.stderr
        );
    }
    let err = crate::repl::refuse_script_flags_in_session();
    assert_eq!(err.code(), Code::Usage);
    assert!(
        err.to_string().contains("aren't available inside one"),
        "{err}"
    );
}

// Go: TestMissingScriptFileIsAUsageError, rather than an internal one: the
// path came off the command line.
#[test]
fn a_missing_script_file_is_a_usage_error() {
    let env = Env::new();
    let missing = env.root.path().join("nope.gage");
    let r = env.script_run(
        &["--script", missing.to_str().unwrap()],
        Run::default(),
        &FakePrompter::new(),
    );
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("opening script"), "{}", r.stderr);
}

// Go: TestScriptAcceptsAssumeYes. A non-interactive session is the case
// `--yes` exists for. Amends Q-RECIPIENT-PROMPT; the refusal in a session
// with a human at it is `trustcache_cli_tests`.
#[test]
fn script_accepts_assume_yes() {
    let env = with_github();
    let file = env.script_file(&["use personal", "show GitHub"]);
    for args in [
        vec!["--script", &file, "--yes"],
        vec!["--yes", "--script", &file],
    ] {
        let r = env.script_run(
            &args,
            Run {
                passphrase_env: Some(PASSPHRASE),
                ..Run::default()
            },
            &FakePrompter::new(),
        );
        assert_eq!(r.code, Code::Success, "{args:?}: {}", r.stderr);
        assert!(r.stdout.contains("hunter2"), "{args:?}: {}", r.stdout);
    }
    let r = env.script_run(
        &["--stdin", "--yes"],
        Run {
            stdin: &script(&["use personal", "show GitHub"]),
            passphrase_env: Some(PASSPHRASE),
            ..Run::default()
        },
        &FakePrompter::new(),
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
}

// `--yes` really is what answers the recipient question in a script: a
// changed recipient list is approved without a prompt, and without
// `--yes` the same script is refused (a script has nobody to ask).
#[test]
fn script_assume_yes_answers_the_recipient_change_and_without_it_the_write_is_refused() {
    use crate::trustcache_cli_tests::{env_with_vault, new_key, tamper_recipients_file};

    let lines = ["use personal", "insert Router"];
    let run = |env: &Env, flags: &[&str]| {
        let file = env.script_file(&lines);
        let mut args = vec!["--script", file.as_str()];
        args.extend_from_slice(flags);
        env.script_run(
            &args,
            Run {
                passphrase_env: Some(PASSPHRASE),
                ..Run::default()
            },
            &FakePrompter::new().with_value("s3cret"),
        )
    };

    let env = env_with_vault();
    env.bootstrap_trust();
    tamper_recipients_file(&env.vault_dir(), &new_key());
    let r = run(&env, &[]);
    assert_eq!(r.code, Code::Conflict, "{}", r.stderr);
    assert!(
        r.stderr.contains("commands.gage:2: insert Router"),
        "{}",
        r.stderr
    );

    let env = env_with_vault();
    env.bootstrap_trust();
    tamper_recipients_file(&env.vault_dir(), &new_key());
    let r = run(&env, &["--yes"]);
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
}

// A session line's own `--yes` is still refused where a human is typing it,
// and accepted (it is a no-op beside the run's own) in a script.
#[test]
fn a_yes_on_a_script_line_is_accepted() {
    let env = with_github();
    let r = ok(env.run_script(&["use personal", "show GitHub --yes"]));
    assert!(r.stdout.contains("hunter2"), "{}", r.stdout);
}

// Go: TestSessionCommandArgumentCountsAreChecked, the --stdin half. A wrong
// argument count prints that command's usage and fails the run, at the line
// that has it.
#[test]
fn stdin_session_command_argument_counts_are_checked() {
    for (line, want) in [
        ("use", "usage: use <vault>"),
        ("use personal work", "usage: use <vault>"),
        ("lock personal work", "usage: lock [vault]"),
        ("status extra", "usage: status"),
    ] {
        let env = Env::new();
        env.init("personal", &[]);
        let r = env.run_stdin_script(&[line, "status"]);
        assert_eq!(r.code, Code::Usage, "{line}: {}", r.stderr);
        assert!(r.stderr.contains(want), "{line}: {}", r.stderr);
        assert!(
            r.stderr.contains(&format!("<stdin>:1: {line}")),
            "{line}: {}",
            r.stderr
        );
        assert!(
            !r.stdout.contains("no vaults have been unlocked"),
            "{line}: the run went on past the usage error:\n{}",
            r.stdout
        );
    }
}

// A clap rejection inside a script carries the command's own usage, as it
// does one-shot (Go: usageerror TestSessionWrongArgCountShowsCommandUsage,
// through --stdin as Go drives it).
#[test]
fn stdin_wrong_arg_count_shows_command_usage() {
    let env = with_github();
    let r = env.run_stdin_script(&["edit"]);
    assert_eq!(r.code, Code::Usage, "{}", r.stderr);
    assert!(r.stderr.contains("<query>"), "{}", r.stderr);
    assert!(r.stderr.contains("Usage:\n  gage edit"), "{}", r.stderr);
}

// Beyond Go's rows: M12c's `-c` in a session schedules its clear, and the
// session's own end is what keeps a copy from outliving the process. A
// script run goes through the same exit path.
#[test]
fn a_script_ends_by_clearing_a_pending_copy() {
    let env = with_github();
    ok(env.run_script(&["use personal", "show GitHub -c"]));
    assert_eq!(
        env.clip.cb.writes().first().map(String::as_str),
        Some("hunter2")
    );
    assert_eq!(env.clip.cb.get(), "", "the copy outlived the script");
    assert!(
        env.clip.timer.stopped(),
        "the pending clear's timer was left running"
    );

    // And a failing line is not an excuse to leave it there.
    let env = with_github();
    let r = env.run_script(&["use personal", "show GitHub -c", "show Nope"]);
    assert_eq!(r.code, Code::NotFound, "{}", r.stderr);
    assert_eq!(env.clip.cb.get(), "", "the copy outlived a failed script");
}

// Every key a script held is dropped when it ends, on the failing path
// too: the next run unlocks again.
#[test]
fn a_script_does_not_leave_a_key_for_the_next_run() {
    let env = with_github();
    let file = env.script_file(&["use personal", "show Nope"]);
    for _ in 0..2 {
        let inner = FakePrompter::new();
        let r = env.script_run(
            &["--script", &file],
            Run {
                passphrase_env: Some(PASSPHRASE),
                ..Run::default()
            },
            &inner,
        );
        assert_eq!(r.code, Code::NotFound, "{}", r.stderr);
    }
}

// --- where the passphrase comes from ------------------------------------------

// Go: TestGagePassphraseUnlocksWithoutPromptingOrReadingTheCommandStream.
// Under --stdin the command stream is the prompter's input, so a prompt
// would eat the next command.
#[test]
fn gage_passphrase_unlocks_without_prompting_or_reading_the_command_stream() {
    let env = with_github();
    let r = env.script_run_real_prompter(
        &["--stdin"],
        Run {
            stdin: &script(&["use personal", "show GitHub"]),
            passphrase_env: Some(PASSPHRASE),
            ..Run::default()
        },
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert!(
        r.stdout.contains("hunter2"),
        "a prompt ate a command:\n{}\n{}",
        r.stdout,
        r.stderr
    );
    assert!(
        !r.stderr.contains("assphrase"),
        "a prompt was rendered: {}",
        r.stderr
    );
}

// Go: TestWrongGagePassphraseFailsAfterOneAttempt. The variable gives the
// same answer every time, so retrying it would only fail identically.
#[test]
fn wrong_gage_passphrase_fails_after_one_attempt() {
    let env = with_github();
    let file = env.script_file(&["use personal", "show GitHub"]);
    let inner = FakePrompter::new();
    let r = env.script_run(
        &["--script", &file],
        Run {
            passphrase_env: Some("not the right passphrase"),
            ..Run::default()
        },
        &inner,
    );
    assert_eq!(r.code, Code::LockedOrAuth, "{}", r.stderr);
    assert!(!r.stdout.contains("hunter2"), "{}", r.stdout);
    assert!(
        r.stderr.contains(ENV_VAR),
        "does not name the variable: {}",
        r.stderr
    );
    assert!(
        r.stderr.contains("commands.gage:1: use personal"),
        "{}",
        r.stderr
    );
    // gage names itself once, not again inside a wrapped refusal.
    assert_eq!(r.stderr.matches("gage: ").count(), 1, "{}", r.stderr);
    assert!(
        inner.unlock_requests().is_empty(),
        "the prompter underneath was asked"
    );
}

// An empty variable is set, and is a wrong passphrase, not "unset".
#[test]
fn an_empty_gage_passphrase_is_set_and_is_refused_as_empty() {
    let env = with_github();
    let file = env.script_file(&["use personal"]);
    let r = env.script_run(
        &["--script", &file],
        Run {
            passphrase_env: Some(""),
            ..Run::default()
        },
        &FakePrompter::new(),
    );
    assert_eq!(r.code, Code::LockedOrAuth, "{}", r.stderr);
}

// Go: TestGagePassphraseIsIgnoredForANewIdentity, end to end: the variable
// unlocks the vault, and `identity add`'s new passphrase still reaches the
// prompter underneath. The run has to get as far as `identity add`, or the
// request log is empty and proves nothing.
#[test]
fn gage_passphrase_is_ignored_for_a_new_identity() {
    let env = with_github();
    let inner = FakePrompter::new().with_passphrase("chosen by a human");
    let file = env.script_file(&["use personal", "identity add --device other-box"]);
    let r = ok(env.script_run(
        &["--script", &file],
        Run {
            tty: true,
            passphrase_env: Some(PASSPHRASE),
            ..Run::default()
        },
        &inner,
    ));
    assert!(r.stdout.contains("other-box"), "{}", r.stdout);
    let asked = inner.unlock_requests();
    assert!(
        !asked.is_empty(),
        "the variable answered the new identity's passphrase"
    );
    assert!(
        asked
            .iter()
            .all(|q| q.purpose == gage::prompter::Purpose::Create),
        "the variable's answer was bypassed for an unlock: {asked:?}"
    );
    assert_eq!(crate::identity_cli_tests::identity_files(&env).len(), 2);
}

// Go: TestScriptWithNoPassphraseSourceFailsFast. No variable and no
// terminal: there is nowhere a passphrase could come from, and saying so
// beats a read that can't be answered or an empty passphrase reported as a
// wrong one. Exit 5, naming the variable, and no read.
#[test]
fn script_with_no_passphrase_source_fails_fast() {
    let env = with_github();
    let file = env.script_file(&["use personal", "show GitHub"]);
    for (name, args, stdin) in [
        (
            "--stdin",
            vec!["--stdin"],
            script(&["use personal", "show GitHub"]),
        ),
        ("--script", vec!["--script", file.as_str()], String::new()),
    ] {
        let r = env.script_run_real_prompter(
            &args,
            Run {
                stdin: &stdin,
                ..Run::default()
            },
        );
        assert_eq!(r.code, Code::LockedOrAuth, "{name}: {}", r.stderr);
        assert!(r.stderr.contains(ENV_VAR), "{name}: {}", r.stderr);
        assert!(
            !r.stdout.contains("hunter2"),
            "{name} decrypted with no source"
        );
        assert!(
            !r.stderr.contains("assphrase:"),
            "{name} prompted: {}",
            r.stderr
        );
    }
}

// Go: TestScriptOnATerminalStillPrompts. `--script FILE` leaves stdin free,
// which is why a human can run one and answer the prompt.
#[test]
fn script_on_a_terminal_still_prompts() {
    let env = with_github();
    let file = env.script_file(&["use personal", "show GitHub"]);
    let inner = unlocks();
    let r = ok(env.script_run(
        &["--script", &file],
        Run {
            tty: true,
            ..Run::default()
        },
        &inner,
    ));
    assert_eq!(
        inner.unlock_requests().len(),
        1,
        "the ordinary prompter should ask once"
    );
    assert!(r.stdout.contains("hunter2"), "{}", r.stdout);

    // And through the real prompter, reading the passphrase off stdin.
    let r = ok(env.script_run_real_prompter(
        &["--script", &file],
        Run {
            stdin: &script(&[PASSPHRASE]),
            tty: true,
            ..Run::default()
        },
    ));
    assert!(
        r.stderr.contains("Enter passphrase for vault"),
        "{}",
        r.stderr
    );
    assert!(r.stdout.contains("hunter2"), "{}", r.stdout);
}

// Go: TestStdinNeverPromptsEvenOnATerminal. Stdin is already the command
// stream, so prompting on it would race the script for the same bytes.
#[test]
fn stdin_never_prompts_even_on_a_terminal() {
    let env = with_github();
    let inner = FakePrompter::new();
    let r = env.script_run(
        &["--stdin"],
        Run {
            stdin: &script(&["use personal", "show GitHub"]),
            tty: true,
            ..Run::default()
        },
        &inner,
    );
    assert_eq!(r.code, Code::LockedOrAuth, "{}", r.stderr);
    assert!(inner.unlock_requests().is_empty(), "--stdin prompted");
    assert!(r.stderr.contains(ENV_VAR), "{}", r.stderr);
}

// Go: TestStdinRefusesToCreateAnIdentityWithNoHuman. The variable doesn't
// answer for a new identity, and under --stdin the human it defers to
// doesn't exist: the real prompter's input *is* the command stream. The
// refusal is explicit, names the reason, and consumes nothing.
#[test]
fn stdin_refuses_to_create_an_identity_with_no_human() {
    let env = with_github();
    let r = env.script_run_real_prompter(
        &["--stdin"],
        Run {
            stdin: &script(&["use personal", "identity add --device other-box"]),
            passphrase_env: Some(PASSPHRASE),
            ..Run::default()
        },
    );
    assert_eq!(r.code, Code::LockedOrAuth, "{}", r.stderr);
    assert!(r.stderr.contains("human"), "{}", r.stderr);
    let all = format!("{}{}", r.stdout, r.stderr);
    assert!(
        !all.contains("Choose a passphrase"),
        "a prompt was rendered: {all}"
    );
    assert!(
        !all.contains("reading input"),
        "the unanswerable read happened: {all}"
    );
}

// Go: TestStdinIdentityAddDoesNotEatTheCommandStream. Whatever happens, the
// lines after `identity add` are commands, never a passphrase.
#[test]
fn stdin_identity_add_does_not_eat_the_command_stream() {
    let env = with_github();
    let input = SharedInput(Rc::new(RefCell::new(Cursor::new(
        script(&[
            "use personal",
            "identity add --device other-box",
            "show GitHub",
        ])
        .into_bytes(),
    ))));
    let stderr = SharedOutput::default();
    let prompter = TerminalPrompter::new(input.clone(), stderr.clone(), SecretInput::Lines);
    let r = env.script_run_over(
        &["--stdin"],
        Run {
            passphrase_env: Some(PASSPHRASE),
            ..Run::default()
        },
        input.clone(),
        stderr,
        &prompter,
    );
    assert_eq!(r.code, Code::LockedOrAuth, "{}", r.stderr);
    assert!(
        !r.stdout.contains("hunter2"),
        "the script went past a failed line"
    );
    // The refused line stopped the run with the next line untouched: it is
    // still in the stream, unread.
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut input.clone(), &mut rest).unwrap();
    assert_eq!(
        rest, "show GitHub\n",
        "a command line was consumed as input"
    );
    assert_eq!(
        crate::identity_cli_tests::identity_files(&env).len(),
        1,
        "an identity was created anyway"
    );
}

// Go: TestScriptOnATerminalStillCreatesIdentitiesInteractively. The refusal
// is scoped to "there is no human", not to scripts: `--script FILE` leaves
// stdin free, so a human running one can still answer.
#[test]
fn script_on_a_terminal_still_creates_identities_interactively() {
    let env = with_github();
    let file = env.script_file(&["use personal", "identity add --device other-box"]);
    let r = ok(env.script_run_real_prompter(
        &["--script", &file],
        Run {
            stdin: &script(&[PASSPHRASE, PASSPHRASE]),
            tty: true,
            passphrase_env: Some(PASSPHRASE),
        },
    ));
    assert!(
        r.stdout.contains("other-box"),
        "the identity was not registered:\n{}",
        r.stdout
    );
    assert!(r.stderr.contains("Choose a passphrase"), "{}", r.stderr);
    assert_eq!(crate::identity_cli_tests::identity_files(&env).len(), 2);
}

// One-shot commands never read the variable: it is a script-mode source,
// as in Go, where only the script prompter looks at it.
#[test]
fn a_one_shot_command_ignores_gage_passphrase() {
    let env = with_github();
    let inner = unlocks();
    let r = env.script_run(
        &["show", "GitHub"],
        Run {
            passphrase_env: Some("not the passphrase"),
            ..Run::default()
        },
        &inner,
    );
    assert_eq!(r.code, Code::Success, "{}", r.stderr);
    assert_eq!(inner.unlock_requests().len(), 1);
}

// Parity: `--script`, `--stdin` and `--yes` are hidden root flags, as in
// Go's help, where `root.txt` lists none and no command's help shows them
// as inherited.
#[test]
fn the_script_flags_are_hidden_root_flags() {
    let fixture = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/go/help/root.txt"),
    )
    .unwrap();
    let env = Env::new();
    for flag in ["--script", "--stdin", "--yes"] {
        assert!(!fixture.contains(flag), "Go's root help lists {flag}");
        for args in [
            &["--help"][..],
            &["help"],
            &["show", "--help"],
            &["help", "show"],
        ] {
            let r = env.run_with(args, &FakePrompter::new());
            assert!(
                !r.stdout.contains(flag),
                "{args:?} lists {flag}:\n{}",
                r.stdout
            );
        }
    }
}
