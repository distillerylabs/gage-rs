//! `gage identity enroll`: `identity add` plus publishing a request to join.
//! What it does lives in the library (`Vault::enroll`); this resolves the
//! device name and TTL, records the device, method and key in global config
//! once the request has reached the remote, and renders. `clone`'s offer
//! calls [`enroll_device`] too, so the two cannot drift.

use std::time::Duration;

use crate::exitcode::ExitCode;
use clap::{Arg, ArgMatches};
use gage::enrollment::{DEFAULT_ENROLLMENT_TTL, EnrollmentRequest, MAX_ENROLLMENT_TTL};
use gage::prompter::Prompter;
use gage::vault::Vault;
use jiff::Timestamp;

use super::entry::use_flag;
use super::identity::record_local_identity;
use super::init::resolve_device_name;
use super::sync::with_resolved_vault;
use super::{Keys, fail, out};
use crate::Io;
use crate::duration::{self, DurationError};
use crate::error::CliError;

pub const LONG_ABOUT: &str = "Register this device's identity and publish a request to join.

Creates a local identity for this vault if there isn't one already (reusing
it if there is), seals this device's name and public key to a freshly
generated enrollment code, commits the result, pushes it, and prints the
code. Give the code to someone who can already read the vault, over a
channel where they can tell it came from you.

Requires git write access. It fetches before it commits and fails outright
if the remote can't be reached — unlike a read, an enrollment request that
was never published accomplishes nothing.";

pub fn flags() -> Vec<Arg> {
    vec![
        Arg::new("device")
            .long("device")
            .value_name("string")
            .help("this device's name for the vault (default: normalized hostname)"),
        Arg::new("ttl")
            .long("ttl")
            .value_name("duration")
            .help(format!(
                "how long the request stays open (max {}) (default {})",
                duration::format(MAX_ENROLLMENT_TTL),
                duration::format(DEFAULT_ENROLLMENT_TTL)
            )),
        use_flag(),
    ]
}

pub fn run(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    let prompter = keys.prompter();
    // Never unlocked: a joining device cannot open this vault, which is
    // the whole reason it is enrolling.
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
        let ttl = match matches.get_one::<String>("ttl") {
            Some(typed) => parse_ttl(typed)?,
            None => DEFAULT_ENROLLMENT_TTL,
        };
        enroll_device(io, vault, &device, ttl, prompter)
    })
}

fn parse_ttl(typed: &str) -> Result<Duration, CliError> {
    duration::parse(typed).map_err(|e| {
        let why = match e {
            DurationError::Malformed => "not a duration such as 24h or 90m",
            DurationError::Negative => "a request cannot live for a negative time",
        };
        fail(ExitCode::Usage, format!("invalid --ttl {typed:?}: {why}"))
    })
}

/// The whole of `identity enroll` below flag parsing.
pub(super) fn enroll_device(
    io: &mut Io,
    vault: &Vault,
    device: &str,
    ttl: Duration,
    prompter: &dyn Prompter,
) -> Result<(), CliError> {
    let req = vault.enroll(device, ttl, prompter)?;

    // A success that did no work: the recorded key is already a
    // recipient. Nothing was generated, so there is no key to record.
    if !req.published {
        out(
            io,
            &[
                format!(
                    "gage: this device is already a recipient of {:?}, so it can read the vault already.",
                    vault.name
                ),
                "gage: nothing was published.".to_string(),
            ],
        );
        return Ok(());
    }

    // After the push, so a registration only ever describes a request that
    // reached the remote.
    record_local_identity(io, &vault.name, device, &req.method, &req.pubkey)?;
    out(io, &enrollment_lines(&vault.name, &req));
    Ok(())
}

/// A published request. The code is labelled at its point of use, as the
/// passphrase prompt before it is ("it stays on this device"): the two
/// are opposites on one screen, and the failure to design against is
/// someone pasting their identity passphrase into a chat window because
/// they took it for the code ("Which secret is which").
fn enrollment_lines(vault: &str, req: &EnrollmentRequest) -> Vec<String> {
    let remaining = (req.expires.as_second() - Timestamp::now().as_second()).max(0) as u64;
    vec![
        format!(
            "gage: this device is {:?}, public key {}",
            req.device, req.pubkey
        ),
        String::new(),
        format!(
            "gage: enrollment request published (expires in {}).",
            human_duration(Duration::from_secs(remaining))
        ),
        String::new(),
        format!("  Enrollment code:  {}", &*req.code),
        String::new(),
        "gage: this code is safe to send and is not your passphrase. Give it to someone who"
            .to_string(),
        format!(
            "gage: can already read {vault:?}, over a channel where they can tell it came from"
        ),
        "gage: you — not through this vault's remote. They run:".to_string(),
        format!("gage:   gage recipient approve --code {}", &*req.code),
        String::new(),
        "gage: then run `gage sync` here.".to_string(),
    ]
}

/// A request's remaining lifetime the way a person would say it ("24h",
/// "2h30m") rather than as `24h0m0s`, which reads like a machine
/// timestamp in a line whose job is to say how long someone has.
pub(crate) fn human_duration(d: Duration) -> String {
    let minutes_total = (d.as_secs() + 30) / 60;
    let (hours, minutes) = (minutes_total / 60, minutes_total % 60);
    match (hours, minutes) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h{m}m"),
    }
}
