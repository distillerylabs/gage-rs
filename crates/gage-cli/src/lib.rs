//! The `gage` command-line frontend: a thin layer that parses arguments,
//! calls into the `gage` library, and renders the result as output, a
//! prompt, or an exit code. Everything that decides what gage does lives in
//! the library; see the design doc's "Library architecture".
//!
//! [`run`] is the whole CLI with its process seams injected, so tests drive
//! it in-process. `main` is the only code that touches the real standard
//! streams or exits.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use gage::exitcode::Code;
use gage::prompter::Prompter;
use gage::xdgpaths::{self, Dirs};

// build.rs include!s it too; the crate itself only needs it for the test
// that checks the Makefile exports the same names.
mod assumeyes;
#[cfg(test)]
mod build_env;
pub mod cli;
pub mod clipboard;
mod commands;
mod complete;
pub mod coredump;
mod editor;
pub mod error;
mod history;
mod lineread;
pub mod prompter;
mod qr;
pub mod registry;
mod repl;
pub mod scriptprompt;
mod shell;
pub mod version;

use cli::Outcome;
use clipboard::ClipboardKeeper;
use commands::Keys;
use error::{CliError, prefix_once};
use registry::{Availability, Registry};
pub use version::BuildInfo;

/// The process seams one invocation runs against.
pub struct Io<'a> {
    pub stdin: &'a mut dyn Read,
    pub stdout: &'a mut dyn Write,
    pub stderr: &'a mut dyn Write,
    /// Whether stdin is a terminal: bare `gage` starts a session on one
    /// and prints help otherwise.
    pub stdin_is_terminal: bool,
    /// Whether stderr is a terminal. Showing a recovery key needs both
    /// this and `stdin_is_terminal`: the key goes to stderr and its
    /// re-type comes from stdin, and `gage init foo 2>build.log` has only
    /// the second. Neither flag stands in for the other
    /// (Q-RECOVERY-TERMINAL).
    pub stderr_is_terminal: bool,
    /// Whether a session reads its command lines through the terminal
    /// line editor. `main` sets it when stdin is a terminal; in-process
    /// tests leave it off, so the session reads `stdin` instead.
    pub line_editor: bool,
    /// What `--version` reports. `main` passes [`BuildInfo::stamped`].
    pub build: BuildInfo,
    /// gage's config, data and state roots. `main` passes
    /// [`Dirs::from_process`]; tests point them at a temp directory. A
    /// function, so a command that needs none (help) never fails for want
    /// of a home directory.
    pub dirs: &'a dyn Fn() -> Result<Dirs, xdgpaths::Error>,
    /// This machine's hostname: `init`'s default device name.
    pub hostname: &'a dyn Fn() -> std::io::Result<String>,
    /// `$EDITOR`, for `edit` and `insert -e`. `main` reads the variable;
    /// in-process tests, which share one process environment, set it here.
    pub editor: Option<String>,
    /// `$GAGE_PASSPHRASE`, for a `--script`/`--stdin` run's unlocks.
    /// `main` reads the variable once, as `Some` even when empty (set-but-
    /// empty is still set); in-process tests, which share one process
    /// environment, set it here. Nothing in the library learns it exists.
    pub passphrase_env: Option<zeroize::Zeroizing<String>>,
    /// The clock a session's idle timeout reads. `main` passes
    /// `Instant::now`; tests step it instead of waiting.
    pub now: &'a dyn Fn() -> Instant,
    /// Where `-c` copies to, and what clears it. `main` passes
    /// [`ClipboardKeeper::system`]; tests pass one over a fake clipboard
    /// and timer. One per run, so a session's copies share one keeper and
    /// its exit clears whatever is pending.
    pub clipboard: &'a ClipboardKeeper,
    /// A one-shot `-c`'s wait for its timeout. `main` passes
    /// [`clipboard::wait_for_clipboard`]; tests record the call instead of
    /// waiting, and never install a signal handler.
    pub clipboard_wait: &'a dyn Fn(Duration),
    /// The zone local times render in: `recipient pending`'s expiries and
    /// the approve screen. `main` passes the system zone; in-process
    /// tests, which share one process environment, set it here rather than
    /// `TZ` (Q-PENDING-TZ).
    pub time_zone: jiff::tz::TimeZone,
}

/// Runs one invocation (`args` without the program name) and returns its
/// exit code.
pub fn run(args: &[String], io: &mut Io, prompter: &dyn Prompter) -> Code {
    run_with(&registry::registry(), args, io, prompter)
}

/// [`run`] against a given registry, so tests can exercise help rendering
/// for commands that don't exist yet.
pub(crate) fn run_with(
    reg: &Registry,
    args: &[String],
    io: &mut Io,
    prompter: &dyn Prompter,
) -> Code {
    match dispatch(reg, args, io, prompter) {
        Ok(()) => Code::Success,
        Err(err) => {
            // If reporting the error fails there is nowhere left to report
            // that, and the exit code still carries the outcome.
            let _ = writeln!(io.stderr, "{}", prefix_once(&err.to_string()));
            err.code()
        }
    }
}

fn dispatch(
    reg: &Registry,
    args: &[String],
    io: &mut Io,
    prompter: &dyn Prompter,
) -> Result<(), CliError> {
    match cli::parse(reg, &io.build, args, io.stdin_is_terminal)? {
        Outcome::Print(text) => print(io, &text),
        Outcome::Session => repl::run_session(reg, io, prompter),
        Outcome::Script(run) => {
            // `--yes` innermost, so the script's passphrase policy sits
            // over it and every question it doesn't answer reaches it.
            let assume_yes = assumeyes::AssumeYes(prompter);
            let base: &dyn Prompter = if run.assume_yes {
                &assume_yes
            } else {
                prompter
            };
            // A human can be asked only when there is a terminal and the
            // commands aren't arriving on it: `--script FILE` leaves stdin
            // free to answer on, `--stdin` has spent it.
            let can_prompt = run.source != cli::ScriptSource::Stdin && io.stdin_is_terminal;
            // With nobody to ask, an ambiguous query fails with its list
            // rather than reading a pick off the command stream.
            let no_choose = scriptprompt::NoChoosePrompter(base);
            let base: &dyn Prompter = if can_prompt { base } else { &no_choose };
            let passphrase_env = io.passphrase_env.take();
            let prompter = scriptprompt::for_script(base, passphrase_env.as_ref(), can_prompt);
            repl::run_script(reg, io, &*prompter, &run.source)
        }
        Outcome::Command("help", matches) => {
            let topic: Vec<String> = matches
                .get_many::<String>("command")
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            let text = cli::help_topic(reg, &io.build, &topic)?;
            print(io, &text)
        }
        Outcome::Command(name, matches) => {
            let info = reg
                .find(name)
                .expect("parse only returns registered commands");
            if info.availability == Availability::SessionOnly {
                return Err(CliError::new(
                    Code::Usage,
                    format!(
                        "{name} is only available in a session; run `gage` with no arguments to start one"
                    ),
                ));
            }
            // Wrapped here, once, rather than at each point a handler hands
            // a prompter to the library: the same decision written out a
            // dozen times would have a dozen chances to be forgotten.
            let assume_yes = assumeyes::AssumeYes(prompter);
            let prompter: &dyn Prompter = if assumeyes::given(&matches) {
                &assume_yes
            } else {
                prompter
            };
            commands::run(name, &matches, io, &mut Keys::OneShot(prompter)).unwrap_or_else(|| {
                Err(CliError::new(
                    Code::Internal,
                    format!("{name} is not implemented yet"),
                ))
            })
        }
    }
}

/// Writes rendered text to stdout. A failed write (`gage --help | head`
/// closing the pipe) is dropped: the process is about to exit and there is
/// nowhere better to report it.
fn print(io: &mut Io, text: &str) -> Result<(), CliError> {
    let _ = io.stdout.write_all(text.as_bytes());
    if !text.ends_with('\n') {
        let _ = io.stdout.write_all(b"\n");
    }
    Ok(())
}

#[cfg(test)]
mod approve_cli_tests;
#[cfg(test)]
mod approve_scope_cli_tests;
#[cfg(test)]
mod auth_tests;
#[cfg(test)]
mod clip_tests;
#[cfg(test)]
mod clone_enroll_tests;
#[cfg(test)]
mod command_reference_tests;
#[cfg(test)]
mod complete_tests;
#[cfg(test)]
mod enroll_cli_tests;
#[cfg(test)]
mod entry_tests;
#[cfg(test)]
mod fixture_support;
#[cfg(test)]
mod help_surface_tests;
#[cfg(test)]
mod identity_cli_tests;
#[cfg(test)]
mod init_identity_tests;
#[cfg(test)]
mod init_recoverykey_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod log_cli_tests;
#[cfg(test)]
mod movecopy_cli_tests;
#[cfg(test)]
mod qr_tests;
#[cfg(test)]
mod query_tests;
#[cfg(test)]
mod recipient_cli_tests;
#[cfg(test)]
mod recovery_cli_tests;
#[cfg(test)]
mod repl_tests;
#[cfg(test)]
mod script_tests;
#[cfg(test)]
mod search_tests;
#[cfg(test)]
mod sync_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod trustcache_cli_tests;
#[cfg(test)]
mod vault_remove_tests;
