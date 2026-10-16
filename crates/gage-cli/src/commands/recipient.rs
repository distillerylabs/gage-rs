//! `gage recipient`: who a vault is encrypted to. `list` and `verify` are
//! the read side and never unlock: both read plaintext, committed files,
//! so they work on a device that holds no identity at all. `add` and
//! `remove` change the list, and always re-encrypt every entry, in one
//! commit or not at all; what they do lives in the library
//! (`gage::recipient`).

use clap::{Arg, ArgAction, ArgMatches};
use gage::exitcode::Code;
use gage::recipient::RecipientChange;

use super::entry::use_flag;
use super::sync::with_resolved_vault;
use super::{Keys, arg, fail, out};
use crate::Io;
use crate::error::CliError;

pub const ADD_LONG_ABOUT: &str = "Authorize a public key and re-encrypt the vault to include it.

Every entry is decrypted and rewritten to the new list, so the new recipient
can read the vault's whole history — there is no way to add a recipient who
can read only part of it. All of it lands in a single commit or none of it
does. Adding a recipient therefore requires being able to read every entry
yourself; run it from a device that can.";

pub const REMOVE_LONG_ABOUT: &str = "Revoke a recipient and re-encrypt the vault without its key.

--reencrypt is required: without it the removed key still opens every entry
that already exists, so the revocation would only look like one. Removing
this device's own key is allowed but asks first, and removing the last
recipient is refused outright.";

pub fn flags(name: &str) -> Vec<Arg> {
    match name {
        // No --reencrypt: adding always re-encrypts (A19). The flag is not
        // accepted and ignored, since a script passing it asked for what
        // used to be optional; clap rejects it as a usage error, the loud
        // version.
        "recipient add" => vec![
            // Required: a bare key says nothing about whose it is, and the
            // label is what `remove` and `list` address it by.
            Arg::new("device")
                .long("device")
                .value_name("string")
                .required(true)
                .help("the device name to label this key with"),
            use_flag(),
        ],
        "recipient remove" => vec![
            Arg::new("reencrypt")
                .long("reencrypt")
                .action(ArgAction::SetTrue)
                .help("re-encrypt every entry to the reduced recipient list (required)"),
            use_flag(),
        ],
        _ => vec![use_flag()],
    }
}

pub fn add(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    let pubkey = arg(matches, "pubkey");
    let device = matches
        .get_one::<String>("device")
        .map(String::as_str)
        .expect("clap enforces --device");
    let prompter = keys.prompter();
    let change = keys.with_vault(matches, io, |_, vault, identity| {
        Ok(vault.add_recipient(device, pubkey, identity, prompter)?)
    })?;
    out(io, &change_lines("added", &change));
    Ok(())
}

pub fn remove(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    // Checked here as well as in the library, so the refusal arrives
    // before the passphrase prompt: asked to unlock, then told the command
    // line was wrong, is the wrong order to fail in.
    if !matches.get_flag("reencrypt") {
        return Err(fail(
            Code::Usage,
            "removing a recipient requires --reencrypt, which rewrites every entry so the removed key stops opening them",
        ));
    }
    let query = arg(matches, "pubkey-or-name");
    let prompter = keys.prompter();
    let change = keys.with_vault(matches, io, |_, vault, identity| {
        Ok(vault.remove_recipient(query, true, identity, prompter)?)
    })?;
    out(io, &change_lines("removed", &change));
    Ok(())
}

/// What a completed add or remove did (Go's `recipientChangeLines`). The
/// count line is unconditional: both verbs always re-encrypt. It says the
/// commit is local on purpose: the guarantee is about HEAD, while
/// publishing is the ordinary post-write push, which warns and carries on
/// when the remote is unreachable. "Committed locally" keeps a failed push
/// legible as a publishing problem rather than a half-done migration.
fn change_lines(verb: &str, change: &RecipientChange) -> Vec<String> {
    let noun = if change.reencrypted == 1 {
        "entry"
    } else {
        "entries"
    };
    vec![
        format!(
            "gage: {verb} recipient {:?} ({})",
            change.device, change.pubkey
        ),
        format!(
            "gage: re-encrypted {} {noun}; committed locally as {}",
            change.reencrypted,
            short_hash(&change.commit)
        ),
    ]
}

/// A commit abbreviated for people, leaving anything shorter than that
/// alone.
pub(crate) fn short_hash(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
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
