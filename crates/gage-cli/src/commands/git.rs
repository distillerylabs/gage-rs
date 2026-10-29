//! `gage git ...`: the commands that exist only because the vault type is
//! git. No generic passthrough; see "Git-specific commands" in the design
//! doc.

use clap::ArgMatches;
use gage::{config, gitrepo};

use super::{arg, load_global};
use crate::Io;
use crate::error::CliError;

/// Sets the repository's `origin` and the registration's
/// `[vaults.<name>.git].origin` in one call, so the two never drift.
pub fn set_remote(matches: &ArgMatches, io: &mut Io) -> Result<(), CliError> {
    let (name, url) = (arg(matches, "name"), arg(matches, "url"));
    let (path, mut global) = load_global(io)?;
    let mut entry = global.entry(name)?.clone();

    gitrepo::set_remote(&entry.path, url)?;
    entry.git.origin = Some(url.to_string());
    global.vaults.insert(name.to_string(), entry);
    config::save_creating_dir(&path, &global)?;
    Ok(())
}
