//! `gage clone`: join a vault that already exists somewhere else.
//!
//! Cloning does not grant access. The vault is encrypted to its
//! recipients, and a new device is not one of them, so the job here is a
//! working, registered vault *and* saying plainly when this device can't
//! read it yet. See "Vault lifecycle" in the design doc. Where a new
//! passphrase can be answered, it also offers to enroll on the spot: there
//! is deliberately no `--enroll` flag, because clone already detects this
//! exact condition and reports it, so a flag opting into acting on a fact
//! the tool just printed would carry no information, and enrolling commits
//! and pushes ("Why there is no `--enroll` flag").

use std::path::PathBuf;

use crate::exitcode::ExitCode;
use clap::{Arg, ArgMatches};
use gage::Prompter;
use gage::enrollment::DEFAULT_ENROLLMENT_TTL;
use gage::storage::config::RegistryError;
use gage::vault::CloneError;
use gage::{CloneSpec, Vault};
use gage::{storage::paths, vault};

use super::enroll::enroll_device;
use super::init::resolve_device_name;
use super::{arg, dirs, fail, out};
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

pub fn run(matches: &ArgMatches, io: &mut Io, prompter: &dyn Prompter) -> Result<(), CliError> {
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
    let path = match flag("dir") {
        Some(dir) => PathBuf::from(dir),
        None => paths::vaults_dir(&dirs).join(&name),
    };
    let spec = CloneSpec {
        name: name.clone(),
        path,
        url: url.to_string(),
        device: device.clone(),
    };
    // Clones, refuses a format_version this build doesn't know, and
    // registers; a failed clone is undone, leaving a --dir that already
    // existed as it was (ADR-0061).
    let cloned = match Vault::clone_remote(&dirs, &spec) {
        Ok(cloned) => cloned,
        Err(CloneError::Registry(RegistryError::AlreadyRegistered(name))) => {
            return Err(fail(
                ExitCode::Conflict,
                format!(
                    "{name:?} is already a registered vault; pass --name to clone it under a different name"
                ),
            ));
        }
        Err(e) => return Err(e.into()),
    };

    out(
        io,
        &[format!("gage: cloned {name:?} to {}", spec.path.display())],
    );
    if cloned.holds_identity {
        // This device already holds an identity for the vault. It was set
        // up deliberately and gage can't cheaply tell whether it is
        // already a recipient, so it is left alone: no message, and
        // certainly no prompt.
        return Ok(());
    }
    let lines = access_lines(&name, &device);
    if !can_answer_new_passphrase(io) {
        out(io, &lines);
        return Ok(());
    }
    let yes = prompter.confirm_default_yes(&format!(
        "This device holds no identity for {name:?}, so it can't read anything here yet.\nSet up an enrollment request now?"
    ))?;
    if !yes {
        out(io, &lines);
        return Ok(());
    }
    // Under the device name clone already resolved, not one derived a
    // second time: `clone --device X` followed by a yes must publish a
    // request naming X.
    enroll_device(io, &cloned.vault, &device, DEFAULT_ENROLLMENT_TTL, prompter)
}

/// Whether a brand-new identity's passphrase can be answered in this run:
/// the one condition the offer is gated on, rather than a TTY test of its
/// own. Enrolling on a device with no identity needs a `Purpose::Create`
/// exchange, and offering to enroll where that must then be refused would
/// ask a question whose only outcome is a failure. `clone` is one-shot
/// only, so a script's `--stdin` and `--script` never reach here: a
/// terminal is the whole of the rule.
fn can_answer_new_passphrase(io: &Io) -> bool {
    io.stdin_is_terminal
}

/// What to tell a device that holds no identity for the vault it just
/// cloned: it can't read anything yet, and the ways in. Whether it holds
/// one is `Vault::clone_remote`'s answer, from the file's presence alone:
/// asking someone to unlock during a clone only to say the unlock was
/// pointless would be backwards.
fn access_lines(name: &str, device: &str) -> Vec<String> {
    vec![
        format!(
            "gage: this device ({device:?}) has no identity for {name:?}, so it cannot decrypt anything in it yet."
        ),
        "gage: run `gage identity enroll` here to generate a key and publish a request to join,"
            .into(),
        "gage: then give the code it prints to someone who can already read the vault.".into(),
        "gage: or, if this device cannot write to the remote: run `gage identity add` here and"
            .into(),
        "gage: have someone with access add the printed key with `gage recipient add`.".into(),
        // A sole owner holding the paper key has nobody to approve a
        // request: for them, the two routes above are a dead end.
        "gage: if you hold this vault's recovery key and there is nobody else to ask, run".into(),
        "gage: `gage recovery enroll` — it admits this device and retires the key it used.".into(),
    ]
}
