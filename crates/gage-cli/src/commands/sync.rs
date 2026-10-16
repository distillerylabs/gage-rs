//! `gage pull`, `push` and `sync`: the manual sync verbs. Each resolves its
//! vault without unlocking it. A fast-forward, and a merge whose two sides
//! touched different entries, decrypt nothing, so a passphrase asked for up
//! front would be one most syncs never use. `sync` unlocks lazily, only
//! when an entry conflict needs decrypting to be resolved, and a session
//! hands over the key it already holds. What sync does lives in the
//! library (`remotesync`, `syncresolve`); this is flags, vault resolution
//! and rendering.

use clap::{Arg, ArgMatches};
use gage::exitcode::Code;
use gage::gitrepo;
use gage::remotesync::{ConflictKind, SyncError, SyncReport};
use gage::syncresolve::entry_count;
use gage::vault::Vault;

use super::entry::{open_vault, use_flag, use_name};
use super::{Keys, fail, out};
use crate::Io;
use crate::commands::session_failure;
use crate::error::CliError;

pub const PULL_LONG_ABOUT: &str = "Fast-forward this vault from its remote.

Fast-forward only: if the remote has moved in a way this vault can't simply
catch up to, nothing local is touched and the divergence is reported.";

pub const SYNC_LONG_ABOUT: &str = "Pull, merge what can be merged, then push.

Pulls, merges a divergence whose two sides touched different entries, and
pushes the result. An entry both sides changed is presented with each
version's date and device, to keep, replace, or keep both under separate
ids — gage never guesses which version of a secret to keep, and with
nobody to ask (a script, CI) it stops rather than choosing.";

pub fn flags() -> Vec<Arg> {
    vec![use_flag()]
}

/// Fetch and fast-forward, never merge: a divergence is reported, as a
/// conflict for a script to see.
pub fn pull(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    with_sync_vault(matches, io, keys, |io, vault| {
        let report = vault.pull()?;
        if report.diverged {
            return Err(fail(Code::Conflict, format!("gage: {}", report.summary())));
        }
        out(io, &[sync_line(vault, &report)]);
        Ok(())
    })
}

/// Publish, merging a divergence whose two sides touched different paths.
pub fn push(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    with_sync_vault(matches, io, keys, |io, vault| {
        finish(io, vault, vault.push())
    })
}

/// Catch up, merge, resolve each entry conflict by asking, then publish.
/// One-shot mode unlocks on the spot if a conflict needs it, and the key
/// is dropped with the command; a session uses the key it holds.
pub fn sync(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    match keys {
        Keys::OneShot(prompter) => {
            let prompter = *prompter;
            let vault = open_vault(matches, io)?;
            let result = vault.sync_resolving(prompter, &mut || vault.unlock(prompter));
            finish(io, &vault, result)
        }
        Keys::Session(session) => {
            let result = session
                .sync_resolving(use_name(matches))
                .map_err(session_failure)?;
            let vault = session
                .vault_without_unlocking(use_name(matches))
                .map_err(session_failure)?;
            finish(io, vault, result)
        }
    }
}

/// The one-line outcome, or the failure with its conflicting paths named
/// first. `push` reports them exactly as `sync` does: a push is where a
/// divergence is discovered, so it can't be the less informative way to
/// learn of one.
fn finish(
    io: &mut Io,
    vault: &Vault,
    result: Result<SyncReport, SyncError>,
) -> Result<(), CliError> {
    match result {
        Ok(report) => {
            out(io, &[sync_line(vault, &report)]);
            Ok(())
        }
        Err(err) => {
            for line in report_conflicts(vault, &err.report) {
                let _ = writeln!(io.stderr, "{line}");
            }
            Err(err.into())
        }
    }
}

/// Runs `f` with the vault `--use` names (or the current one), never
/// unlocked: one-shot from its registration, in a session through the
/// session, so a pull still invalidates the session's index.
fn with_sync_vault<T>(
    matches: &ArgMatches,
    io: &mut Io,
    keys: &mut Keys,
    f: impl FnOnce(&mut Io, &Vault) -> Result<T, CliError>,
) -> Result<T, CliError> {
    match keys {
        Keys::OneShot(_) => {
            let vault = open_vault(matches, io)?;
            f(io, &vault)
        }
        Keys::Session(session) => {
            let vault = session
                .vault_without_unlocking(use_name(matches))
                .map_err(session_failure)?;
            f(io, vault)
        }
    }
}

/// What a successful sync did, in one line. A vault with no origin says
/// so, rather than claiming to be "in sync with" a remote that doesn't
/// exist; a lookup that fails falls through to the ordinary wording,
/// since it is not worth failing a sync that succeeded.
pub(crate) fn sync_line(vault: &Vault, r: &SyncReport) -> String {
    let name = &vault.name;
    if let Ok(None) = gitrepo::remote_url(&vault.path) {
        return format!("gage: {name:?} is local-only; there is no remote to sync with");
    }
    match (r.merged, r.pulled, r.pushed) {
        // The one outcome a human produced: they answered the questions,
        // and the line says so rather than describing it as gage's doing.
        (true, _, true) if r.resolved > 0 => format!(
            "gage: resolved {} in {name:?}, merged origin's changes and pushed",
            entry_count(r.resolved)
        ),
        (true, _, true) => format!("gage: merged origin's changes into {name:?} and pushed"),
        (_, true, true) => format!("gage: caught {name:?} up with origin and pushed"),
        (_, _, true) => format!("gage: pushed {name:?} to origin"),
        (_, true, false) => format!("gage: caught {name:?} up with origin"),
        _ => format!("gage: {name:?} is already in sync with origin"),
    }
}

/// The paths a divergence couldn't merge, so a human sees what conflicts
/// and not only that something does. Entry files are named by UUID, which
/// is all gage can say without decrypting them. Empty when nothing
/// conflicted.
pub(crate) fn report_conflicts(vault: &Vault, r: &SyncReport) -> Vec<String> {
    if r.conflicts.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "gage: {} path(s) changed on both sides:",
        r.conflicts.len()
    )];
    lines.extend(r.conflicts.iter().map(|path| format!("  {path}")));
    if r.conflict_kind() == ConflictKind::Recipients {
        lines.push(
            "gage: these define who can read this vault, so gage will never merge them for you."
                .into(),
        );
    }
    lines.push(format!(
        "gage: the vault is an ordinary git repository at {}",
        vault.path.display()
    ));
    lines
}
