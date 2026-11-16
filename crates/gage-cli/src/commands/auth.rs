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
use gage::prompter::Warning;
use gage::sync::remotecheck::{self, Access};
use gage::sync::{gitrepo, remoteauth};
use gage::{ErrorKind, HasKind};

use super::{Keys, dirs, fail, load_global, out};
use crate::Io;
use crate::error::CliError;

pub const LOGIN_LONG_ABOUT: &str = "Store a token for a git host.

gage never issues a token for you: create one on the host and paste it here.
Scope it as narrowly as the host allows — on GitHub, a fine-grained personal
access token limited to the vault's own repository, not a classic repo-scoped
one that reaches every repository you own.

If the current vault's remote is on that host, the token is tested there
first, without changing anything: a token the host refuses is not stored, and
one that can read but not push is stored with a warning.";

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

/// `--host` if given, else the host of the current vault's origin.
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
    let (name, origin) = current_origin(io)?.ok_or_else(no_vault)?;
    let Some(origin) = origin else {
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
                "vault {name:?}'s remote ({}) is a local path, which needs no token; pass --host to name one",
                remoteauth::redacted(&origin)
            ),
        ));
    }
    Ok(host)
}

/// The current vault's name and origin, `None` with no current vault. The
/// repository is the authority on what origin is: the global config's
/// record of it can lag a `git remote set-url` run outside gage.
fn current_origin(io: &Io) -> Result<Option<(String, Option<String>)>, CliError> {
    let Ok((_, global)) = load_global(io) else {
        return Ok(None);
    };
    let Ok(name) = global.name_or_current(None) else {
        return Ok(None);
    };
    let Ok(entry) = global.entry(name) else {
        return Ok(None);
    };
    let origin = match &entry.git.origin {
        Some(origin) => Some(origin.clone()),
        None => gitrepo::remote_url(&entry.path)?,
    };
    Ok(Some((name.to_string(), origin.filter(|o| !o.is_empty()))))
}

/// Prompts for the token, never takes it as an argument: one on a command
/// line would land in shell history and the session's history file. The
/// prompter masks it the way it masks a secret value.
///
/// When the current vault's origin is on `host`, the token is tested there
/// first (ADR-0080): one the host refuses is not stored, so it can't
/// replace a working one. A read-only token is stored with a warning, and
/// an unreachable host doesn't stop the login, which is local.
pub fn login(matches: &ArgMatches, io: &mut Io, keys: &Keys) -> Result<(), CliError> {
    let host = resolve_host(host_flag(matches), io)?;
    let token = keys
        .prompter()
        .value(&format!("token for {host}"))
        .map_err(|e| fail(ExitCode::LockedOrAuth, e))?;
    if token.trim().is_empty() {
        return Err(remoteauth::Error::EmptyToken.into());
    }
    let dirs = dirs(io)?;
    let origin = current_origin(io).ok().flatten().and_then(|(_, o)| o);
    let Some(origin) = origin else {
        remoteauth::save(&dirs, &host, &token)?;
        out(
            io,
            &[format!(
                "gage: stored a token for {host}; there is no current vault's remote to check it against"
            )],
        );
        return Ok(());
    };
    let url = remoteauth::redacted(&origin);
    let checked = remoteauth::host(&origin)
        .is_ok_and(|h| h.eq_ignore_ascii_case(&host))
        .then(|| remotecheck::check_remote(&dirs, &origin, Some(&token)));
    let warn = |text: String| keys.prompter().warn(&Warning::from(text));
    let note = match checked {
        None => "; the current vault's remote is elsewhere, so it was not checked".to_string(),
        Some(Ok(Access::ReadWrite)) => format!("; it can read and push to {url}"),
        Some(Ok(Access::ReadOnly)) => {
            warn(format!(
                "this token can read {url} but not push to it: writes, approvals and \
                 enrolling a device there will be refused. Issue one with write access \
                 to the repository's contents and log in again"
            ));
            String::new()
        }
        Some(Err(e)) if e.kind() == ErrorKind::LockedOrAuth => {
            return Err(fail(
                ExitCode::LockedOrAuth,
                format!(
                    "{host} refused this token for {url}, so it was not stored ({e}); \
                     check that it is current and scoped to that repository"
                ),
            ));
        }
        Some(Err(e)) if e.kind() == ErrorKind::Unreachable => {
            warn(format!(
                "could not reach {url} to check the token, so it is stored unchecked ({e})"
            ));
            String::new()
        }
        Some(Err(e)) => {
            warn(format!(
                "the token could not be checked, so it is stored unchecked ({e})"
            ));
            String::new()
        }
    };
    remoteauth::save(&dirs, &host, &token)?;
    out(io, &[format!("gage: stored a token for {host}{note}")]);
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
