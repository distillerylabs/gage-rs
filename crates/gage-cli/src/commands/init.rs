//! `gage init`: resolves the defaults (device name, target path), refuses
//! a name already registered, creates the vault through the library, and
//! only once it exists registers it in global config.
//!
//! Until P2 · M2 there is no identity to generate, so `--recipient` is
//! required and its first key is labelled as this device's own
//! (Q-INIT-INTERIM). There is no `--remote` until P4 · M8a, since Go's
//! publishes immediately and that needs push.

use std::path::PathBuf;

use clap::{Arg, ArgAction, ArgMatches};
use gage::config::{RegistryError, VaultEntry};
use gage::exitcode::Code;
use gage::vault::{self, CreateSpec, Vault};
use gage::{devicename, paths, vaultconfig};

use super::{arg, dirs, fail, load_global, out};
use crate::Io;
use crate::error::CliError;

pub const LONG_ABOUT: &str = "Create a new vault.

Creates the vault's directory, its committed config (.gage/config.toml) and
recipient list (.age-recipients), makes one git commit, and registers the
vault on this machine. Nothing is encrypted yet.

--recipient is required for now: the first key given is labelled as this
device's own (--device, or the normalized hostname), any later ones
recipient-2, recipient-3, and so on. A later release generates this
device's key itself and makes --recipient additive.";

pub fn flags() -> Vec<Arg> {
    let accepted = |list: &[&str]| format!("[{}]", list.join(" "));
    vec![
        Arg::new("device")
            .long("device")
            .value_name("NAME")
            .help("this device's name (default: normalized hostname)"),
        Arg::new("dir")
            .long("dir")
            .value_name("PATH")
            .help("target directory (default: $GAGE_DATA/vaults/<name>)"),
        Arg::new("method")
            .long("method")
            .value_name("METHOD")
            .default_value(vault::METHOD_PASSPHRASE)
            .help(format!(
                "default identity method for devices joining this vault (accepted: {})",
                accepted(vault::ALLOWED_METHODS)
            )),
        Arg::new("recipient")
            .long("recipient")
            .value_name("KEY")
            .action(ArgAction::Append)
            .help("recipient public key (repeatable; required for now, the first is this device's own)"),
        Arg::new("type")
            .long("type")
            .value_name("TYPE")
            .default_value(vault::TYPE_GIT)
            .help(format!(
                "vault type (accepted: {})",
                accepted(vault::ALLOWED_TYPES)
            )),
    ]
}

pub fn run(matches: &ArgMatches, io: &mut Io) -> Result<(), CliError> {
    let name = arg(matches, "name");
    let flag = |id| matches.get_one::<String>(id).cloned();
    let recipients: Vec<String> = matches
        .get_many::<String>("recipient")
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    if recipients.is_empty() {
        return Err(fail(
            Code::Usage,
            "--recipient KEY is required: this version of gage can't generate this device's key yet, so pass the public key it will decrypt with",
        ));
    }

    let device = match flag("device") {
        Some(explicit) => resolve_device_name(Some(&explicit), "")?,
        None => {
            let hostname = (io.hostname)()
                .map_err(|e| CliError::internal(anyhow::anyhow!("reading the hostname: {e}")))?;
            resolve_device_name(None, &hostname)?
        }
    };

    let (config_path, mut global) = load_global(io)?;
    if global.vaults.contains_key(name) {
        return Err(RegistryError::AlreadyRegistered(name.to_string()).into());
    }
    let path = match flag("dir") {
        Some(dir) => PathBuf::from(dir),
        None => paths::vaults_dir(&dirs(io)?).join(name),
    };

    let spec = CreateSpec {
        name: name.to_string(),
        id: vaultconfig::new_id(),
        path,
        vault_type: flag("type").expect("defaulted"),
        method: flag("method").expect("defaulted"),
        device,
        recipients,
        remote: None,
    };
    let created = Vault::create(&spec)?;

    global.register(
        name,
        VaultEntry {
            path: created.path.clone(),
            id: created.id.clone(),
            vault_type: spec.vault_type.clone(),
            device: spec.device.clone(),
            method: spec.method.clone(),
            ..Default::default()
        },
    )?;
    gage::config::save(&config_path, &global)?;

    out(
        io,
        &[
            format!(
                "gage: initialized vault {name:?} at {}",
                created.path.display()
            ),
            format!("gage: this device is {:?}", spec.device),
        ],
    );
    Ok(())
}

/// `--device` if given (it must already be a valid name: an explicit name
/// is never rewritten into a different one), else the normalized
/// hostname. A hostname that normalizes to nothing fails with Usage naming
/// the flag rather than inventing a name: one-shot mode has no free-text
/// prompt to ask with.
pub fn resolve_device_name(explicit: Option<&str>, hostname: &str) -> Result<String, CliError> {
    if let Some(explicit) = explicit {
        return if devicename::valid(explicit) {
            Ok(explicit.to_string())
        } else {
            Err(fail(
                Code::Usage,
                format!("--device {explicit:?} is invalid"),
            ))
        };
    }
    devicename::normalize(hostname).ok_or_else(|| {
        fail(
            Code::Usage,
            format!(
                "this machine's hostname ({hostname:?}) doesn't normalize to a usable device name; pass --device NAME"
            ),
        )
    })
}
