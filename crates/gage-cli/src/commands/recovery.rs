//! `gage recovery`: the vault's offline recovery key. `verify` checks a
//! stored copy without unlocking anything. `enroll` spends it to get a
//! device back, retiring it in the same commit (`Vault::recover_device`).
//! `rotate` replaces it, or adds the first one, from an unlocked device
//! (`Vault::rotate_recovery_key`). A new key is delivered exactly as
//! `init` delivers one (`recoverykey`).

use std::path::PathBuf;

use crate::exitcode::ExitCode;
use clap::{Arg, ArgAction, ArgMatches};
use gage::error::{ErrorKind, HasKind};
use gage::identityfile::{self, IdentityBackup};
use gage::prompter::Prompter;
use gage::recipient::RecipientError;
use gage::recovery::{
    RECOVERY_DEVICE_LABEL, RecoverResult, RecoverSpec, RecoveryError, RotateResult,
    check_device_label_free, new_recovery_key,
};
use gage::vault::Vault;
use gage::vaultconfig::UnlockMethod;

use super::entry::{open_vault, use_flag};
use super::identity::record_local_identity;
use super::init::device_flag_or_hostname;
use super::recoverykey::{
    KeyPlan, check_recovery_key_out_path, deliver_recovery_key, plan_recovery_key,
};
use super::{Keys, cwd, dirs, fail, out};
use crate::Io;
use crate::error::{CliError, prefixed};

pub const ROTATE_LONG_ABOUT: &str = "Replace this vault's recovery key, or add one if it has none.

Generates a new recovery key, registers it as the vault's recovery-paper-key,
and retires the old one, re-encrypting every entry in a single commit. The new
key is shown once, exactly as `gage init` shows one; --recovery-key-out writes
it to a file instead of the screen.

Use it if a recovery key may have leaked. Retiring it stops it opening anything
written from now on, but it still opens every version already in the vault's
git history, so rotate the secrets themselves too.

A vault made with --no-recovery-key has nothing to retire; this simply adds one.";

pub const VERIFY_LONG_ABOUT: &str =
    "Check a stored recovery key is still one of this vault's recipients.

Paste the key you stored offline (it is read without echo, and neither shown
nor kept) and gage checks that it is one of this vault's recipients. That
proves the copy you hold is intact and belongs to this vault. It does not
decrypt anything, so it cannot tell you the vault's entries are undamaged.

It also says whether `gage recovery enroll` will accept the key, which is a
narrower question: only the key registered as the vault's recovery key can
enroll a device, even though any recipient's key decrypts the entries.

It needs no passphrase: it never touches this device's identity file.";

pub const ENROLL_LONG_ABOUT: &str = "Use a recovery key to enroll a fresh identity for this device.

For the machine whose identity file is gone, or whose passphrase is. Paste the
recovery key you stored offline and gage generates a fresh identity for this
device, admits it to the vault, and retires the key you just used — all in one
commit. The key is retired because it has now been typed into a terminal, and
it reads everything.

A replacement recovery key is generated and shown once, so the vault is never
left without a way back. --no-new-recovery-key skips it; --recovery-key-out
writes it to a file instead of the screen.

--replaces names the device that was lost, removing its key in the same commit
so a stolen identity file stops opening anything new. Naming the device being
enrolled is how you rebuild a machine under the name it already had, and the
only case in which this replaces a local identity file rather than refusing.";

pub fn flags(name: &str) -> Vec<Arg> {
    match name {
        "recovery enroll" => vec![
            Arg::new("device")
                .long("device")
                .value_name("NAME")
                .help("this device's name for the vault (default: normalized hostname)"),
            Arg::new("no-new-recovery-key")
                .long("no-new-recovery-key")
                .action(ArgAction::SetTrue)
                .help("don't generate a replacement recovery key (leaves the vault without one)"),
            Arg::new("recovery-key-out")
                .long("recovery-key-out")
                .value_name("FILE")
                .help("write the replacement recovery key to this file (mode 0600) instead of showing it"),
            Arg::new("replaces")
                .long("replaces")
                .value_name("NAME")
                .help("device whose key to remove in the same commit (the one that was lost)"),
            use_flag(),
        ],
        "recovery rotate" => vec![
            Arg::new("recovery-key-out")
                .long("recovery-key-out")
                .value_name("FILE")
                .help("write the new recovery key to this file (mode 0600) instead of showing it"),
            use_flag(),
        ],
        "recovery verify" => vec![use_flag()],
        _ => Vec::new(),
    }
}

/// `recovery verify`: checks a pasted key against the plaintext recipient
/// list. No unlock and no lock, and that is the point: a recovery key is
/// what you reach for when this device's identity file is gone, so the
/// check can't depend on it.
pub fn verify(matches: &ArgMatches, io: &mut Io, prompter: &dyn Prompter) -> Result<(), CliError> {
    let vault = open_vault(matches, io)?;
    // Masked, and dropped (so zeroed) when this returns; never echoed.
    let pasted = prompter
        .value("Paste the recovery key to check")
        .map_err(|e| fail(ExitCode::LockedOrAuth, e))?;
    let label = vault.verify_recovery_key(&pasted)?;
    drop(pasted);

    // Two claims, reported separately, because only one of them is what
    // `recovery enroll` needs. A key under another label really does
    // decrypt this vault (`age -d -i` opens every entry with it), so
    // calling that a failure would be false. What it can't do is enroll a
    // device, and verify is the check the docs tell people to trust, so it
    // must not pass a key enroll will refuse.
    let name = &vault.name;
    let mut lines = vec![format!(
        "gage: that key is a recipient of vault {name:?}, registered as {label:?}"
    )];
    if label == RECOVERY_DEVICE_LABEL {
        lines.push(
            "gage: it can decrypt this vault, and `gage recovery enroll` will accept it".into(),
        );
    } else {
        lines.push(
            "gage: it can decrypt this vault, but `gage recovery enroll` will not accept it —"
                .into(),
        );
        lines.push(format!(
            "gage: that needs the key registered as {RECOVERY_DEVICE_LABEL:?}."
        ));
    }
    out(io, &lines);
    Ok(())
}

/// `recovery enroll` refusing to replace a wrapped identity this machine
/// already holds for the device it was told to enroll, unless `--replaces`
/// names that same device.
///
/// A frontend policy, so it lives here: `identityfile::create` would
/// happily *reuse* the file, asking for its passphrase, and a forgotten
/// passphrase is one of the two things this command exists to route
/// around. Without `--replaces`, the file is taken to be a different
/// machine's key, and enrolling over it would strand a vault this machine
/// can still open.
#[derive(Debug, thiserror::Error)]
#[error(
    "this machine already holds an identity for that device: {} already exists.\n\
     gage: if this machine is being rebuilt under that name, say so with --replaces {device};\n\
     gage: otherwise enroll under a different --device NAME",
    path.display()
)]
pub struct LocalIdentityExists {
    pub path: PathBuf,
    pub device: String,
}

impl HasKind for LocalIdentityExists {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Usage
    }
}

/// A name the vault already lists, refused before the paste: the
/// recipient verbs' own `RecipientExists`, as its source so a caller still
/// matches it, with the two ways out.
#[derive(Debug, thiserror::Error)]
#[error(
    "{source}.\n\
     gage: if this machine is being rebuilt under that name, say so with --replaces {device};\n\
     gage: otherwise enroll under a different --device NAME"
)]
struct DeviceNameTaken {
    device: String,
    source: RecipientError,
}

impl HasKind for DeviceNameTaken {
    fn kind(&self) -> ErrorKind {
        self.source.kind()
    }
}

/// `recovery enroll`: spends the recovery key to get a working device
/// back. Ordered so nothing irreversible happens before it must: every
/// refusal that needs no secret, then the paste, then the new passphrase.
pub fn enroll(matches: &ArgMatches, io: &mut Io, prompter: &dyn Prompter) -> Result<(), CliError> {
    let flag = |name| matches.get_one::<String>(name).map(String::as_str);

    // 1. Everything knowable without a secret.
    let vault = open_vault(matches, io)?;
    let device = device_flag_or_hostname(matches, io)?;
    let replaces = flag("replaces");
    // The replacement key first, before a single prompt: a run with
    // nowhere to show it must not get as far as asking for the old one.
    let mut plan = plan_recovery_key(
        io,
        Some("--no-new-recovery-key"),
        matches.get_flag("no-new-recovery-key"),
        flag("recovery-key-out"),
    )?;
    if let KeyPlan::File(dest) = &mut plan {
        *dest = check_recovery_key_out_path(dest, &cwd()?, &vault.path, &dirs(io)?)?;
    }
    // The reservation before the collision check, so its own reason is
    // what the user sees: the collision would otherwise suggest
    // `--replaces recovery-paper-key`, which the library then refuses.
    check_device_label_free(&device)?;
    // The used key is retired whether or not it is named. Knowable from
    // the flags, so refused here rather than by the swap after a new
    // passphrase; the swap still refuses it too.
    if replaces == Some(RECOVERY_DEVICE_LABEL) {
        return Err(RecoveryError::ReplacesRecoveryKey.into());
    }
    check_device_name_free(&vault, &device, replaces)?;
    // Held only when --replaces says this device is being rebuilt;
    // otherwise its presence is the refusal.
    let backup = identityfile::backup(&dirs(io)?, &vault.id, &device)?;
    if let Some(found) = &backup
        && replaces != Some(device.as_str())
    {
        return Err(LocalIdentityExists {
            path: found.path().to_path_buf(),
            device,
        }
        .into());
    }

    // 2. The paste, checked before the new passphrase is chosen: a wrong
    // key is the likeliest mistake here, and must not cost a passphrase
    // entry first. `recover_device` checks it again for real; this is only
    // about the order of the questions.
    let pasted = prompter
        .value(&format!("Paste the recovery key for {}", vault.name))
        .map_err(|e| fail(ExitCode::LockedOrAuth, e))?;
    check_pasted_recovery_key(&vault, &pasted)?;

    // 3. The replacement, before anything is written: generating it can't
    // be undone by anything later, but it touches nothing either.
    let new_key = match plan {
        KeyPlan::None => None,
        _ => Some(new_recovery_key()?),
    };

    // 4. The device identity. The displaced file goes first, so `create`
    // generates a fresh key instead of reusing it and asking for the
    // passphrase nobody has.
    let dirs = dirs(io)?;
    if backup.is_some() {
        identityfile::remove(&dirs, &vault.id, &device)?;
    }
    let pubkey = match identityfile::create(&dirs, &vault.id, &vault.name, &device, prompter) {
        Ok(pubkey) => pubkey,
        Err(e) => return Err(restore_after(backup.as_ref(), e.into())),
    };

    // 5. The swap. On failure it committed nothing, so the identity just
    // generated is referenced by nothing: it goes, and whatever it
    // displaced comes back exactly as it was. Same reasoning as init's
    // rollback.
    let spec = RecoverSpec {
        device: device.clone(),
        pubkey: pubkey.clone(),
        replaces: replaces.map(str::to_string),
        new_recovery_pubkey: new_key.as_ref().map(|k| k.pubkey.clone()),
    };
    let res = match vault.recover_device(&spec, pasted, prompter) {
        Ok(res) => res,
        Err(e) => {
            let err = CliError::from(e);
            if let Err(rollback) = identityfile::remove(&dirs, &vault.id, &device) {
                return Err(fail(
                    ExitCode::Internal,
                    format!("{err} (and rolling back the generated identity failed: {rollback})"),
                ));
            }
            return Err(restore_after(backup.as_ref(), err));
        }
    };
    // Committed and pushed: the file it replaced is no one's now.
    drop(backup);

    // 6. The replacement key goes out first, ahead of everything that can
    // still fail: it is already the vault's, and its private half exists
    // only in this process, so anything that errored before this would
    // take it with it. Its own error is held, as init holds it.
    let delivered = match &new_key {
        Some(key) => deliver_recovery_key(io, prompter, &plan, &vault.name, key),
        None => Ok(()),
    };
    drop(new_key);

    // 7. Now the local bookkeeping. A failure here is real but
    // recoverable (the identity file exists and the vault lists it), and
    // it must not be what swallows the key above.
    let recorded = record_local_identity(
        io,
        &vault.name,
        &device,
        &UnlockMethod::Passphrase,
        &pubkey,
    )
    // The remedy after the cause, on its own line, keeping the
    // cause's code, however the failure ends up reported.
    .map_err(|e| {
        fail(
            e.code(),
            format!(
                "{e}\ngage: the vault change is committed and {device:?} is a recipient; re-run \
                         `gage identity add --device {device}` to register it on this machine"
            ),
        )
    });

    // 8. What happened, then whichever error matters most.
    out(io, &enroll_lines(&vault.name, &res));
    if res.new_recovery_pubkey.is_none() {
        warn_no_recovery_key(io, &vault.name);
    }
    if let Err(e) = delivered {
        // The lost key is the worse of the two, so it sets the exit code;
        // the other is reported beside it.
        if let Err(record) = recorded {
            let _ = writeln!(io.stderr, "{}", prefixed(&record));
        }
        return Err(e);
    }
    recorded
}

/// Puts back the identity file `recovery enroll` displaced, if it
/// displaced one, and returns `cause`: every failure after the file was
/// cleared goes through here. A restore that fails too is reported with
/// both, and where the file belonged, since its bytes are then gone.
fn restore_after(backup: Option<&IdentityBackup>, cause: CliError) -> CliError {
    let Some(backup) = backup else {
        return cause;
    };
    match backup.restore() {
        Ok(()) => cause,
        Err(e) => fail(
            ExitCode::Internal,
            format!(
                "{cause} (and restoring the identity file at {} failed: {e})",
                backup.path().display()
            ),
        ),
    }
}

fn enroll_lines(vault: &str, res: &RecoverResult) -> Vec<String> {
    let mut lines = vec![
        format!(
            "gage: enrolled {:?} in vault {vault:?}, public key {}",
            res.device, res.pubkey
        ),
        format!(
            "gage: retired the recovery key you used ({})",
            res.retired_recovery
        ),
    ];
    if let Some(replaced) = &res.replaced {
        lines.push(format!(
            "gage: removed {replaced:?} from the recipient list"
        ));
    }
    if let Some(pubkey) = &res.new_recovery_pubkey {
        lines.push(format!("gage: new recovery key is {pubkey}"));
    }
    let noun = if res.reencrypted == 1 {
        "entry"
    } else {
        "entries"
    };
    lines.push(format!("gage: re-encrypted {} {noun}", res.reencrypted));
    lines
}

/// Said on stderr, beside the summary: the vault now has exactly the
/// exposure the recovery key exists to remove.
fn warn_no_recovery_key(io: &mut Io, vault: &str) {
    let _ = writeln!(
        io.stderr,
        "gage: {vault:?} now has no recovery key. This device's identity file is the only\n\
         gage: way in; if it or its passphrase is lost, the vault is unreadable forever.\n\
         gage: `gage recovery rotate` adds one."
    );
}

/// The early half of the two-stage check on the pasted key. It must be a
/// recipient, and it must be *the recovery recipient*: any recipient's key
/// decrypts the vault, but only the one under the fixed label may enroll a
/// device.
fn check_pasted_recovery_key(vault: &Vault, pasted: &str) -> Result<(), CliError> {
    let label = vault.verify_recovery_key(pasted)?;
    if label != RECOVERY_DEVICE_LABEL {
        return Err(fail(
            ExitCode::LockedOrAuth,
            format!(
                "that key is registered as {label:?}, not as {RECOVERY_DEVICE_LABEL:?}, so it cannot enroll a device.\n\
                 gage: it still decrypts this vault's entries with `age -d -i`."
            ),
        ));
    }
    Ok(())
}

/// Refuses a device name the vault already lists, before anything is
/// asked. The swap would refuse it too, but only after the paste and a new
/// passphrase, for a mistake visible in a plaintext file already read.
/// The name being replaced is the exception, and the common one:
/// rebuilding a machine under the name it always had.
fn check_device_name_free(
    vault: &Vault,
    device: &str,
    replaces: Option<&str>,
) -> Result<(), CliError> {
    if replaces == Some(device) {
        return Ok(());
    }
    if vault.recipients()?.iter().any(|r| r.device == device) {
        return Err(DeviceNameTaken {
            device: device.to_string(),
            source: RecipientError::RecipientExists {
                device: device.to_string(),
                pubkey: None,
            },
        }
        .into());
    }
    Ok(())
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
