//! The one-shot commands: each one's flags, its long help, and the thin
//! layer that turns parsed arguments into library calls and the result
//! into output. What a command *does* lives in the `gage` library.

use std::path::PathBuf;

use crate::exitcode::ExitCode;
use clap::{Arg, ArgMatches};
use gage::Prompter;
use gage::Session;
use gage::Vault;
use gage::session::OpenError;
use gage::storage::config::{self, Global};
use gage::storage::paths;
use gage::storage::xdgpaths::Dirs;

use crate::Io;
use crate::error::CliError;
use crate::registry::{Command, RunCommand};

mod approve;
mod auth;
mod catfmt;
mod clone;
mod enroll;
mod entry;
mod git;
mod identity;
mod init;
mod log;
mod movecopy;
mod recipient;
mod recovery;
mod recoverykey;
mod secretout;
mod sync;
mod vault;

#[cfg(test)]
pub(crate) use catfmt::{align_cat_output, pad_key_lines};
#[cfg(test)]
pub(crate) use enroll::human_duration;
#[cfg(test)]
pub(crate) use entry::{
    parse_field_flags, render_ls, render_ls_table, trim_one_trailing_newline, with_unlocked,
};
pub(crate) use entry::{session_failure, short_id};
#[cfg(test)]
pub(crate) use init::resolve_device_name;
#[cfg(test)]
pub(crate) use recipient::short_hash;
#[cfg(test)]
pub(crate) use recovery::LocalIdentityExists;
#[cfg(test)]
pub(crate) use recoverykey::{
    MAX_RECOVERY_CONFIRM_ATTEMPTS, RECOVERY_CONFIRM_CHARS, check_recovery_key_out_path,
    path_is_inside,
};
#[cfg(test)]
pub(crate) use sync::{report_conflicts, sync_line};

/// The flags of `command`, beyond its positionals. Session verbs are not
/// parsed by clap with flags of their own.
pub fn flags(command: Command) -> Vec<Arg> {
    use RunCommand::*;
    let Command::Run(run) = command else {
        return Vec::new();
    };
    // The commands that take only `--use` are listed, not defaulted, so a
    // new command has to decide.
    let use_only = || vec![entry::use_flag()];
    match run {
        Init => init::flags(),
        Clone => clone::flags(),
        Sync | Pull | Push => sync::flags(),
        Insert => entry::insert_flags(),
        Show => entry::show_flags(),
        Rename => entry::rename_flags(),
        Generate => entry::generate_flags(),
        Ls => entry::ls_flags(),
        Cat | Edit | Rm | Search | Reindex => use_only(),
        History => log::history_flags(),
        Log => use_only(),
        Mv | Cp => movecopy::flags(),
        AuthLogin | AuthStatus | AuthLogout => auth::flags(),
        IdentityAdd => identity::add_flags(),
        IdentityList => use_only(),
        IdentityEnroll => enroll::flags(),
        RecipientAdd => recipient::add_flags(),
        RecipientRemove => recipient::remove_flags(),
        RecipientVerify => recipient::verify_flags(),
        RecipientList => use_only(),
        RecipientApprove => approve::approve_flags(),
        RecipientPending | RecipientDeny => use_only(),
        RecoveryEnroll => recovery::enroll_flags(),
        RecoveryRotate => recovery::rotate_flags(),
        RecoveryVerify => use_only(),
        VaultList | VaultInfo | VaultRemove | VaultSetDefault | GitSetRemote => Vec::new(),
    }
}

/// The description `command`'s own help shows in place of its one-line
/// short, when it has more to say.
pub fn long_about(command: Command) -> Option<&'static str> {
    use RunCommand::*;
    let Command::Run(run) = command else {
        return None;
    };
    match run {
        Init => Some(init::LONG_ABOUT),
        Clone => Some(clone::LONG_ABOUT),
        Pull => Some(sync::PULL_LONG_ABOUT),
        Sync => Some(sync::SYNC_LONG_ABOUT),
        Ls => Some(entry::LS_LONG_ABOUT),
        Log => Some(log::LOG_LONG_ABOUT),
        History => Some(log::HISTORY_LONG_ABOUT),
        AuthLogin => Some(auth::LOGIN_LONG_ABOUT),
        IdentityAdd => Some(identity::ADD_LONG_ABOUT),
        IdentityEnroll => Some(enroll::LONG_ABOUT),
        RecipientAdd => Some(recipient::ADD_LONG_ABOUT),
        RecipientRemove => Some(recipient::REMOVE_LONG_ABOUT),
        RecipientVerify => Some(recipient::VERIFY_LONG_ABOUT),
        RecipientPending => Some(approve::PENDING_LONG_ABOUT),
        RecipientApprove => Some(approve::APPROVE_LONG_ABOUT),
        RecipientDeny => Some(approve::DENY_LONG_ABOUT),
        RecoveryEnroll => Some(recovery::ENROLL_LONG_ABOUT),
        RecoveryRotate => Some(recovery::ROTATE_LONG_ABOUT),
        RecoveryVerify => Some(recovery::VERIFY_LONG_ABOUT),
        Push | Insert | Show | Cat | Edit | Rename | Generate | Rm | Mv | Cp | Search | Reindex
        | VaultList | VaultInfo | VaultRemove | VaultSetDefault | GitSetRemote | AuthStatus
        | AuthLogout | IdentityList | RecipientList => None,
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

/// Runs `command`. Session verbs never get here: the REPL handles them
/// before parsing, and a one-shot run refuses them first.
pub fn run(
    command: RunCommand,
    matches: &ArgMatches,
    io: &mut Io,
    keys: &mut Keys,
) -> Result<(), CliError> {
    use RunCommand::*;
    match command {
        Init => init::run(matches, io, keys.prompter()),
        Clone => clone::run(matches, io, keys.prompter()),
        Sync => sync::sync(matches, io, keys),
        Pull => sync::pull(matches, io, keys),
        Push => sync::push(matches, io, keys),
        Insert => entry::insert(matches, io, keys),
        Show => entry::show(matches, io, keys),
        Cat => entry::cat(matches, io, keys),
        Edit => entry::edit(matches, io, keys),
        Rename => entry::rename(matches, io, keys),
        Generate => entry::generate(matches, io, keys),
        Rm => entry::rm(matches, io, keys),
        Mv => movecopy::mv(matches, io, keys),
        Cp => movecopy::cp(matches, io, keys),
        Ls => entry::ls(matches, io, keys),
        Search => entry::search(matches, io, keys),
        Reindex => entry::reindex(matches, io, keys),
        Log => log::log(matches, io, keys),
        History => log::history(matches, io, keys),
        VaultList => vault::list(io),
        VaultInfo => vault::info(matches, io),
        VaultRemove => vault::remove(matches, io, keys.prompter()),
        VaultSetDefault => vault::set_default(matches, io),
        GitSetRemote => git::set_remote(matches, io),
        AuthLogin => auth::login(matches, io, keys),
        AuthStatus => auth::status(matches, io),
        AuthLogout => auth::logout(matches, io),
        IdentityAdd => identity::add(matches, io, keys),
        IdentityEnroll => enroll::run(matches, io, keys),
        IdentityList => identity::list(matches, io, keys),
        RecipientAdd => recipient::add(matches, io, keys),
        RecipientRemove => recipient::remove(matches, io, keys),
        RecipientList => recipient::list(matches, io, keys),
        RecipientVerify => recipient::verify(matches, io, keys),
        RecipientPending => approve::pending(matches, io, keys),
        RecipientApprove => approve::approve(matches, io, keys),
        RecipientDeny => approve::deny(matches, io, keys),
        RecoveryEnroll => recovery::enroll(matches, io, keys.prompter()),
        RecoveryRotate => recovery::rotate(matches, io, keys),
        RecoveryVerify => recovery::verify(matches, io, keys.prompter()),
    }
}

/// The process's directory roots, resolved only when a command needs them,
/// so help and usage errors never fail for want of a home directory.
fn dirs(io: &Io) -> Result<Dirs, CliError> {
    Ok((io.dirs)()?)
}

/// The process's working directory, which a relative path argument is
/// relative to. Asked for only by a command that needs it, and passed on
/// as a parameter, so tests give their own rather than move the process's.
fn cwd() -> Result<PathBuf, CliError> {
    std::env::current_dir()
        .map_err(|e| CliError::internal(anyhow::anyhow!("reading the working directory: {e}")))
}

/// A registered vault by name: the lookup a session's opener runs, the
/// same one one-shot `--use` resolves through.
pub(crate) fn open_registered(dirs: &Dirs, name: &str) -> Result<Vault, OpenError> {
    let global = config::load_or_default(&paths::global_config_file(dirs))?;
    let entry = global.entry(name)?;
    Ok(Vault::from_registration(dirs, name, entry)?)
}

/// The global config and where it lives.
fn load_global(io: &Io) -> Result<(PathBuf, Global), CliError> {
    let path = paths::global_config_file(&dirs(io)?);
    let global = config::load_or_default(&path)?;
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
fn fail(code: ExitCode, msg: impl std::fmt::Display) -> CliError {
    CliError::new(code, msg)
}
