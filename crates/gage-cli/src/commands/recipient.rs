//! `gage recipient list` and `recipient verify`: the read side of who a
//! vault is encrypted to. Neither unlocks: both read plaintext, committed
//! files, so they work on a device that holds no identity at all.

use clap::{Arg, ArgMatches};
use gage::exitcode::Code;

use super::entry::use_flag;
use super::sync::with_resolved_vault;
use super::{Keys, fail, out};
use crate::Io;
use crate::error::CliError;

pub fn flags() -> Vec<Arg> {
    vec![use_flag()]
}

pub fn list(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    with_resolved_vault(matches, io, keys, |io, vault| {
        let lines: Vec<String> = vault
            .recipients()?
            .iter()
            .map(|r| format!("{}  {}", r.device, r.pubkey))
            .collect();
        out(io, &lines);
        Ok(())
    })
}

/// Exit 0 and "in sync" when `.age-recipients` and `config.toml` list the
/// same keys; exit 1 (Conflict) with each difference named otherwise.
pub fn verify(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    with_resolved_vault(matches, io, keys, |io, vault| {
        let verdict = vault.verify_recipients()?;
        if verdict.in_sync {
            out(
                io,
                &[format!(
                    "gage: .age-recipients and .gage/config.toml are in sync ({:?})",
                    vault.name
                )],
            );
            return Ok(());
        }
        let config = vault.recipients()?;
        for key in &verdict.only_in_recipients_file {
            let _ = writeln!(
                io.stderr,
                "gage: {key} is in .age-recipients but not in .gage/config.toml: entries are encrypted to a key the vault does not list"
            );
        }
        for key in &verdict.only_in_config {
            let device = config
                .iter()
                .find(|r| &r.pubkey == key)
                .map_or("", |r| r.device.as_str());
            let _ = writeln!(
                io.stderr,
                "gage: {key} ({device}) is in .gage/config.toml but not in .age-recipients: that device cannot read new entries"
            );
        }
        Err(fail(
            Code::Conflict,
            "recipient files disagree: .age-recipients and .gage/config.toml list different keys",
        ))
    })
}
