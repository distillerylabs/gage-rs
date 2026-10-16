//! `mv` and `cp`: share an entry into another vault. What they do (decrypt
//! here, encrypt to the other vault's recipients, the lock order and the
//! write order) lives in the library; this is flags, the checks that must
//! fail before any unlock, and the session's index bookkeeping.
//!
//! The destination is never unlocked, in either mode. Everything knowable
//! from the registry fails before the source asks for a passphrase: a
//! destination that isn't registered, and one that is the source.

use clap::{Arg, ArgMatches};
use gage::exitcode::Code;
use gage::identity::Identity;
use gage::movecopy::{MoveError, MoveResult};
use gage::prompter::Prompter;
use gage::vault::Vault;

use super::entry::{open_vault, resolve_failure, use_flag, use_name, with_unlocked};
use super::{Keys, arg, dirs, fail, open_registered, out, session_failure};
use crate::Io;
use crate::error::CliError;

pub fn flags() -> Vec<Arg> {
    vec![
        Arg::new("to-vault")
            .long("to-vault")
            .value_name("string")
            .required(true)
            .help("destination vault to share the entry into (required)"),
        use_flag(),
    ]
}

pub fn mv(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    run(matches, io, keys, true)
}

pub fn cp(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    run(matches, io, keys, false)
}

fn run(matches: &ArgMatches, io: &mut Io, keys: &mut Keys, remove: bool) -> Result<(), CliError> {
    let to = arg(matches, "to-vault");
    let query = arg(matches, "query");
    let prompter = keys.prompter();
    let moved = match keys {
        Keys::OneShot(_) => {
            let source = open_vault(matches, io)?;
            let dest = open_registered(&dirs(io)?, to)?;
            if source.name == dest.name || source.id == dest.id {
                return Err(same_vault(to));
            }
            with_unlocked(&source, prompter, |identity| {
                transfer(&source, query, &dest, identity, prompter, remove)
            })?
        }
        Keys::Session(session) => {
            if use_name(matches).or(session.current()) == Some(to) {
                return Err(same_vault(to));
            }
            // Registers the destination without unlocking it, so an
            // unknown one fails before the source's passphrase is asked.
            session
                .vault_without_unlocking(Some(to))
                .map_err(session_failure)?;
            // As `rm`: the picker runs here, and the library resolves the
            // chosen id again under the locks.
            let (id, _) = session
                .resolve(use_name(matches), query)
                .map_err(session_failure)?;
            let (source, identity, dest) = session
                .vault_and_destination(use_name(matches), to)
                .map_err(session_failure)?;
            let moved = transfer(source, &id.to_string(), dest, identity, prompter, remove)?;
            if remove {
                session.forget_entry(use_name(matches), moved.source_id);
            }
            session.note_entry(Some(to), moved.dest_id, &moved.entry);
            moved
        }
    };
    let verb = if remove { "moved" } else { "copied" };
    out(
        io,
        &[format!(
            "gage: {verb} {:?} to {to} ({})",
            moved.entry.title, moved.dest_id
        )],
    );
    Ok(())
}

fn transfer(
    source: &Vault,
    query: &str,
    dest: &Vault,
    identity: &Identity,
    prompter: &dyn Prompter,
    remove: bool,
) -> Result<MoveResult, CliError> {
    let result = if remove {
        source.move_to(query, dest, identity, prompter)
    } else {
        source.copy_to(query, dest, identity, prompter)
    };
    result.map_err(|err| match err {
        MoveError::Resolve(e) => resolve_failure(e),
        other => other.into(),
    })
}

fn same_vault(to: &str) -> CliError {
    fail(
        Code::Usage,
        format!("{to:?} is the source vault; --to-vault must name a different vault"),
    )
}
