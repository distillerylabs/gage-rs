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
use std::rc::Rc;

use gage::config;
use gage::exitcode::Code;
use gage::paths;
use gage::prompter::Prompter;
use gage::session::{Session, SessionConfig};
use gage::xdgpaths::Dirs;

use crate::Io;
use crate::cli::{self, Outcome};
use crate::commands::{self, Keys};
use crate::complete::{SessionHelper, resolve_session_command_name};
use crate::error::{CliError, prefix_once};
use crate::history::History;
use crate::lineread::{LineReader, PlainLineReader, ReadOutcome, RustylineReader};
use crate::registry::Registry;
use crate::shell::{self, PromptState, ShellSettings};

/// Runs a session until `exit` or end of input. Every key it held is
/// dropped on the way out, whichever way that is.
pub fn run_session(reg: &Registry, io: &mut Io, prompter: &dyn Prompter) -> Result<(), CliError> {
    let dirs = (io.dirs)()?;
    let global = config::load(&paths::global_config_file(&dirs))?;
    let settings = shell::resolve(&global.shell, &dirs, &std::env::home_dir)?;

    let now = io.now;
    let open_dirs = dirs.clone();
    // Shared with the line editor's completer, which reads the session
    // while a line is being typed; the loop holds it mutably only while a
    // command runs, so the two never overlap.
    let session = Rc::new(RefCell::new(Session::new(SessionConfig {
        // The same lookup one-shot `--use` resolves through, so both
        // modes agree on what a vault name means.
        open: Box::new(move |name| commands::open_registered(&open_dirs, name)),
        prompter,
        idle_timeout: settings.idle_timeout,
        // A session starts pointed where a one-shot command would, so
        // `show foo` works before any `use`. Naming it doesn't unlock it.
        current: Some(global.current),
        now: Some(Box::new(now)),
    })));

    let mut history = History::open(Some(&settings.history_file)).unwrap_or_else(|e| {
        // Losing recall is no reason to refuse a session: say so once.
        let msg = e.to_string();
        let _ = writeln!(
            io.stderr,
            "gage: continuing without command history: {}",
            msg.trim_start_matches("gage: ")
        );
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
            CliError::new(
                Code::Internal,
                format!("gage: starting the line editor: {e}"),
            )
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
    /// input, then drops every key the session holds, on every way out,
    /// an error included.
    pub fn run(&mut self, io: &mut Io) -> Result<(), CliError> {
        let result = self.run_lines(io);
        self.session.borrow_mut().close();
        result
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
                        Code::Internal,
                        format!("gage: reading a command: {e}"),
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
    let _ = writeln!(io.stderr, "{}", prefix_once(&err.to_string()));
}

/// Runs one typed line. `Ok(true)` is `exit`/`quit`.
pub fn exec_line(
    reg: &Registry,
    session: &mut Session,
    io: &mut Io,
    line: &str,
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
        _ => run_command(reg, session, io, &args)?,
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
) -> Result<(), CliError> {
    // init (and later clone) create a vault rather than operating on one,
    // which leaves "does the new vault become current?" unanswered. Read
    // off the registry, so a later one-shot-only command is covered the
    // day it's registered.
    if let Some(info) = reg.find(&command_path(reg, args))
        && !info.availability.session_visible()
    {
        return Err(CliError::new(
            Code::Usage,
            format!(
                "gage: {0} is a one-shot command and isn't available in a session; run `gage {0}` from your shell instead",
                info.name
            ),
        ));
    }
    let root = cli::build(reg, &io.build);
    if cli::reached_path(&root, args).is_empty() {
        return Err(CliError::new(
            Code::Usage,
            format!(
                "gage: unknown command {:?}; type `help` for what's available in a session",
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
        Outcome::Command(name, matches) => {
            commands::run(name, &matches, io, &mut Keys::Session(session)).unwrap_or_else(|| {
                Err(CliError::new(
                    Code::Internal,
                    format!("{name} is not implemented yet"),
                ))
            })
        }
    }
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
        return Err(CliError::new(Code::Usage, "gage: usage: use <vault>"));
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
        _ => return Err(CliError::new(Code::Usage, "gage: usage: lock [vault]")),
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
        return Err(CliError::new(Code::Usage, "gage: usage: status"));
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
            line.push_str(&format!("  (last used {} ago)", go_duration(idle)));
        }
        print(io, line.trim_end());
    }
    Ok(())
}

/// A duration as Go prints one rounded to the second: `5s`, `2m5s`,
/// `1h0m0s`.
fn go_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs() + u64::from(d.subsec_millis() >= 500);
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    match (h, m) {
        (0, 0) => format!("{s}s"),
        (0, _) => format!("{m}m{s}s"),
        _ => format!("{h}h{m}m{s}s"),
    }
}

/// In-session `help` and `help <command>`, from the same registry and the
/// same clap tree as the one-shot surfaces (Q-HELP-SURFACES).
fn session_help(reg: &Registry, io: &mut Io, topic: &[String]) -> Result<(), CliError> {
    if topic.is_empty() {
        print(io, &cli::render_session_help(reg));
        return Ok(());
    }
    let root = cli::build(reg, &io.build);
    let path = cli::reached_path(&root, topic);
    if path.len() != topic.len() {
        return Err(CliError::new(
            Code::Usage,
            format!(
                "gage: unknown help topic {:?}; type `help` for what's available in a session",
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
            Code::Usage,
            format!("gage: unbalanced {q} in command line"),
        ));
    }
    if in_arg {
        args.push(current);
    }
    Ok(args)
}
