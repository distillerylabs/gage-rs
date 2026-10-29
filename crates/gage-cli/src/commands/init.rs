//! `gage init`: resolves the defaults (device name, target path), refuses
//! what is knowable from the flags, generates this device's identity (the
//! passphrase prompt) and a recovery key, creates the vault through the
//! library with both, and only once it exists registers it in global
//! config and delivers the recovery key. If creating the vault fails, the
//! identity is removed again; the recovery key was never shown or stored,
//! so it needs no rollback.
//!
//! `--remote` sets origin and publishes the new vault at once, asking for
//! the host's token first if none is stored (#38).

use std::path::{Path, PathBuf};

use clap::{Arg, ArgAction, ArgMatches};
use gage::config::{GitMeta, RegistryError, VaultEntry};
use gage::exitcode::{Code, HasCode};
use gage::prompter::Prompter;
use gage::recovery::{RECOVERY_DEVICE_LABEL, check_device_label_free, new_recovery_key};
use gage::syncerr::Kind;
use gage::vault::{CreateSpec, LabelledRecipient, Vault};
use gage::vaultconfig::{UnlockMethod, VaultId, VaultType};
use gage::{devicename, identityfile, paths, remoteauth};

use super::recoverykey::{
    KeyPlan, check_recovery_key_out_path, deliver_recovery_key, plan_recovery_key,
};
use super::{arg, cwd, dirs, fail, load_global, out};
use crate::Io;
use crate::error::{CliError, prefix_once};

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
    let (config_path, mut global) = load_global(io)?;
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

    // The id is minted here, before the identity, and handed to create
    // rather than minted there: the identity file is filed under it, and
    // an id that only existed after the file was written would leave the
    // vault carrying one id and this device's only key filed under
    // another, with nothing visibly wrong (ADR-0006).
    let mut spec = CreateSpec {
        name: name.to_string(),
        id: VaultId::new(),
        path,
        vault_type: flag("type").expect("defaulted").parse::<VaultType>()?,
        method: UnlockMethod::from(flag("method").expect("defaulted")),
        device,
        recipients: extra_recipients,
        extra_recipients: Vec::new(),
        remote: flag("remote"),
    };
    // Everything knowable from the flags fails before a human types a
    // passphrase twice. The target directory is checked by
    // create, after the key exists; the rollback below covers
    // it.
    spec.validate_without_device_key()?;

    let pubkey = identityfile::create(&dirs, &spec.id, name, &spec.device, prompter)?;
    // This device's key leads and --recipient keys follow in the order
    // given: create labels the first recipient with the device name.
    spec.recipients.insert(0, pubkey.clone());

    // Generated here, after the identity, but not shown until the vault
    // exists. Nothing needs rolling back if anything below fails: it was
    // never written down, stored or seen.
    let recovery_key = match plan {
        KeyPlan::None => None,
        KeyPlan::Show | KeyPlan::File(_) => Some(new_recovery_key()?),
    };
    if let Some(key) = &recovery_key {
        spec.extra_recipients = vec![LabelledRecipient {
            device: RECOVERY_DEVICE_LABEL.into(),
            pubkey: key.pubkey.clone(),
        }];
    }

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
            id: Some(created.id.clone()),
            vault_type: spec.vault_type,
            device: spec.device.clone(),
            // Recorded now, the one moment gage holds this device's key
            // without an unlock, so "is my key still a recipient?" can be
            // answered by comparing keys (ADR-0013).
            pubkey: Some(pubkey.clone()),
            method: spec.method.clone(),
            git: GitMeta {
                origin: spec.remote.clone(),
            },
        },
    )?;
    gage::config::save(&config_path, &global)?;

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
            let _ = writeln!(io.stderr, "{}", prefix_once(&push.to_string()));
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
