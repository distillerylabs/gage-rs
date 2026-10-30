//! Session mode: bare `gage` on a terminal. One [`Session`] held across
//! many typed commands, read through a [`LineReader`]. Everything here is
//! wiring; the behavior it drives (unlock once, hold, re-lock on `lock` or
//! on going idle) belongs to the library's `Session`, where it is tested.
//! See "Session model" in the design doc.
//!
//! A typed line runs through the same clap tree and the same command
//! handlers one-shot mode does, with the session's key borrowed instead of
//! a fresh unlock. Only the session's own verbs (`use`, `lock`, `status`,
//! `exit`, `help`) are handled here, before clap.

use std::cell::RefCell;
use std::io::{BufReader, Read};
use std::rc::Rc;

use crate::exitcode::ExitCode;
use gage::Prompter;
use gage::storage::config;
use gage::storage::paths;
use gage::storage::xdgpaths::Dirs;
use gage::{Session, SessionConfig};

use crate::Io;
use crate::cli::{self, Outcome, ScriptSource};
use crate::commands::{self, Keys};
use crate::complete::{SessionHelper, resolve_session_command_name};
use crate::error::{CliError, prefixed};
use crate::history::History;
use crate::lineread::{LineReader, PlainLineReader, ReadOutcome, RustylineReader, read_one_line};
use crate::registry::Registry;
use crate::shell::{self, PromptState, ShellSettings};

/// What the interactive session and a script run share: where gage keeps
/// things, the `[shell]` settings, and the [`Session`] itself. Each is
/// driven differently from here (a line editor and a history file, or a
/// list of lines) and ends the same way: see [`finish`].
struct Opened<'a> {
    dirs: Dirs,
    settings: ShellSettings,
    /// Shared with the line editor's completer, which reads the session
    /// while a line is being typed; a loop holds it mutably only while a
    /// command runs, so the two never overlap.
    session: Rc<RefCell<Session<'a>>>,
}

fn open<'a>(
    dirs: Result<Dirs, gage::storage::xdgpaths::Error>,
    now: impl Fn() -> std::time::Instant + 'a,
    prompter: &'a dyn Prompter,
) -> Result<Opened<'a>, CliError> {
    let dirs = dirs?;
    let global = config::load_or_default(&paths::global_config_file(&dirs))?;
    let settings = shell::resolve(&global.shell, &dirs, &std::env::home_dir)?;

    let open_dirs = dirs.clone();
    let session = Rc::new(RefCell::new(Session::new(SessionConfig {
        // The same lookup one-shot `--use` resolves through, so both
        // modes agree on what a vault name means.
        open: Box::new(move |name| commands::open_registered(&open_dirs, name)),
        prompter,
        idle_timeout: settings.idle_timeout,
        // A session starts pointed where a one-shot command would, so
        // `show foo` works before any `use`. Naming it doesn't unlock it.
        current: global.current,
        now: Some(Box::new(now)),
    })));
    Ok(Opened {
        dirs,
        settings,
        session,
    })
}

/// How every session ends, whichever way: drops every key it holds, an
/// error included.
///
/// The clipboard too: a session's `-c` schedules its clear and hands the
/// prompt back, so the session's own end is what keeps a copy made a
/// second before `exit` from outliving the process that made it. A failed
/// clear is reported and doesn't change how the session ended.
fn finish(
    session: &RefCell<Session>,
    io: &mut Io,
    result: Result<(), CliError>,
) -> Result<(), CliError> {
    session.borrow_mut().close();
    if let Err(e) = io.clipboard.clear() {
        report(io, &e);
    }
    result
}

/// Runs a session until `exit` or end of input. Every key it held is
/// dropped on the way out, whichever way that is.
pub fn run_session(reg: &Registry, io: &mut Io, prompter: &dyn Prompter) -> Result<(), CliError> {
    let Opened {
        dirs,
        settings,
        session,
    } = open((io.dirs)(), io.now, prompter)?;

    let mut history = History::open(Some(&settings.history_file)).unwrap_or_else(|e| {
        // Losing recall is no reason to refuse a session: say so once.
        let _ = writeln!(io.stderr, "gage: continuing without command history: {e}");
        History::open(None).expect("a disabled history always opens")
    });
    let mut reader: Box<dyn LineReader + '_> = if io.line_editor {
        let helper = SessionHelper::new(
            reg.clone(),
            cli::build(reg, &io.build),
            dirs.clone(),
            Rc::clone(&session),
        );
        Box::new(RustylineReader::new(&history.lines(), helper).map_err(|e| {
            CliError::new(ExitCode::Internal, format!("starting the line editor: {e}"))
        })?)
    } else {
        Box::new(PlainLineReader)
    };

    Repl {
        reg,
        session: &session,
        reader: reader.as_mut(),
        history: &mut history,
        settings: &settings,
        dirs: &dirs,
    }
    .run(io)
}

/// `--script FILE` and `--stdin`: one session driven by a list of commands
/// instead of a human at a prompt. The same [`Session`], holding keys the
/// same way and dropping them at the same point; what differs is
/// deliberate and is the whole of this function:
///
/// - It stops at the first failing line instead of reporting and carrying
///   on. A script has nobody to notice a line that failed halfway down,
///   and continuing would run the rest against a state its author never
///   anticipated. The error names the source, the line number and the
///   line, and keeps that line's exit code.
/// - It opens no history file. The history file is a record of what a
///   human typed at a prompt (see "Command history must never contain
///   plaintext"); a script's lines were never typed, and appending them
///   would mix a machine's command list into a human's recall buffer.
///
/// `prompter` is the script's own (see [`crate::scriptprompt`]): it is the
/// session's, and what a line that asks something directly gets too, so no
/// prompt in the run can read the stream the commands arrive on.
pub fn run_script(
    reg: &Registry,
    io: &mut Io,
    prompter: &dyn Prompter,
    source: &ScriptSource,
) -> Result<(), CliError> {
    // Before any session opens: a missing file is the command line's fault.
    let mut lines = Lines::open(source)?;
    let name = lines.name();
    let Opened { session, .. } = open((io.dirs)(), io.now, prompter)?;

    let result = (|| {
        for n in 1usize.. {
            let line = match lines.next(io) {
                Ok(Some(line)) => line,
                Ok(None) => return Ok(()),
                Err(e) => {
                    return Err(CliError::new(
                        ExitCode::Internal,
                        format!("reading {name}: {e}"),
                    ));
                }
            };
            let line = line.trim();
            // Blank lines and `#` comments, so a checked-in script can be
            // commented like any other file.
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match exec_line_in(
                reg,
                &mut session.borrow_mut().begin_command(),
                io,
                line,
                Mode::Script,
            ) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(e) => return Err(script_error(e, &name, n, line)),
            }
        }
        unreachable!("the range has no end")
    })();
    finish(&session, io, result)
}

/// A failed line's error with where it came from, in the one place that
/// can locate a failure in a file nobody watched run, keeping the exit
/// code a script should fail with: the one its command would have.
pub(crate) fn script_error(err: CliError, source: &str, line_no: usize, line: &str) -> CliError {
    err.context(format!("{source}:{line_no}: {line}"))
}

/// The lines of a script, one at a time and never ahead.
enum Lines {
    File {
        path: String,
        file: BufReader<std::fs::File>,
    },
    /// The command stream is stdin, which a prompt would also read; one
    /// line at a time, so whatever follows belongs to whoever reads next.
    Stdin,
}

impl Lines {
    fn open(source: &ScriptSource) -> Result<Lines, CliError> {
        match source {
            ScriptSource::Stdin => Ok(Lines::Stdin),
            ScriptSource::File(path) => {
                let file = std::fs::File::open(path)
                    .map_err(|e| CliError::new(ExitCode::Usage, format!("opening script: {e}")))?;
                Ok(Lines::File {
                    path: path.clone(),
                    file: BufReader::new(file),
                })
            }
        }
    }

    /// What messages call the source.
    fn name(&self) -> String {
        match self {
            Lines::File { path, .. } => path.clone(),
            Lines::Stdin => crate::scriptprompt::STDIN_SOURCE.to_string(),
        }
    }

    fn next(&mut self, io: &mut Io) -> std::io::Result<Option<String>> {
        match self {
            Lines::File { file, .. } => read_one_line(file as &mut dyn Read),
            Lines::Stdin => read_one_line(io.stdin),
        }
    }
}

/// One running session's loop and what it drives. Its parts are fields so
/// a test can run one over a session it holds itself, which is how "every
/// key is dropped on the way out" is checked at this level.
pub struct Repl<'r, 'a> {
    pub reg: &'r Registry,
    pub session: &'r RefCell<Session<'a>>,
    pub reader: &'r mut dyn LineReader,
    pub history: &'r mut History,
    pub settings: &'r ShellSettings,
    pub dirs: &'r Dirs,
}

impl Repl<'_, '_> {
    /// Reads, records and runs one line at a time until `exit` or end of
    /// input, then ends the session as every session ends ([`finish`]).
    pub fn run(&mut self, io: &mut Io) -> Result<(), CliError> {
        let result = self.run_lines(io);
        finish(self.session, io, result)
    }

    fn run_lines(&mut self, io: &mut Io) -> Result<(), CliError> {
        loop {
            let prompt = shell::render_prompt(&self.settings.prompt, &self.prompt_state());
            let line = match self.reader.read_line(io, &prompt) {
                Ok(ReadOutcome::Line(line)) => line,
                Ok(ReadOutcome::Interrupted) => continue,
                Ok(ReadOutcome::Eof) => return Ok(()),
                Err(e) => {
                    return Err(CliError::new(
                        ExitCode::Internal,
                        format!("reading a command: {e}"),
                    ));
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            // The typed line, and only it: see "Command history must
            // never contain plaintext". Failing to record it is reported
            // and the session goes on.
            if let Err(e) = self.history.add(&line) {
                report(io, &e);
            }
            self.reader.add_history(&line);
            // One line is one command: the idle timeout runs once before
            // it, never between its parts.
            let result = exec_line(
                self.reg,
                &mut self.session.borrow_mut().begin_command(),
                io,
                &line,
            );
            match result {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                // The human is right there and can retype the line.
                Err(e) => report(io, &e),
            }
        }
    }

    /// The prompt's view of the session. Lock state comes from `status`,
    /// which applies the idle timeout first: what lets the prompt drop to
    /// locked on its own.
    fn prompt_state(&mut self) -> PromptState {
        let mut session = self.session.borrow_mut();
        let Some(vault) = session.current().map(str::to_string) else {
            return PromptState::default();
        };
        let unlocked = session
            .status()
            .iter()
            .any(|st| st.name == vault && st.unlocked);
        let dirty =
            shell::wants_dirty(&self.settings.prompt) && shell::vault_is_dirty(&vault, self.dirs);
        PromptState {
            vault: Some(vault),
            unlocked,
            dirty,
        }
    }
}

fn report(io: &mut Io, err: &CliError) {
    let _ = writeln!(io.stderr, "{}", prefixed(err));
}

/// Whether a line is typed by a human or read from a script. They differ in
/// one thing: `--yes` on a line.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Interactive,
    Script,
}

/// Runs one typed line. `Ok(true)` is `exit`/`quit`.
pub fn exec_line(
    reg: &Registry,
    session: &mut Session,
    io: &mut Io,
    line: &str,
) -> Result<bool, CliError> {
    exec_line_in(reg, session, io, line, Mode::Interactive)
}

fn exec_line_in(
    reg: &Registry,
    session: &mut Session,
    io: &mut Io,
    line: &str,
    mode: Mode,
) -> Result<bool, CliError> {
    let mut args = split_line(line)?;
    let Some(first) = args.first() else {
        return Ok(false);
    };
    // An unambiguous abbreviation of a top-level command runs it; the
    // first token only, so subcommands and flags are never abbreviated.
    args[0] = resolve_session_command_name(reg, first)?;
    let first = &args[0];
    let rest = &args[1..];
    match reg.find(first).map(|c| c.name) {
        Some("exit") => return Ok(true),
        Some("use") => session_use(session, rest)?,
        Some("lock") => session_lock(session, io, rest)?,
        Some("status") => session_status(session, io, rest)?,
        Some("help") => session_help(reg, io, rest)?,
        _ => run_command(reg, session, io, &args, mode)?,
    }
    Ok(false)
}

/// Everything that isn't a session verb: through the one-shot clap tree,
/// with the session's key.
fn run_command(
    reg: &Registry,
    session: &mut Session,
    io: &mut Io,
    args: &[String],
    mode: Mode,
) -> Result<(), CliError> {
    // init (and later clone) create a vault rather than operating on one,
    // which leaves "does the new vault become current?" unanswered. Read
    // off the registry, so a later one-shot-only command is covered the
    // day it's registered.
    if let Some(info) = reg.find(&command_path(reg, args))
        && !info.availability.session_visible()
    {
        return Err(CliError::new(
            ExitCode::Usage,
            format!(
                "{0} is a one-shot command and isn't available in a session; run `gage {0}` from your shell instead",
                info.name
            ),
        ));
    }
    let root = cli::build(reg, &io.build);
    if cli::reached_path(&root, args).is_empty() {
        return Err(CliError::new(
            ExitCode::Usage,
            format!(
                "unknown command {:?}; type `help` for what's available in a session",
                args[0]
            ),
        ));
    }
    match cli::parse(reg, &io.build, args, true)? {
        Outcome::Print(text) => {
            print(io, &text);
            Ok(())
        }
        Outcome::Session => Ok(()),
        Outcome::Script(_) => Err(refuse_script_flags_in_session()),
        // A script run is the case `--yes` exists for, and the run's own
        // `--yes` is what covers it: on a line it changes nothing, the
        // question being asked through the prompter the session holds.
        // Only a human at a prompt is refused.
        Outcome::Command(_, matches)
            if mode == Mode::Interactive && crate::assumeyes::given(&matches) =>
        {
            Err(crate::assumeyes::refused_in_session())
        }
        Outcome::Command(name, matches) => {
            commands::run(name, &matches, io, &mut Keys::Session(session)).unwrap_or_else(|| {
                Err(CliError::new(
                    ExitCode::Internal,
                    format!("{name} is not implemented yet"),
                ))
            })
        }
    }
}

/// `--script`/`--stdin` *start* a session, so asking for one from inside a
/// session is a category error. A bare flag never gets this far (a line of
/// only flags is an unknown command, and with a command it is refused at
/// parse), so this guards the day one does: a session quietly trying to
/// take over its own command stream.
pub(crate) fn refuse_script_flags_in_session() -> CliError {
    CliError::new(
        ExitCode::Usage,
        "--script/--stdin start a session and aren't available inside one",
    )
}

/// The registry name a line starts with: two words when they name a
/// nested command (`vault list`), else the first. Registry names are at
/// most two words.
fn command_path(reg: &Registry, args: &[String]) -> String {
    if let [first, second, ..] = args {
        let nested = format!("{first} {second}");
        if reg.find(&nested).is_some() {
            return nested;
        }
    }
    args[0].clone()
}

/// `use <vault>`: unlock if needed, and make it the vault later bare
/// commands use.
fn session_use(session: &mut Session, args: &[String]) -> Result<(), CliError> {
    let [name] = args else {
        return Err(CliError::new(ExitCode::Usage, "usage: use <vault>"));
    };
    session.use_vault(name).map_err(commands::session_failure)
}

/// `lock [vault]`: one vault, or every one this session holds. It reports
/// what was actually holding a key, so a bare `lock` over a session where
/// some vaults already re-locked doesn't claim to have locked them.
fn session_lock(session: &mut Session, io: &mut Io, args: &[String]) -> Result<(), CliError> {
    let name = match args {
        [] => None,
        [name] => Some(name.as_str()),
        _ => return Err(CliError::new(ExitCode::Usage, "usage: lock [vault]")),
    };
    let targets: Vec<String> = match name {
        Some(name) => vec![name.to_string()],
        None => session
            .status()
            .into_iter()
            .filter(|st| st.unlocked)
            .map(|st| st.name)
            .collect(),
    };
    if targets.is_empty() {
        print(io, "gage: no vaults are unlocked in this session.");
        return Ok(());
    }
    session.lock(name).map_err(commands::session_failure)?;
    for name in targets {
        print(
            io,
            &format!("{name} locked. run `use {name}` to unlock again."),
        );
    }
    Ok(())
}

/// `status`/`whoami`: every vault touched this session, with the idle
/// timeout already applied.
fn session_status(session: &mut Session, io: &mut Io, args: &[String]) -> Result<(), CliError> {
    if !args.is_empty() {
        return Err(CliError::new(ExitCode::Usage, "usage: status"));
    }
    let states = session.status();
    if states.is_empty() {
        print(
            io,
            "gage: no vaults have been unlocked in this session yet.",
        );
        return Ok(());
    }
    for st in states {
        let marker = if st.current { "* " } else { "  " };
        let state = if st.unlocked { "unlocked" } else { "locked" };
        let mut line = format!("{marker}{:<12} {state:<8}", st.name);
        if let Some(idle) = st.idle {
            line.push_str(&format!(
                "  (last used {} ago)",
                crate::duration::format_rounded(idle)
            ));
        }
        print(io, line.trim_end());
    }
    Ok(())
}

/// In-session `help` and `help <command>`, from the same registry and the
/// same clap tree as the one-shot surfaces (ADR-0031).
fn session_help(reg: &Registry, io: &mut Io, topic: &[String]) -> Result<(), CliError> {
    if topic.is_empty() {
        print(io, &cli::render_session_help(reg));
        return Ok(());
    }
    let root = cli::build(reg, &io.build);
    let path = cli::reached_path(&root, topic);
    if path.len() != topic.len() {
        return Err(CliError::new(
            ExitCode::Usage,
            format!(
                "unknown help topic {:?}; type `help` for what's available in a session",
                topic.join(" ")
            ),
        ));
    }
    print(io, &cli::command_help(&root, reg, &path));
    Ok(())
}

fn print(io: &mut Io, text: &str) {
    let _ = io.stdout.write_all(text.as_bytes());
    if !text.ends_with('\n') {
        let _ = io.stdout.write_all(b"\n");
    }
}

/// Splits a typed line into arguments on whitespace, with single and
/// double quotes grouping, so `show "AWS root account"` is two arguments.
///
/// There is no backslash escaping, on purpose: a session is where someone
/// types a Windows path (`insert C:\keys\prod`), and an escape rule would
/// eat its separators on the one platform that has to behave the same.
/// Quotes are the only grouping, which is also Windows' own rule. An
/// unbalanced quote is a usage error.
pub fn split_line(line: &str) -> Result<Vec<String>, CliError> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_arg = false;
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => current.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                in_arg = true;
            }
            None if c.is_whitespace() => {
                if in_arg {
                    args.push(std::mem::take(&mut current));
                    in_arg = false;
                }
            }
            None => {
                current.push(c);
                in_arg = true;
            }
        }
    }
    if let Some(q) = quote {
        return Err(CliError::new(
            ExitCode::Usage,
            format!("unbalanced {q} in command line"),
        ));
    }
    if in_arg {
        args.push(current);
    }
    Ok(args)
}
