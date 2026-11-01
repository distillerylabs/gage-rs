//! `gage init`: resolves the defaults (device name, target path), refuses
//! what is knowable from the flags, generates a recovery key, and hands
//! the rest to `Vault::init`, which creates this device's identity (the
//! passphrase prompt), creates the vault and registers it, removing the
//! identity again if the vault can't be created (ADR-0061). Only once the
//! vault is registered is the recovery key delivered; it was never shown
//! or stored before then, so it needs no rollback.
//!
//! `--remote` sets origin and publishes the new vault at once, asking for
//! the host's token first if none is stored (#38).

use std::path::{Path, PathBuf};

use crate::exitcode::ExitCode;
use clap::{Arg, ArgAction, ArgMatches};
use gage::HasKind;
use gage::Prompter;
use gage::recovery::{RECOVERY_DEVICE_LABEL, check_device_label_free, new_recovery_key};
use gage::storage::config::RegistryError;
use gage::sync::syncerr::Kind;
use gage::vault::Initialized;
use gage::{InitSpec, Vault};
use gage::{UnlockMethod, VaultType};
use gage::{devicename, storage::paths, sync::remoteauth};

use super::recoverykey::{
    KeyPlan, check_recovery_key_out_path, deliver_recovery_key, plan_recovery_key,
};
use super::{arg, cwd, dirs, fail, load_global, out};
use crate::Io;
use crate::error::{CliError, prefixed};

pub const LONG_ABOUT: &str = "Create a new vault.

Generates this device's identity, wraps its private key with a passphrase
you choose, and writes the public half into the vault's recipient files.
The wrapped key is stored outside the vault and is never committed or synced.

A vault that depends on one identity file surviving forever has no recovery
story, so init also generates an offline recovery key and shows it once. It is
not encrypted and gage keeps no copy: write it down, store it offline, and
treat the paper as you would the secrets it opens. --no-recovery-key skips it;
--recovery-key-out writes it to a file instead of the screen.

--recipient is additive: each one is written alongside this device's own key,
for recipients you already hold public keys for.

--remote publishes the new vault immediately. If its host needs a token and
none is stored yet, init asks for one on the spot (leave blank to skip and
try anonymously) — the same prompt as `gage auth login`, so a private repo
doesn't need a separate auth login round trip before the first push succeeds.";

/// `init`'s default method, as a static so clap can hold its name.
static DEFAULT_METHOD: UnlockMethod = UnlockMethod::Passphrase;

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
        Arg::new("method")
            .long("method")
            .value_name("METHOD")
            .default_value(DEFAULT_METHOD.as_str())
            .help(format!(
                "default identity method for devices joining this vault (accepted: [{}])",
                UnlockMethod::names(" ")
            )),
        Arg::new("no-recovery-key")
            .long("no-recovery-key")
            .action(ArgAction::SetTrue)
            .help("don't generate an offline recovery key (leaves this device's identity file the only way in)"),
        Arg::new("recipient")
            .long("recipient")
            .value_name("KEY")
            .action(ArgAction::Append)
            .help("additional recipient public key (repeatable; this device's own key is always included)"),
        Arg::new("recovery-key-out")
            .long("recovery-key-out")
            .value_name("FILE")
            .help("write the recovery key to this file (mode 0600) instead of showing it"),
        Arg::new("remote")
            .long("remote")
            .value_name("URL")
            .help("git remote (origin) URL; omit to start local-only"),
        Arg::new("type")
            .long("type")
            .value_name("TYPE")
            .default_value(VaultType::default().as_str())
            .help(format!("vault type (accepted: [{}])", VaultType::names(" "))),
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

    // Settled before anything is created, with the other knowable
    // failures: a default run with no terminal to show a key on costs
    // nobody a passphrase.
    let mut plan = plan_recovery_key(
        io,
        Some("--no-recovery-key"),
        matches.get_flag("no-recovery-key"),
        flag("recovery-key-out").as_deref(),
    )?;
    // Reserved whether or not this run makes a key: a vault made with
    // --no-recovery-key can grow one through `recovery rotate`, which
    // replaces whatever holds the label, so a device there would be
    // evicted by its own rotation. Create refuses it too; this answers
    // before the registry is even read.
    check_device_label_free(&device)?;

    let dirs = dirs(io)?;
    // Vault::init refuses a registered name too, and its check is the one
    // that holds. This one runs first only so that a registered name is
    // still reported ahead of a bad --recovery-key-out, below.
    let (_, global) = load_global(io)?;
    if global.vaults.contains_key(name) {
        return Err(RegistryError::AlreadyRegistered(name.to_string()).into());
    }
    let path = match flag("dir") {
        Some(dir) => PathBuf::from(dir),
        None => paths::vaults_dir(&dirs).join(name),
    };
    // Here rather than with the flags: whether a destination is safe
    // depends on where this vault is going. Still before anything is
    // created, which is what matters: a destination that can't be
    // written, or that gage would delete, must not be found out after
    // there is a key riding on it.
    if let KeyPlan::File(out) = &mut plan {
        *out = check_recovery_key_out_path(out, &cwd()?, &path, &dirs)?;
    }
    let vault_type = flag("type").expect("defaulted").parse::<VaultType>()?;

    // Generated before the vault, but not shown until it exists and is
    // registered. Nothing needs rolling back if anything below fails: it
    // was never written down, stored or seen. Only its public key goes to
    // the library.
    let recovery_key = match plan {
        KeyPlan::None => None,
        KeyPlan::Show | KeyPlan::File(_) => Some(new_recovery_key()?),
    };
    let spec = InitSpec {
        name: name.to_string(),
        path,
        vault_type,
        method: UnlockMethod::from(flag("method").expect("defaulted")),
        device,
        recipients: extra_recipients,
        recovery_key: recovery_key.as_ref().map(|key| key.pubkey.clone()),
        remote: flag("remote"),
    };
    // Validates, asks for the new passphrase, creates and registers, and
    // removes the identity again if the vault can't be created.
    let Initialized {
        vault: created,
        pubkey,
    } = Vault::init(&dirs, &spec, prompter)?;

    // After registration, so a key is never shown for a vault that isn't
    // usable yet, and before the summary, so the one thing needing action
    // isn't what scrolled off. Its error is held: the vault is created,
    // committed and registered, and returning now would skip the summary
    // and the push, leaving a vault that exists and was never published
    // with nothing on screen saying so. Everything init was asked to do
    // still happens, and the command then exits on this.
    let delivered = match &recovery_key {
        Some(key) => deliver_recovery_key(io, prompter, &plan, name, key),
        None => Ok(()),
    };

    let mut summary = vec![
        format!(
            "gage: initialized vault {name:?} at {}",
            created.path.display()
        ),
        format!(
            "gage: this device is {:?}, public key {pubkey}",
            spec.device
        ),
    ];
    if let Some(key) = &recovery_key {
        summary.push(format!(
            "gage: recovery key is {RECOVERY_DEVICE_LABEL:?}, public key {}",
            key.pubkey
        ));
    }
    out(io, &summary);
    let published = match &spec.remote {
        Some(remote) => publish(io, prompter, &created, remote),
        None => Ok(()),
    };
    match (delivered, published) {
        // Both failed. The key is the worse of the two: an unpublished
        // vault can be pushed again, a key nobody has can't be reissued.
        // So it sets the exit code, and the push failure is reported
        // beside it.
        (Err(delivery), Err(push)) => {
            let _ = writeln!(io.stderr, "{}", prefixed(&push));
            Err(delivery)
        }
        (Err(e), Ok(())) | (Ok(()), Err(e)) => Err(e),
        (Ok(()), Ok(())) => Ok(()),
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
    // been made yet (for a sync, the same path is
    // an unmounted drive, ADR-0017).
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
        ExitCode::LockedOrAuth
    } else {
        ExitCode::from(err.kind())
    };
    // The failure's own remedy (a login, for a refused token) stays with
    // the failure, ahead of the line about creating the repository.
    let remedy = err
        .remedy()
        .map(|r| crate::remedy::render(&r))
        .unwrap_or_default();
    Err(fail(
        code,
        format!(
            "{name:?} was created locally, but publishing it to {remote} failed: {err}{remedy}\n\
             gage: gage never creates a repository for you — create an empty one there, then run `gage push`"
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
        .map_err(|e| fail(ExitCode::LockedOrAuth, e))?;
    if token.trim().is_empty() {
        return Ok(());
    }
    remoteauth::save(&dirs, &host, &token)?;
    out(io, &[format!("gage: stored a token for {host}")]);
    Ok(())
}

/// `--device` if given, or else this machine's normalized hostname, by
/// [`resolve_device_name`]'s rules.
pub(super) fn device_flag_or_hostname(matches: &ArgMatches, io: &Io) -> Result<String, CliError> {
    match matches.get_one::<String>("device") {
        Some(explicit) => resolve_device_name(Some(explicit), ""),
        None => {
            let hostname = (io.hostname)()
                .map_err(|e| CliError::internal(anyhow::anyhow!("reading the hostname: {e}")))?;
            resolve_device_name(None, &hostname)
        }
    }
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
                ExitCode::Usage,
                format!("--device {explicit:?} is invalid"),
            ))
        };
    }
    devicename::normalize(hostname).ok_or_else(|| {
        fail(
            ExitCode::Usage,
            format!(
                "this machine's hostname ({hostname:?}) doesn't normalize to a usable device name; pass --device NAME"
            ),
        )
    })
}
