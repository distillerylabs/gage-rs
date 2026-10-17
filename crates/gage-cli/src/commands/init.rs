//! `gage init`: resolves the defaults (device name, target path), refuses
//! what is knowable from the flags, generates this device's identity (the
//! passphrase prompt), creates the vault through the library with that
//! key first, and only once it exists registers it in global config. If
//! creating the vault fails, the identity is removed again.
//!
//! `--remote` sets origin and publishes the new vault at once, asking for
//! the host's token first if none is stored (#38). The recovery key is
//! P8's.

use std::path::{Path, PathBuf};

use clap::{Arg, ArgAction, ArgMatches};
use gage::config::{GitMeta, RegistryError, VaultEntry};
use gage::exitcode::{Code, HasCode};
use gage::prompter::Prompter;
use gage::syncerr::Kind;
use gage::vault::{self, CreateSpec, Vault};
use gage::{devicename, identityfile, paths, remoteauth, vaultconfig};

use super::{arg, dirs, fail, load_global, out};
use crate::Io;
use crate::error::CliError;

pub const LONG_ABOUT: &str = "Create a new vault.

Generates this device's identity, wraps its private key with a passphrase
you choose, and writes the public half into the vault's recipient files.
The wrapped key is stored outside the vault and is never committed or synced.

--recipient is additive: each one is written alongside this device's own key,
for recipients you already hold public keys for.

--remote publishes the new vault immediately. If its host needs a token and
none is stored yet, init asks for one on the spot (leave blank to skip and
try anonymously) — the same prompt as `gage auth login`, so a private repo
doesn't need a separate auth login round trip before the first push succeeds.";

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
        Arg::new("remote")
            .long("remote")
            .value_name("URL")
            .help("git remote (origin) URL; omit to start local-only"),
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
        extra_recipients: Vec::new(),
        remote: flag("remote"),
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
            git: GitMeta {
                origin: spec.remote.clone().unwrap_or_default(),
            },
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
    match &spec.remote {
        Some(remote) => publish(io, prompter, &created, remote),
        None => Ok(()),
    }
}

/// Pushes the new vault's first commit to the remote it was given: naming
/// a remote at creation means the vault is on it, not that it will be
/// after some later write. The vault is created and registered by now and
/// stays so whatever happens here: it is durable locally, and being unable
/// to publish is a thing to report, never a reason to throw it away.
fn publish(io: &mut Io, prompter: &dyn Prompter, v: &Vault, remote: &str) -> Result<(), CliError> {
    ensure_remote_token(io, prompter, remote)?;
    let name = &v.name;
    let err = match v.push() {
        Ok(_) => {
            out(io, &[format!("gage: published {name:?} to {remote}")]);
            return Ok(());
        }
        Err(err) => err,
    };
    // Offline at creation is not a failure: the next successful sync
    // publishes it. A local path that isn't there is not offline, though:
    // gage never creates a repository, so at init it is one that hasn't
    // been made yet (Go reports it as such; for a sync, the same path is
    // an unmounted drive, Q-SYNC-CLASSIFY).
    let missing_path =
        remoteauth::host(remote).is_ok_and(|h| h.is_empty()) && !Path::new(remote).exists();
    if err.remote_kind() == Some(Kind::Unreachable) && !missing_path {
        let _ = writeln!(
            io.stderr,
            "gage: could not reach {remote}; {name:?} exists locally and will publish on the next successful sync"
        );
        return Ok(());
    }
    let code = if missing_path {
        Code::LockedOrAuth
    } else {
        err.code()
    };
    let msg = err.to_string();
    Err(fail(
        code,
        format!(
            "{name:?} was created locally, but publishing it to {remote} failed: {}\n\
             gage: gage never creates a repository for you — create an empty one there, then run `gage push`",
            msg.strip_prefix("gage: ").unwrap_or(&msg)
        ),
    ))
}

/// Asks for a token for `remote`'s host, once, before the first push, so
/// a private HTTPS repository doesn't take three commands (init, auth
/// login, push) to publish. Only for a host that is a token candidate at
/// all, never a local path or an SSH remote, and only when no token is
/// stored. A blank answer stores nothing and the push goes anonymously:
/// not every remote needs a token, and init can't know which kind this is
/// without asking.
fn ensure_remote_token(io: &mut Io, prompter: &dyn Prompter, remote: &str) -> Result<(), CliError> {
    let host = remoteauth::token_host(remote)?;
    if host.is_empty() {
        return Ok(());
    }
    let dirs = dirs(io)?;
    match remoteauth::load(&dirs, &host) {
        Ok(_) => return Ok(()),
        Err(remoteauth::Error::NoToken { .. }) => {}
        Err(e) => return Err(e.into()),
    }
    let token = prompter
        .value(&format!("token for {host} (leave blank to skip)"))
        .map_err(|e| fail(Code::LockedOrAuth, e))?;
    if token.trim().is_empty() {
        return Ok(());
    }
    remoteauth::store(&dirs, &host, &token)?;
    out(io, &[format!("gage: stored a token for {host}")]);
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
