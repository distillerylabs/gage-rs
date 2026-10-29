//! `gage identity add` and `identity list`: this device's own identities
//! for a vault, as against `recipient`, the vault's list of who may read
//! it. Adding an identity grants nothing; it prints the key and the command
//! that, run on a device with access, would. What it does lives in the
//! library (`identity_add`); this records the device, method and key in
//! global config (never in the vault) and renders.

use clap::{Arg, ArgMatches};
use gage::exitcode::Code;
use gage::vaultconfig::UnlockMethod;

use super::entry::use_flag;
use super::init::resolve_device_name;
use super::sync::with_resolved_vault;
use super::{Keys, fail, load_global, out};
use crate::Io;
use crate::error::CliError;

pub const ADD_LONG_ABOUT: &str = "Register this device's identity and print its public key.

Generates a fresh keypair for this device, wraps its private half with a
passphrase you choose, and prints the public half. The vault does not yet
trust that key: hand it to a device that can already read the vault and run
`gage recipient add <pubkey> --device NAME` there.

If this device lost its identity file, register a fresh identity under a new
name rather than restoring the old key from a backup, and have a device with
access add it. If no device can, `gage recovery enroll` gets you back in with
the vault's recovery key.";

pub fn flags(name: &str) -> Vec<Arg> {
    match name {
        "identity add" => {
            vec![
            Arg::new("device")
                .long("device")
                .value_name("string")
                .help("this device's name for the vault (default: normalized hostname)"),
            Arg::new("method").long("method").value_name("string").help(format!(
                "how this device unlocks the vault (accepted: [{}]; default: the vault's own)",
                UnlockMethod::names(" ")
            )),
            use_flag(),
        ]
        }
        _ => vec![use_flag()],
    }
}

pub fn add(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    let prompter = keys.prompter();
    with_resolved_vault(matches, io, keys, |io, vault| {
        let device = match matches.get_one::<String>("device") {
            Some(explicit) => resolve_device_name(Some(explicit), "")?,
            None => {
                let hostname = (io.hostname)().map_err(|e| {
                    CliError::internal(anyhow::anyhow!("reading the hostname: {e}"))
                })?;
                resolve_device_name(None, &hostname)?
            }
        };
        // The vault's own method is a suggestion for a joining device; this
        // device's is its own choice, recorded locally (ADR-0011).
        let method = match matches.get_one::<String>("method") {
            Some(method) => UnlockMethod::from(method.as_str()),
            None => vault.info()?.method,
        };
        if let UnlockMethod::Other(method) = &method {
            return Err(fail(
                Code::Usage,
                format!(
                    "unknown method {method:?}; accepted: {}",
                    UnlockMethod::names(", ")
                ),
            ));
        }

        let pubkey = vault.add_identity(&device, prompter)?;

        record_local_identity(io, &vault.name, &device, &method, &pubkey)?;

        let name = &vault.name;
        out(
            io,
            &[
                format!("gage: registered identity {device:?} for {name:?}; public key {pubkey}"),
                format!(
                    "gage: {name:?} does not trust it yet. On a device that can already read it, run:"
                ),
                format!("gage:   gage recipient add {pubkey} --device {device}"),
            ],
        );
        Ok(())
    })
}

/// Records this device's name, method and key for a vault in global config,
/// never in the vault: the library never writes global config.
pub(super) fn record_local_identity(
    io: &Io,
    vault: &str,
    device: &str,
    method: &UnlockMethod,
    pubkey: &str,
) -> Result<(), CliError> {
    let (path, mut global) = load_global(io)?;
    let entry = global.vaults.get_mut(vault).ok_or_else(|| {
        fail(
            Code::NotFound,
            format!("{vault:?} is not a registered vault"),
        )
    })?;
    entry.device = device.to_string();
    entry.method = method.clone();
    entry.pubkey = Some(pubkey.to_string());
    gage::config::save(&path, &global)?;
    Ok(())
}

pub fn list(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    with_resolved_vault(matches, io, keys, |io, vault| {
        let identities = vault.list_identities()?;
        if identities.is_empty() {
            out(
                io,
                &[format!(
                    "gage: this machine holds no identity for {:?}; run `gage identity add`",
                    vault.name
                )],
            );
            return Ok(());
        }
        let lines: Vec<String> = identities
            .iter()
            .map(|i| {
                let marker = if i.current { '*' } else { ' ' };
                format!("{marker} {}  {}", i.device, i.path.display())
            })
            .collect();
        out(io, &lines);
        Ok(())
    })
}
