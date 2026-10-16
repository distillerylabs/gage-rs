//! `gage vault list/info/remove/set-default`: list, inspect, forget, or
//! re-point vaults already registered. See "Vault lifecycle" in the design
//! doc.

use clap::ArgMatches;
use gage::config::{self, VaultEntry};
use gage::identityfile;
use gage::orphan::{self, KeepReason, OrphanCheck};
use gage::prompter::Prompter;
use gage::trustcache;
use gage::vault::Vault;

use super::{arg, dirs, load_global, out};
use crate::Io;
use crate::error::CliError;

/// Every registered name, sorted, the current one marked `*`.
pub fn list(io: &mut Io) -> Result<(), CliError> {
    let (_, global) = load_global(io)?;
    let lines: Vec<String> = global
        .vaults
        .keys()
        .map(|name| {
            let marker = if *name == global.current { "* " } else { "  " };
            format!("{marker}{name}")
        })
        .collect();
    out(io, &lines);
    Ok(())
}

/// One vault's details, read from the vault itself through the resolver,
/// so a registration pointing at a different vault is refused here too
/// rather than described.
pub fn info(matches: &ArgMatches, io: &mut Io) -> Result<(), CliError> {
    let (_, global) = load_global(io)?;
    let given = matches.get_one::<String>("name").map(String::as_str);
    let name = global.name_or_current(given)?;
    let vault = Vault::from_registration(&dirs(io)?, name, global.entry(name)?)?;
    let info = vault.info()?;

    let mut lines = vec![
        format!("name: {name}"),
        format!("id: {}", info.id),
        format!("type: {}", info.vault_type),
        format!("method: {}", info.method),
        format!("recipients: {}", info.recipients),
    ];
    if let Some(git) = info.git {
        lines.push(format!(
            "remote: {}",
            git.remote.as_deref().unwrap_or("(none)")
        ));
        lines.push(format!(
            "status: {}",
            if git.clean { "clean" } else { "dirty" }
        ));
    }
    out(io, &lines);
    Ok(())
}

/// Forgets the registration only: the vault's own files are never
/// touched. It reads nothing from the vault to do so, which is when it is
/// most needed: it works when the vault's files are gone.
///
/// This device's trust cache for the vault goes with it. Keeping it would
/// let a re-add silently inherit an approval of a list nobody looked at in
/// between, the one thing the cache exists to prevent; dropping it makes a
/// re-add a first use. It is keyed by the registration's own recorded id,
/// not a resolved vault, for the reason above. A registration with no id
/// has no cache to remove, and is no reason to fail: forgetting a
/// registration must always be possible.
///
/// Last, and only after the registration is gone, the identity file: see
/// [`offer_orphaned_identity`]. Nothing about it can fail the command.
pub fn remove(matches: &ArgMatches, io: &mut Io, prompter: &dyn Prompter) -> Result<(), CliError> {
    let name = arg(matches, "name");
    let (path, mut global) = load_global(io)?;
    let entry = global.unregister(name)?;
    config::save(&path, &global)?;
    let dirs = dirs(io)?;
    if !entry.id.is_empty() {
        trustcache::remove(&dirs, &entry.id)?;
    }
    offer_orphaned_identity(io, prompter, &dirs, name, &entry);
    Ok(())
}

/// Offers to delete this device's identity file for the vault just
/// forgotten, when the library finds the key in it is no longer a
/// recipient. The file is the only copy of a private key, so:
///
/// - it is asked about, naming the path, never deleted on a guess, and a
///   confirmation that fails (no terminal, end of input) is a no, so a
///   scripted `vault remove` keeps it by construction
/// - every case that is not an offer keeps it and says why on stderr, since
///   guessing wrong would strand the vault's ciphertext for good
fn offer_orphaned_identity(
    io: &mut Io,
    prompter: &dyn Prompter,
    dirs: &gage::xdgpaths::Dirs,
    name: &str,
    entry: &VaultEntry,
) {
    let kept = |io: &mut Io, path: &std::path::Path, why: String| {
        let _ = writeln!(
            io.stderr,
            "gage: kept the local identity file for {name:?} ({}) \u{2014} {why}.",
            path.display()
        );
    };
    match orphan::assess(dirs, name, entry) {
        OrphanCheck::Nothing => {}
        OrphanCheck::Keep { path, reason } => {
            let why = match reason {
                KeepReason::NoPublicKeyRecorded { device } => format!(
                    "gage doesn't know which public key {device} holds here, so it can't tell whether the key is still a recipient. Delete the file yourself once you're sure it's safe"
                ),
                KeepReason::RegistrationDoesNotMatch { detail } => detail,
                KeepReason::RecipientsUnreadable { device, detail } => {
                    format!("could not confirm {device}'s key is no longer a recipient ({detail})")
                }
                KeepReason::StillARecipient { label } => format!(
                    "the key it holds is still listed as a recipient there (as {label:?}). Remove it as a recipient first (from another device, with `gage recipient remove {label} --reencrypt`) if you want the local identity file gone too"
                ),
            };
            kept(io, &path, why);
        }
        OrphanCheck::Offer { path, device } => {
            let confirmed = prompter
                .confirm(&format!(
                    "{device}'s key is no longer a recipient of {name:?}. Delete this device's identity file for it?\n  {}\nThis is the only copy of that private key, and deleting it cannot be undone.",
                    path.display()
                ))
                .unwrap_or(false);
            if !confirmed {
                return kept(io, &path, "not confirmed".to_string());
            }
            if let Err(e) = identityfile::remove(dirs, &entry.id, &entry.device) {
                let _ = writeln!(
                    io.stderr,
                    "gage: could not remove the local identity file for {name:?}: {e}"
                );
            }
        }
    }
}

pub fn set_default(matches: &ArgMatches, io: &mut Io) -> Result<(), CliError> {
    let (path, mut global) = load_global(io)?;
    global.set_default(arg(matches, "name"))?;
    config::save(&path, &global)?;
    Ok(())
}
