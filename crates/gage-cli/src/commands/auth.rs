//! `gage auth login|status|logout`: per-host tokens for HTTPS remotes
//! (ADR-0020). Under the git group because a token for a git host
//! means nothing to a different backing store. See "Git-specific
//! commands" and "Authentication" in the design doc.
//!
//! gage never issues a token: no OAuth device flow and no client id in the
//! binary. On GitHub a device-flow grant is scope-based and reaches every
//! private repository you own, which is broader than the fine-grained,
//! single-repository token `login` asks for.

use crate::exitcode::ExitCode;
use clap::{Arg, ArgMatches};
use gage::{gitrepo, remoteauth};

use super::{Keys, dirs, fail, load_global, out};
use crate::Io;
use crate::error::CliError;

pub const LOGIN_LONG_ABOUT: &str = "Store a token for a git host.

gage never issues a token for you: create one on the host and paste it here.
Scope it as narrowly as the host allows — on GitHub, a fine-grained personal
access token limited to the vault's own repository, not a classic repo-scoped
one that reaches every repository you own.";

/// The `--host` flag all three verbs share.
pub fn flags() -> Vec<Arg> {
    vec![
        Arg::new("host").long("host").value_name("host").help(
            "git host the token belongs to (default: the host of the current vault's origin)",
        ),
    ]
}

fn host_flag(matches: &ArgMatches) -> Option<&str> {
    matches.get_one::<String>("host").map(String::as_str)
}

/// `--host` if given, else the host of the current vault's origin. The
/// repository is the authority on what origin is: the global config's
/// record of it can lag a `git remote set-url` run outside gage.
fn resolve_host(explicit: Option<&str>, io: &Io) -> Result<String, CliError> {
    if let Some(host) = explicit {
        return Ok(host.to_string());
    }
    let no_vault = || {
        fail(
            ExitCode::Usage,
            "no --host given and no current vault to take one from",
        )
    };
    let (_, global) = load_global(io).map_err(|_| no_vault())?;
    let name = global.name_or_current(None).map_err(|_| no_vault())?;
    let entry = global.entry(name).map_err(|_| no_vault())?;

    let origin = match &entry.git.origin {
        Some(origin) => Some(origin.clone()),
        None => gitrepo::remote_url(&entry.path)?,
    };
    let Some(origin) = origin.filter(|o| !o.is_empty()) else {
        return Err(fail(
            ExitCode::Usage,
            format!(
                "vault {name:?} has no remote, so there is no host to authenticate to; pass --host"
            ),
        ));
    };
    let host = remoteauth::host(&origin)?;
    if host.is_empty() {
        return Err(fail(
            ExitCode::Usage,
            format!(
                "vault {name:?}'s remote ({origin}) is a local path, which needs no token; pass --host to name one"
            ),
        ));
    }
    Ok(host)
}

/// Prompts for the token, never takes it as an argument: one on a command
/// line would land in shell history and the session's history file. The
/// prompter masks it the way it masks a secret value.
pub fn login(matches: &ArgMatches, io: &mut Io, keys: &Keys) -> Result<(), CliError> {
    let host = resolve_host(host_flag(matches), io)?;
    let token = keys
        .prompter()
        .value(&format!("token for {host}"))
        .map_err(|e| fail(ExitCode::LockedOrAuth, e))?;
    remoteauth::save(&dirs(io)?, &host, &token)?;
    out(io, &[format!("gage: stored a token for {host}")]);
    Ok(())
}

/// Which hosts have a token. No network: whether a token still works is
/// answered by the next fetch or push, in terms of what the human wanted.
pub fn status(matches: &ArgMatches, io: &mut Io) -> Result<(), CliError> {
    if let Some(host) = host_flag(matches) {
        return report(host, io);
    }
    let hosts = remoteauth::hosts(&dirs(io)?)?;
    if hosts.is_empty() {
        // The current vault's host makes the no-token case actionable
        // instead of blank.
        if let Ok(host) = resolve_host(None, io) {
            return report(&host, io);
        }
        out(io, &["gage: no tokens are stored on this machine".into()]);
        return Ok(());
    }
    let lines: Vec<String> = hosts.iter().map(|h| format!("{h}  configured")).collect();
    out(io, &lines);
    Ok(())
}

/// One host's state; never the token, only whether there is one.
fn report(host: &str, io: &mut Io) -> Result<(), CliError> {
    let line = match remoteauth::load(&dirs(io)?, host) {
        Ok(_) => format!("{host}  configured"),
        Err(_) => format!("{host}  no token (run `gage auth login --host {host}`)"),
    };
    out(io, &[line]);
    Ok(())
}

pub fn logout(matches: &ArgMatches, io: &mut Io) -> Result<(), CliError> {
    let host = resolve_host(host_flag(matches), io)?;
    remoteauth::remove(&dirs(io)?, &host)?;
    out(io, &[format!("gage: forgot the token for {host}")]);
    Ok(())
}
