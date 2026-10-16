//! `gage vault list/info/remove/set-default`: list, inspect, forget, or
//! re-point vaults already registered. See "Vault lifecycle" in the design
//! doc.

use clap::ArgMatches;
use gage::config;
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
/// touched. It reads nothing from the vault, so it works when the vault's
/// files are gone, which is when it is most needed.
///
/// This device's trust cache for the vault goes with it. Keeping it would
/// let a re-add silently inherit an approval of a list nobody looked at in
/// between, the one thing the cache exists to prevent; dropping it makes a
/// re-add a first use. It is keyed by the registration's own recorded id,
/// not a resolved vault, for the reason above. A registration with no id
/// has no cache to remove, and is no reason to fail: forgetting a
/// registration must always be possible.
pub fn remove(matches: &ArgMatches, io: &mut Io) -> Result<(), CliError> {
    let (path, mut global) = load_global(io)?;
    let entry = global.unregister(arg(matches, "name"))?;
    config::save(&path, &global)?;
    if !entry.id.is_empty() {
        trustcache::remove(&dirs(io)?, &entry.id)?;
    }
    Ok(())
}

pub fn set_default(matches: &ArgMatches, io: &mut Io) -> Result<(), CliError> {
    let (path, mut global) = load_global(io)?;
    global.set_default(arg(matches, "name"))?;
    config::save(&path, &global)?;
    Ok(())
}
