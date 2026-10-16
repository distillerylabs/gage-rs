//! `gage clone`: join a vault that already exists somewhere else.
//!
//! Cloning does not grant access. The vault is encrypted to its
//! recipients, and a new device is not one of them, so the job here is a
//! working, registered vault *and* saying plainly when this device can't
//! read it yet. See "Vault lifecycle" in the design doc. The offer to
//! enroll on the spot is P7's.

use std::path::{Path, PathBuf};

use clap::{Arg, ArgMatches};
use gage::config::{GitMeta, VaultEntry};
use gage::exitcode::Code;
use gage::gitrepo::{self, RemoteOptions};
use gage::{paths, vault, vaultconfig};

use super::init::resolve_device_name;
use super::{arg, dirs, fail, load_global, out};
use crate::Io;
use crate::error::CliError;

pub const LONG_ABOUT: &str = "Clone an existing vault from a remote.

Cloning gets you the vault's files and history; it does NOT grant you access
to their contents. If this device isn't already a recipient, gage says so and
points at the command that generates a key to be added from a device that is.";

pub fn flags() -> Vec<Arg> {
    vec![
        Arg::new("device")
            .long("device")
            .value_name("NAME")
            .help("this device's name (default: normalized hostname)"),
        Arg::new("dir")
            .long("dir")
            .value_name("PATH")
            .help("target directory (default: $GAGE_DATA/vaults/<name>)"),
        Arg::new("name")
            .long("name")
            .value_name("NAME")
            .help("local name for the vault (default: inferred from the remote URL)"),
    ]
}

pub fn run(matches: &ArgMatches, io: &mut Io) -> Result<(), CliError> {
    let url = arg(matches, "remote-url");
    let flag = |id| matches.get_one::<String>(id).cloned();

    // Every name goes through the check, inferred or given: an inferred one
    // comes from whoever controls the URL.
    let name = match flag("name") {
        Some(name) => name,
        None => vault::name_from_url(url)?,
    };
    vault::check_name(&name)?;

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
    if global.vaults.contains_key(&name) {
        return Err(fail(
            Code::Conflict,
            format!(
                "gage: {name:?} is already a registered vault; pass --name to clone it under a different name"
            ),
        ));
    }
    let path = match flag("dir") {
        Some(dir) => PathBuf::from(dir),
        None => paths::vaults_dir(&dirs).join(&name),
    };

    // What to undo on failure is this command's own work only. A --dir
    // that already existed is the user's: one that was empty is emptied
    // again, and one that wasn't (libgit2 refuses it) is never touched.
    let prior = Prior::of(&path);
    let undo = |err: CliError| match roll_back(&path, prior) {
        Ok(()) => err,
        Err(e) => fail(
            Code::Internal,
            format!(
                "{err} (and removing the partial clone at {} failed: {e})",
                path.display()
            ),
        ),
    };

    gitrepo::clone(url, &path, &RemoteOptions::new(&dirs)).map_err(|e| undo(e.into()))?;
    // Read before anything is registered: a format_version this build
    // doesn't know means it can't operate on the vault safely, and
    // registering it anyway would trade one clear failure now for a
    // confusing one on every later command.
    let file =
        vaultconfig::read(&path.join(".gage").join("config.toml")).map_err(|e| undo(e.into()))?;

    global.register(
        &name,
        VaultEntry {
            path: path.clone(),
            id: file.vault.id.clone(),
            vault_type: file.vault.vault_type.clone(),
            device: device.clone(),
            // A clone holds no identity, so there is no public key to
            // record until `identity add` puts one here.
            pubkey: String::new(),
            // The vault's default is a suggestion for joining devices, and
            // what a later `identity add` offers (Q-METHOD-SCOPE).
            method: file.method.default.clone(),
            git: GitMeta {
                origin: url.to_string(),
            },
        },
    )?;
    gage::config::save(&config_path, &global)?;

    out(
        io,
        &[format!("gage: cloned {name:?} to {}", path.display())],
    );
    out(io, &access_lines(&dirs, &file.vault.id, &name, &device));
    Ok(())
}

/// Whether this device can read what it just cloned, answered from whether
/// it holds an identity for the vault at all: no passphrase needed. Asking
/// someone to unlock during a clone only to say the unlock was pointless
/// would be backwards. An identity that exists was set up deliberately and
/// is left alone, as is anything that can't be checked.
fn access_lines(dirs: &gage::xdgpaths::Dirs, id: &str, name: &str, device: &str) -> Vec<String> {
    match paths::identity_file_path(dirs, id, device) {
        Ok(path) if !path.exists() => vec![
            format!(
                "gage: this device ({device:?}) has no identity for {name:?}, so it cannot decrypt anything in it yet."
            ),
            "gage: run `gage identity add` here and have someone with access add the printed key"
                .into(),
            "gage: with `gage recipient add`.".into(),
        ],
        _ => Vec::new(),
    }
}

/// What was at the clone's target before it ran.
#[derive(Clone, Copy)]
enum Prior {
    Absent,
    EmptyDir,
    /// A file, a non-empty directory, or something unreadable: not ours.
    Other,
}

impl Prior {
    fn of(path: &Path) -> Prior {
        if !path.exists() {
            return Prior::Absent;
        }
        match std::fs::read_dir(path).map(|mut entries| entries.next().is_none()) {
            Ok(true) => Prior::EmptyDir,
            _ => Prior::Other,
        }
    }
}

/// Undoes a failed clone so a retry isn't blocked by a leftover: removes
/// the directory it made, or empties the empty one it was given.
fn roll_back(path: &Path, prior: Prior) -> std::io::Result<()> {
    match prior {
        Prior::Other => return Ok(()),
        Prior::Absent => {
            return match std::fs::remove_dir_all(path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            };
        }
        Prior::EmptyDir => {}
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}
