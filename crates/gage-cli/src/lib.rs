//! The `gage` command-line frontend: a thin layer that parses arguments,
//! calls into the `gage` library, and renders the result as output, a
//! prompt, or an exit code. Everything that decides what gage does lives in
//! the library; see the design doc's "Library architecture".
//!
//! [`run`] is the whole CLI with its process seams injected, so tests drive
//! it in-process. `main` is the only code that touches the real standard
//! streams or exits.

use std::io::{Read, Write};

use gage::exitcode::Code;
use gage::prompter::Prompter;
use gage::xdgpaths::{self, Dirs};

// build.rs include!s it too; the crate itself only needs it for the test
// that checks the Makefile exports the same names.
#[cfg(test)]
mod build_env;
pub mod cli;
mod commands;
pub mod coredump;
mod editor;
pub mod error;
pub mod prompter;
pub mod registry;
pub mod version;

use cli::Outcome;
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
        Outcome::Session => Err(CliError::new(
            Code::Internal,
            "session mode is not implemented yet",
        )),
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
            commands::run(name, &matches, io, prompter).unwrap_or_else(|| {
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
mod entry_tests;
#[cfg(test)]
mod init_identity_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod query_tests;
#[cfg(test)]
mod tests;
