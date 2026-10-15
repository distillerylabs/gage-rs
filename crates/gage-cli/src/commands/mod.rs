//! The one-shot commands: each one's flags, its long help, and the thin
//! layer that turns parsed arguments into library calls and the result
//! into output. What a command *does* lives in the `gage` library.

use std::path::PathBuf;

use clap::{Arg, ArgMatches};
use gage::config::{self, Global};
use gage::exitcode::Code;
use gage::paths;
use gage::prompter::Prompter;
use gage::session::{OpenError, Session};
use gage::vault::Vault;
use gage::xdgpaths::Dirs;

use crate::Io;
use crate::error::CliError;

mod auth;
mod entry;
mod git;
mod init;
mod vault;

#[cfg(test)]
pub(crate) use entry::{parse_field_flags, render_ls, trim_one_trailing_newline, with_unlocked};
pub(crate) use entry::{session_failure, short_id};
#[cfg(test)]
pub(crate) use init::resolve_device_name;

/// The flags of the command registered as `name`, beyond its positionals.
pub fn flags(name: &str) -> Vec<Arg> {
    match name {
        "init" => init::flags(),
        "insert" | "show" | "cat" | "edit" | "rename" | "generate" | "rm" | "ls" | "search"
        | "reindex" => entry::flags(name),
        "auth login" | "auth status" | "auth logout" => auth::flags(),
        _ => Vec::new(),
    }
}

/// The description `name`'s own help shows in place of its one-line
/// short, when it has more to say.
pub fn long_about(name: &str) -> Option<&'static str> {
    match name {
        "init" => Some(init::LONG_ABOUT),
        "ls" => Some(entry::LS_LONG_ABOUT),
        "auth login" => Some(auth::LOGIN_LONG_ABOUT),
        _ => None,
    }
}

/// Where a command's identity comes from, and the one thing that differs
/// between the two invocation modes: a fresh unlock dropped when the
/// command ends, or the key a running session already holds, borrowed.
/// Either way the handler is given a `&Vault` and a `&Identity` and calls
/// the same `Vault` methods.
pub enum Keys<'k, 'a> {
    OneShot(&'a dyn Prompter),
    Session(&'k mut Session<'a>),
}

impl<'a> Keys<'_, 'a> {
    /// The prompter for a question of the command's own (an insert's
    /// value).
    pub fn prompter(&self) -> &'a dyn Prompter {
        match self {
            Keys::OneShot(p) => *p,
            Keys::Session(s) => s.prompter(),
        }
    }

    pub fn in_session(&self) -> bool {
        matches!(self, Keys::Session(_))
    }
}

/// Runs the command registered as `name`, or `None` if there is no such
/// implemented command.
pub fn run(
    name: &str,
    matches: &ArgMatches,
    io: &mut Io,
    keys: &mut Keys,
) -> Option<Result<(), CliError>> {
    Some(match name {
        "init" => init::run(matches, io, keys.prompter()),
        "insert" => entry::insert(matches, io, keys),
        "show" => entry::show(matches, io, keys),
        "cat" => entry::cat(matches, io, keys),
        "edit" => entry::edit(matches, io, keys),
        "rename" => entry::rename(matches, io, keys),
        "generate" => entry::generate(matches, io, keys),
        "rm" => entry::rm(matches, io, keys),
        "ls" => entry::ls(matches, io, keys),
        "search" => entry::search(matches, io, keys),
        "reindex" => entry::reindex(matches, io, keys),
        "vault list" => vault::list(io),
        "vault info" => vault::info(matches, io),
        "vault remove" => vault::remove(matches, io),
        "vault set-default" => vault::set_default(matches, io),
        "git set-remote" => git::set_remote(matches, io),
        "auth login" => auth::login(matches, io, keys),
        "auth status" => auth::status(matches, io),
        "auth logout" => auth::logout(matches, io),
        _ => return None,
    })
}

/// The process's directory roots, resolved only when a command needs them,
/// so help and usage errors never fail for want of a home directory.
fn dirs(io: &Io) -> Result<Dirs, CliError> {
    Ok((io.dirs)()?)
}

/// A registered vault by name: the lookup a session's opener runs, the
/// same one one-shot `--use` resolves through.
pub(crate) fn open_registered(dirs: &Dirs, name: &str) -> Result<Vault, OpenError> {
    let global = config::load(&paths::global_config_file(dirs)).map_err(OpenError::new)?;
    let entry = global.entry(name).map_err(OpenError::new)?;
    Vault::from_registration(dirs, name, entry).map_err(OpenError::new)
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
