//! `gage recovery`: the vault's offline recovery key. `rotate` replaces
//! it, or adds the first one, from an unlocked device; the swap is the
//! library's (`Vault::rotate_recovery_key`), and the new key is delivered
//! exactly as `init` delivers one (`recoverykey`).

use clap::{Arg, ArgMatches};
use gage::recovery::{RotateResult, new_recovery_key};

use super::entry::{open_vault, use_flag};
use super::recoverykey::{
    KeyPlan, check_recovery_key_out_path, deliver_recovery_key, plan_recovery_key,
};
use super::{Keys, cwd, dirs, out};
use crate::Io;
use crate::error::CliError;

pub const ROTATE_LONG_ABOUT: &str = "Replace this vault's recovery key, or add one if it has none.

Generates a new recovery key, registers it as the vault's recovery-paper-key,
and retires the old one, re-encrypting every entry in a single commit. The new
key is shown once, exactly as `gage init` shows one; --recovery-key-out writes
it to a file instead of the screen.

Use it if a recovery key may have leaked. Retiring it stops it opening anything
written from now on, but it still opens every version already in the vault's
git history, so rotate the secrets themselves too.

A vault made with --no-recovery-key has nothing to retire; this simply adds one.";

pub fn flags(name: &str) -> Vec<Arg> {
    match name {
        "recovery rotate" => vec![
            Arg::new("recovery-key-out")
                .long("recovery-key-out")
                .value_name("FILE")
                .help("write the new recovery key to this file (mode 0600) instead of showing it"),
            use_flag(),
        ],
        _ => Vec::new(),
    }
}

pub fn rotate(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    // Before the unlock, so a run with nowhere to show the key never asks
    // for a passphrase. No opt-out to name: producing a key is the whole
    // command.
    let mut plan = plan_recovery_key(
        io,
        None,
        false,
        matches
            .get_one::<String>("recovery-key-out")
            .map(String::as_str),
    )?;
    // Before the unlock too (R5's review): knowable from the path alone,
    // and found after the passphrase and a sync it charges the user twice
    // for one mistake. The vault is resolved without unlocking to learn
    // where it is.
    if let KeyPlan::File(dest) = &mut plan {
        let vault = open_vault(matches, io)?;
        *dest = check_recovery_key_out_path(dest, &cwd()?, &vault.path, &dirs(io)?)?;
    }

    let prompter = keys.prompter();
    let (name, result, key) = keys.with_vault(matches, io, |_, vault, identity| {
        let key = new_recovery_key()?;
        let result = vault.rotate_recovery_key(&key.pubkey, identity, prompter)?;
        Ok((vault.name.clone(), result, key))
    })?;

    // Held, as init's is: the vault has changed and been pushed, so the
    // summary still has to say so.
    let delivered = deliver_recovery_key(io, prompter, &plan, &name, &key);
    out(io, &rotate_lines(&name, &result));
    delivered
}

fn rotate_lines(vault: &str, res: &RotateResult) -> Vec<String> {
    let first = match &res.retired {
        None => format!("gage: added a recovery key to vault {vault:?} (it had none)"),
        Some(retired) => {
            format!("gage: replaced vault {vault:?}'s recovery key; retired {retired}")
        }
    };
    let noun = if res.reencrypted == 1 {
        "entry"
    } else {
        "entries"
    };
    vec![
        first,
        format!("gage: new recovery key is {}", res.pubkey),
        format!("gage: re-encrypted {} {noun}", res.reencrypted),
    ]
}
