//! The one-shot commands: each one's flags, its long help, and the thin
//! layer that turns parsed arguments into library calls and the result
//! into output. What a command *does* lives in the `gage` library.

use std::path::PathBuf;

use clap::{Arg, ArgMatches};
use gage::config::{self, Global};
use gage::exitcode::Code;
use gage::paths;
use gage::prompter::Prompter;
use gage::xdgpaths::Dirs;

use crate::Io;
use crate::error::CliError;

mod entry;
mod git;
mod init;
mod vault;

#[cfg(test)]
pub(crate) use entry::{
    parse_field_flags, render_ls, short_id, trim_one_trailing_newline, with_unlocked,
};
#[cfg(test)]
pub(crate) use init::resolve_device_name;

/// The flags of the command registered as `name`, beyond its positionals.
pub fn flags(name: &str) -> Vec<Arg> {
    match name {
        "init" => init::flags(),
        "insert" | "show" | "cat" | "edit" | "rename" | "generate" | "rm" | "ls" => {
            entry::flags(name)
        }
        _ => Vec::new(),
    }
}

/// The description `name`'s own help shows in place of its one-line
/// short, when it has more to say.
pub fn long_about(name: &str) -> Option<&'static str> {
    match name {
        "init" => Some(init::LONG_ABOUT),
        "ls" => Some(entry::LS_LONG_ABOUT),
        _ => None,
    }
}

/// Runs the one-shot command registered as `name`, or `None` if there is
/// no such implemented command.
pub fn run(
    name: &str,
    matches: &ArgMatches,
    io: &mut Io,
    prompter: &dyn Prompter,
) -> Option<Result<(), CliError>> {
    Some(match name {
        "init" => init::run(matches, io, prompter),
        "insert" => entry::insert(matches, io, prompter),
        "show" => entry::show(matches, io, prompter),
        "cat" => entry::cat(matches, io, prompter),
        "edit" => entry::edit(matches, io, prompter),
        "rename" => entry::rename(matches, io, prompter),
        "generate" => entry::generate(matches, io, prompter),
        "rm" => entry::rm(matches, io, prompter),
        "ls" => entry::ls(matches, io, prompter),
        "vault list" => vault::list(io),
        "vault info" => vault::info(matches, io),
        "vault remove" => vault::remove(matches, io),
        "vault set-default" => vault::set_default(matches, io),
        "git set-remote" => git::set_remote(matches, io),
        _ => return None,
    })
}

/// The process's directory roots, resolved only when a command needs them,
/// so help and usage errors never fail for want of a home directory.
fn dirs(io: &Io) -> Result<Dirs, CliError> {
    Ok((io.dirs)()?)
}

/// The global config and where it lives.
fn load_global(io: &Io) -> Result<(PathBuf, Global), CliError> {
    let path = paths::global_config_file(&dirs(io)?);
    let global = config::load(&path)?;
    Ok((path, global))
}

/// A positional argument clap has already required.
fn arg<'a>(matches: &'a ArgMatches, id: &str) -> &'a str {
    matches
        .get_one::<String>(id)
        .map(String::as_str)
        .expect("clap enforces required positionals")
}

/// Writes lines to stdout. A failed write (a closed pipe) is dropped, as
/// `print` in lib.rs does: the outcome is already decided.
fn out(io: &mut Io, lines: &[String]) {
    for line in lines {
        let _ = writeln!(io.stdout, "{line}");
    }
}

/// An error a command constructs itself, rather than one a library call
/// returned.
fn fail(code: Code, msg: impl std::fmt::Display) -> CliError {
    CliError::new(code, msg)
}
