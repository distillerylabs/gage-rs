//! `gage init`: resolves the defaults (device name, target path), refuses
//! what is knowable from the flags, generates this device's identity (the
//! passphrase prompt), creates the vault through the library with that
//! key first, and only once it exists registers it in global config. If
//! creating the vault fails, the identity is removed again.
//!
//! There is no `--remote` until P4 · M8a, since Go's publishes immediately
//! and that needs push, and no recovery key until P7.

use std::path::PathBuf;

use clap::{Arg, ArgAction, ArgMatches};
use gage::config::{RegistryError, VaultEntry};
use gage::exitcode::Code;
use gage::prompter::Prompter;
use gage::vault::{self, CreateSpec, Vault};
use gage::{devicename, identityfile, paths, vaultconfig};

use super::{arg, dirs, fail, load_global, out};
use crate::Io;
use crate::error::CliError;

pub const LONG_ABOUT: &str = "Create a new vault.

Generates this device's identity, wraps its private key with a passphrase
you choose, and writes the public half into the vault's recipient files.
The wrapped key is stored outside the vault and is never committed or synced.

--recipient is additive: each one is written alongside this device's own key,
for recipients you already hold public keys for.";

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
            .help("additional recipient public key (repeatable; this device's own key is always included)"),
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

pub fn run(matches: &ArgMatches, io: &mut Io, prompter: &dyn Prompter) -> Result<(), CliError> {
    let name = arg(matches, "name");
    let flag = |id| matches.get_one::<String>(id).cloned();
    let extra_recipients: Vec<String> = matches
        .get_many::<String>("recipient")
        .into_iter()
        .flatten()
        .cloned()
        .collect();

    let device = match flag("device") {
        Some(explicit) => resolve_device_name(Some(&explicit), "")?,
        None => {
            let hostname = (io.hostname)()
                .map_err(|e| CliError::internal(anyhow::anyhow!("reading the hostname: {e}")))?;
            resolve_device_name(None, &hostname)?
        }
    };

    let dirs = dirs(io)?;
    let (config_path, mut global) = load_global(io)?;
    if global.vaults.contains_key(name) {
        return Err(RegistryError::AlreadyRegistered(name.to_string()).into());
    }
    let path = match flag("dir") {
        Some(dir) => PathBuf::from(dir),
        None => paths::vaults_dir(&dirs).join(name),
    };

    // The id is minted here, before the identity, and handed to create
    // rather than minted there: the identity file is filed under it, and
    // an id that only existed after the file was written would leave the
    // vault carrying one id and this device's only key filed under
    // another, with nothing visibly wrong (A20).
    let mut spec = CreateSpec {
        name: name.to_string(),
        id: vaultconfig::new_id(),
        path,
        vault_type: flag("type").expect("defaulted"),
        method: flag("method").expect("defaulted"),
        device,
        recipients: extra_recipients,
        remote: None,
    };
    // Everything knowable from the flags fails before a human types a
    // passphrase twice (Q-INIT-ORDER). The target directory is checked by
    // create, after the key exists, as Go does; the rollback below covers
    // it.
    spec.validate_without_device_key()?;

    let pubkey = identityfile::create(&dirs, &spec.id, name, &spec.device, prompter)?;
    // This device's key leads and --recipient keys follow in the order
    // given: create labels the first recipient with the device name.
    spec.recipients.insert(0, pubkey.clone());

    let created = match Vault::create(&dirs, &spec) {
        Ok(created) => created,
        Err(err) => {
            // Unconditionally: the key is referenced by nothing, and sits
            // under an id that now belongs to no vault. `init` mints a new
            // id every run, so without this every failed attempt would
            // leave one orphaned private key behind.
            if let Err(rollback) = identityfile::remove(&dirs, &spec.id, &spec.device) {
                return Err(fail(
                    Code::Internal,
                    format!("{err} (and rolling back the generated identity failed: {rollback})"),
                ));
            }
            return Err(err.into());
        }
    };

    global.register(
        name,
        VaultEntry {
            path: created.path.clone(),
            id: created.id.clone(),
            vault_type: spec.vault_type.clone(),
            device: spec.device.clone(),
            // Recorded now, the one moment gage holds this device's key
            // without an unlock, so "is my key still a recipient?" can be
            // answered by comparing keys (Q-ORPHAN-BY-NAME).
            pubkey: pubkey.clone(),
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
            format!(
                "gage: this device is {:?}, public key {pubkey}",
                spec.device
            ),
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
