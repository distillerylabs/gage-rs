//! `gage recipient pending`, `approve` and `deny`: the approving side of
//! enrollment. What they do lives in the library (`gage::enrollment`);
//! this is rendering, gathering and ordering. The library returns every
//! question as a value, and this file asks a human and puts the answers
//! back in.
//!
//! The ordering is the part worth reading before changing anything:
//! open, then the collision check, then the render, then the confirmation,
//! and only then the unlock. The unlock comes last on purpose (see "The
//! approver unlocks, and the unlock comes late"): a wrong code, an expired
//! or malformed request, a request for another vault, or a "no" costs the
//! approver no passphrase. That is why `approve` resolves its vault
//! without unlocking and takes the identity through [`Keys::with_vault`]
//! only after the confirmation.

use std::time::Duration;

use clap::{Arg, ArgAction, ArgMatches};
use gage::devicename;
use gage::enrollment::{
    Approval, ApprovalResult, EnrollmentError, OpenedRequest, PendingRequest, RequestCandidates,
};
use gage::exitcode::{Code, HasCode};
use gage::prompter::Prompter;
use gage::vault::Vault;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use zeroize::Zeroizing;

use super::enroll::human_duration;
use super::entry::use_flag;
use super::recipient::short_hash;
use super::sync::with_resolved_vault;
use super::{Keys, arg, fail, out};
use crate::Io;
use crate::error::CliError;

pub const PENDING_LONG_ABOUT: &str = "List enrollment requests waiting to be approved.

Everything shown comes from the request's filename, so this needs no unlock,
no code and no history walk. Device names are deliberately absent: they are
under the seal, and listing them would tell every reader of the vault who is
setting up a new machine before anyone approved anything.";

pub const APPROVE_LONG_ABOUT: &str =
    "Admit an enrolling device and re-encrypt the vault to include it.

Opens each pending request with the code(s) you were given out-of-band, shows
what each one claims, and — on confirmation — adds them to the recipient list,
re-encrypts every entry so they can read the whole vault, and removes every
approved request, all in one commit.

There is no --reencrypt flag: approval always re-encrypts, because a recipient
who can read only part of a vault is a state gage has no use for. The unlock
comes after the confirmation, so a wrong code or a declined prompt costs no
passphrase. --device relabels a request whose claimed name now collides with
an existing recipient; the code authenticated the key, not the label.";

pub const DENY_LONG_ABOUT: &str = "Remove an enrollment request without granting anything.

Needs no code and no unlock: refusing to grant access requires proving
nothing, it changes no recipient list, and anyone with git write access could
delete the request directly anyway. It still takes the vault write lock and
commits, like any other write — and warns if the push fails, since a deny that
never reached the remote leaves the request live for every other device.";

/// A runaway guard on the code prompt, not a retry policy (Go's
/// `maxCodePrompts`): the loop ends when the human stops answering, which
/// at a masked prompt means a bare Enter.
const MAX_CODE_PROMPTS: usize = 100;

/// How local times render: the same instant reads differently by zone,
/// which is the point.
const TIME_FORMAT: &str = "%Y-%m-%d %H:%M %Z";

pub fn flags(name: &str) -> Vec<Arg> {
    match name {
        "recipient approve" => vec![
            Arg::new("code")
                .long("code")
                .value_name("stringArray")
                .action(ArgAction::Append)
                .help("an enrollment code you were given out-of-band (repeatable; prompted for if absent)"),
            Arg::new("device")
                .long("device")
                .value_name("string")
                .help("record the approved key under this name instead of the one the request claims"),
            use_flag(),
        ],
        _ => vec![use_flag()],
    }
}

pub fn pending(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    with_resolved_vault(matches, io, keys, |io, vault| {
        let pending = vault.pending_enrollments()?;
        let lines = pending_lines(&vault.name, &pending, &io.time_zone);
        out(io, &lines);
        Ok(())
    })
}

/// The listing: id and expiry, and nothing else there is to say without a
/// code. No "requested at": that would come from the introducing commit,
/// an O(history) walk no more trustworthy than the filename.
fn pending_lines(vault: &str, pending: &[PendingRequest], tz: &TimeZone) -> Vec<String> {
    if pending.is_empty() {
        return vec![format!(
            "gage: no pending enrollment requests for {vault:?}"
        )];
    }
    let mut lines = vec![
        format!(
            "gage: {} pending enrollment {} for {vault:?}:",
            pending.len(),
            plural(pending.len(), "request", "requests")
        ),
        String::new(),
    ];
    lines.extend(pending.iter().map(|r| {
        format!(
            "  {}   expires {}  (in {})",
            short_request_id(&r.id),
            local(r.expires, tz),
            human_duration(until(r.expires))
        )
    }));
    lines.extend([
        String::new(),
        "gage: nothing here can decrypt anything until it is approved.".to_string(),
        "gage: run `gage recipient approve --code <code>` with a code you received out-of-band."
            .to_string(),
    ]);
    lines
}

pub fn approve(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    let device = matches.get_one::<String>("device").map(String::as_str);
    // A rejection of the command line itself, so before anything is read,
    // opened or asked. The recipient write checks it again, authoritatively,
    // but only after the confirmation and the passphrase.
    if let Some(device) = device
        && !devicename::valid(device)
    {
        return Err(fail(
            Code::Usage,
            format!("gage: device name {device:?} is invalid"),
        ));
    }
    let typed_codes: Vec<Zeroizing<String>> = matches
        .get_many::<String>("code")
        .map(|codes| codes.map(|c| Zeroizing::new(c.clone())).collect())
        .unwrap_or_default();
    let ids: Vec<&str> = matches
        .get_many::<String>("ID")
        .map(|ids| ids.map(String::as_str).collect())
        .unwrap_or_default();
    let prompter = keys.prompter();

    // Steps 1 to 4, holding no identity: everything carried out of here is
    // owned, so the vault is free to be unlocked afterwards.
    let (approvals, codes) = with_resolved_vault(matches, io, keys, |io, vault| {
        let scope = approval_scope(io, vault, &ids)?;
        let (opened, codes) = open_with_codes(vault, &scope, typed_codes, prompter)?;
        // One flag can't say which of several requests it relabels.
        if device.is_some() && opened.len() != 1 {
            return Err(fail(
                Code::Usage,
                format!(
                    "gage: --device relabels one request, but this run opened {}; pass the ID of the one to relabel (see `gage recipient pending`)",
                    opened.len()
                ),
            ));
        }
        let approvals: Vec<Approval> = opened
            .into_iter()
            .map(|o| match device {
                Some(label) => Approval::relabeled(o, label),
                None => Approval::new(o),
            })
            .collect();
        // Before the confirmation and any unlock: it reads the plaintext
        // recipient list.
        vault.check_enrollment_labels(&approvals)?;
        confirm_approval(io, vault, &approvals, prompter)?;
        Ok((approvals, codes))
    })?;

    // Step 5, the unlock, and only now. One-shot unlocks for this command;
    // a session reuses the identity it holds, or unlocks once and keeps it.
    let (name, result) = keys.with_vault(matches, io, |_, vault, identity| {
        let result = vault.approve_enrollments(&approvals, &codes, identity, prompter)?;
        Ok((vault.name.clone(), result))
    })?;
    out(io, &approval_lines(&name, &result));
    Ok(())
}

/// The difference between the two runs, and the only one: with no ID,
/// every live request; with IDs, exactly the ones they resolve to. No flag
/// turns the 32-request bound on or off. A broad run can hit it and an
/// ID-scoped run never can, which is what makes `TooManyPending`'s advice
/// an escape hatch by construction.
fn approval_scope(
    io: &mut Io,
    vault: &Vault,
    ids: &[&str],
) -> Result<Vec<PendingRequest>, CliError> {
    if ids.is_empty() {
        return Ok(vault.pending_enrollments()?);
    }
    let mut scope: Vec<PendingRequest> = Vec::new();
    for id in ids {
        let request = vault
            .resolve_enrollment(id)
            .map_err(|e| report_ambiguous_request(io, e))?;
        // Two spellings of one id name one request. Kept twice, it would
        // open twice on the prompted path, show one device as two, and be
        // refused under the lock as a duplicate after the confirmation and
        // the passphrase (a gap Go's `approvalScope` leaves open).
        if !scope.iter().any(|r| r.id == request.id) {
            scope.push(request);
        }
    }
    Ok(scope)
}

/// The codes a human supplied, turned into opened requests, with the codes
/// that opened them for the re-open under the lock.
///
/// Each `--code` is tried on its own, which is what makes "every code must
/// open something" checkable: a code that opens nothing fails the whole
/// run rather than approving the subset that did, since a typo is likelier
/// than a deliberately surplus code. One code given twice opens the same
/// request twice, and only the first sighting is kept: a batch carrying it
/// twice would show one device as two and then be refused under the lock
/// as a duplicate, after the confirmation and the passphrase.
fn open_with_codes(
    vault: &Vault,
    scope: &[PendingRequest],
    typed: Vec<Zeroizing<String>>,
    prompter: &dyn Prompter,
) -> Result<(Vec<OpenedRequest>, Vec<Zeroizing<String>>), CliError> {
    if typed.is_empty() {
        return prompt_for_code(vault, scope, prompter);
    }
    let mut opened: Vec<OpenedRequest> = Vec::new();
    for code in &typed {
        let got = vault
            .open_enrollment(scope, std::slice::from_ref(code))
            .map_err(too_many_pending_advice)?;
        for request in got {
            if !opened.iter().any(|o| o.id == request.id) {
                opened.push(request);
            }
        }
    }
    Ok((opened, typed))
}

/// Reads a code through `Prompter::value` (no new prompter method,
/// D-ENROLL-PROMPTER) and retries a wrong one. The loop keys on
/// `CodeWrong` and only on it: a wrong-vault, malformed, expired or skewed
/// request exits with its own advice, since retyping a code that already
/// worked can't fix it. A blank answer cancels.
fn prompt_for_code(
    vault: &Vault,
    scope: &[PendingRequest],
    prompter: &dyn Prompter,
) -> Result<(Vec<OpenedRequest>, Vec<Zeroizing<String>>), CliError> {
    for _ in 0..MAX_CODE_PROMPTS {
        let code = prompter
            .value("enrollment code")
            .map_err(|e| fail(Code::LockedOrAuth, e))?;
        if code.trim().is_empty() {
            return Err(fail(
                Code::Conflict,
                "gage: no enrollment code given; nothing was approved",
            ));
        }
        let codes = vec![code];
        match vault.open_enrollment(scope, &codes) {
            // The code travels on with what it opened: the re-open under
            // the lock needs it.
            Ok(opened) => return Ok((opened, codes)),
            Err(EnrollmentError::CodeWrong) => prompter
                .warn("gage: that code did not open any pending request; check it and try again."),
            Err(e) => return Err(too_many_pending_advice(e)),
        }
    }
    Err(EnrollmentError::CodeWrong.into())
}

/// Renders what each request claims and asks the one question this
/// feature adds, in the terms the approver cares about (the device will
/// read everything, including what is already there) rather than naming
/// re-encryption, which is how that happens.
///
/// Not M10's question, and never collapsed into it: "should this device be
/// let in" and "do you trust the list you are about to encrypt to" are
/// different, and `--yes` answers only the second.
fn confirm_approval(
    io: &mut Io,
    vault: &Vault,
    approvals: &[Approval],
    prompter: &dyn Prompter,
) -> Result<(), CliError> {
    let entries = vault.entry_ids()?.len();
    let mut lines = vec![
        format!(
            "gage: opened {} pending {}.",
            approvals.len(),
            plural(approvals.len(), "request", "requests")
        ),
        String::new(),
    ];
    for a in approvals {
        let r = &a.request;
        lines.extend([
            format!("  device:  {}", r.device),
            format!("  pubkey:  {}", r.pubkey),
            format!("  method:  {}", r.method),
            format!(
                "  created: {}  (expires in {})",
                local(r.created, &io.time_zone),
                human_duration(until(r.expires))
            ),
        ]);
        if a.label() != r.device {
            lines.push(format!("  recorded as: {}  (--device)", a.label()));
        }
        lines.push(String::new());
    }
    out(io, &lines);

    let names: Vec<String> = approvals
        .iter()
        .map(|a| format!("{:?}", a.label()))
        .collect();
    let question = format!(
        "Add {} as {} of {:?}? It will be able to read all {entries} {}, including everything already in the vault.",
        names.join(", "),
        plural(approvals.len(), "a recipient", "recipients"),
        vault.name,
        plural(entries, "entry", "entries"),
    );
    match prompter.confirm(&question) {
        Ok(true) => Ok(()),
        Ok(false) => Err(fail(
            Code::Conflict,
            "gage: nothing was approved, and the request is still pending",
        )),
        // A frontend with nobody to ask is not one that agrees: letting a
        // device in is exactly what a run that was never asked must not
        // decide by omission.
        Err(e) => Err(fail(
            Code::LockedOrAuth,
            format!(
                "gage: there is nobody here to answer whether to admit this device, so nothing was approved and the request is still pending: {e}"
            ),
        )),
    }
}

/// What a completed approval did, per request and then once for the
/// batch. "Committed locally", as `recipient add` says it: the guarantee
/// is about HEAD, and publishing is the push, which warns and carries on.
fn approval_lines(vault: &str, res: &ApprovalResult) -> Vec<String> {
    let mut lines = Vec::new();
    let mut added = 0;
    for o in &res.outcomes {
        if !o.added {
            lines.push(format!(
                "gage: {:?} is already a recipient of {vault:?}, so nothing was granted; its request is cleared.",
                o.label
            ));
            continue;
        }
        added += 1;
        lines.push(format!(
            "gage: approved {:?} ({})",
            o.label, o.request.pubkey
        ));
    }
    lines.push(format!(
        "gage: {added} {} added, {} {} re-encrypted, {} {} cleared; committed locally as {}",
        plural(added, "recipient", "recipients"),
        res.reencrypted,
        plural(res.reencrypted, "entry", "entries"),
        res.outcomes.len(),
        plural(res.outcomes.len(), "request", "requests"),
        short_hash(&res.commit)
    ));
    lines
}

pub fn deny(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    let id = arg(matches, "ID");
    let prompter = keys.prompter();
    with_resolved_vault(matches, io, keys, |io, vault| {
        let denied = vault
            .deny_enrollment(id, prompter)
            .map_err(|e| report_ambiguous_request(io, e))?;
        out(
            io,
            &[format!(
                "gage: removed pending request {} from {:?}. Nothing was granted; nothing to re-encrypt.",
                short_request_id(&denied.id),
                vault.name
            )],
        );
        Ok(())
    })
}

/// Prints an ambiguous id's candidates to stderr, then hands the error
/// back. The library returns the matches as a value; how to show them is
/// this frontend's call. The expiry rides along because in an ambiguous
/// run the id is by definition not enough to tell them apart, and it is
/// the only other thing knowable without a code.
fn report_ambiguous_request(io: &mut Io, err: EnrollmentError) -> CliError {
    if let EnrollmentError::Ambiguous(RequestCandidates { id, matches }) = &err {
        let _ = writeln!(
            io.stderr,
            "gage: {id:?} matches {} pending enrollment requests:",
            matches.len()
        );
        for (i, r) in matches.iter().enumerate() {
            let _ = writeln!(
                io.stderr,
                "  {}. {}   expires {}",
                i + 1,
                short_request_id(&r.id),
                local(r.expires, &io.time_zone)
            );
        }
    }
    err.into()
}

/// `TooManyPending` with the exact command line that gets out of it. The
/// library can say "by id"; only a frontend knows what to type, and
/// `recipient pending` is named because the id has to come from somewhere
/// and that listing opens nothing, so a stuffed directory can't block it.
fn too_many_pending_advice(err: EnrollmentError) -> CliError {
    if !matches!(err, EnrollmentError::TooManyPending { .. }) {
        return err.into();
    }
    fail(
        err.code(),
        format!(
            "gage: {err}; run `gage recipient pending` to find the id, then `gage recipient approve <ID> --code <code>`"
        ),
    )
}

/// The eight-character form the listing shows and every resolver accepts.
fn short_request_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn local(t: Timestamp, tz: &TimeZone) -> String {
    t.to_zoned(tz.clone()).strftime(TIME_FORMAT).to_string()
}

fn until(t: Timestamp) -> Duration {
    Duration::from_secs((t.as_second() - Timestamp::now().as_second()).max(0) as u64)
}

fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}
