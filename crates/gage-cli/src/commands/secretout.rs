//! The one place a decrypted value leaves gage.
//! The default prints it; `-q` renders a QR code *instead*; `-c` copies it
//! *instead*. Nothing prints the value alongside either one: that would
//! defeat the point of asking for them (principle 6, "plaintext should be
//! surprising to produce").

use clap::{Arg, ArgAction, ArgMatches};
use gage::exitcode::Code;

use super::{Keys, fail, load_global};
use crate::Io;
use crate::error::CliError;
use crate::{duration, qr, shell};

/// `-c/--clip`, with each command's own help.
pub(super) fn clip_flag(help: &'static str) -> Arg {
    Arg::new("clip")
        .short('c')
        .long("clip")
        .action(ArgAction::SetTrue)
        .help(help)
}

/// `-q/--qr`, with each command's own help.
pub(super) fn qr_flag(help: &'static str) -> Arg {
    Arg::new("qr")
        .short('q')
        .long("qr")
        .action(ArgAction::SetTrue)
        .help(help)
}

/// Where the value goes instead of stdout, if anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Destination {
    Print,
    Clipboard,
    Qr,
}

impl Destination {
    /// Refuses asking for two destinations at once. They are two answers
    /// to one question, and a run doing both would block for the
    /// clipboard timeout with a QR code on screen, which is neither thing
    /// anyone asked for. Allowing it later is additive.
    ///
    /// Separate from [`emit_secret`] so a command asks before it does
    /// anything: `generate` would otherwise leave a new secret in the vault
    /// behind a message saying the command failed, and `show` would ask
    /// for a passphrase only to refuse.
    pub(super) fn from_matches(matches: &ArgMatches) -> Result<Destination, CliError> {
        match (matches.get_flag("clip"), matches.get_flag("qr")) {
            (true, true) => Err(fail(
                Code::Usage,
                "gage: -c/--clip and -q/--qr are mutually exclusive",
            )),
            (true, false) => Ok(Destination::Clipboard),
            (false, true) => Ok(Destination::Qr),
            (false, false) => Ok(Destination::Print),
        }
    }
}

/// Sends `value` to `dest`.
pub(super) fn emit_secret(
    io: &mut Io,
    keys: &Keys,
    value: &str,
    dest: Destination,
) -> Result<(), CliError> {
    match dest {
        Destination::Qr => {
            let art = qr::render_qr(value)?;
            let _ = io.stdout.write_all(art.as_bytes());
            Ok(())
        }
        Destination::Clipboard => copy_secret(io, keys, value),
        Destination::Print => {
            let _ = io.stdout.write_all(value.as_bytes());
            if !value.ends_with('\n') {
                let _ = io.stdout.write_all(b"\n");
            }
            Ok(())
        }
    }
}

/// `-c`: copy, and make sure it doesn't stay there.
///
/// The two modes differ in who waits, and that is the design decision, not
/// an optimization. The clear always happens in the process that wrote the
/// clipboard, never a forked child, which would be the surviving background
/// process principle 5 refuses. So a one-shot blocks until the timeout,
/// while a session is already long-lived and clears on a timer.
///
/// The notice goes to stderr: `-c` prints no plaintext, and a human still
/// needs telling their clipboard is on a clock.
fn copy_secret(io: &mut Io, keys: &Keys, value: &str) -> Result<(), CliError> {
    let timeout = clipboard_timeout(io)?;
    io.clipboard.copy(value)?;
    let shown = duration::format(timeout);
    if keys.in_session() {
        io.clipboard.schedule_clear(timeout);
        let _ = writeln!(io.stderr, "gage: copied to clipboard; clears in {shown}.");
        return Ok(());
    }
    let _ = writeln!(
        io.stderr,
        "gage: copied to clipboard; clears in {shown} — Ctrl-C to clear now."
    );
    // What the command printed (generate's confirmation) is out before it
    // blocks.
    let _ = io.stdout.flush();
    (io.clipboard_wait)(timeout);
    io.clipboard.clear()
}

/// `[shell].clipboard_timeout`, read when a copy happens, so a command that
/// never copies never reads it. A session resolved the same table when it
/// started, so a value that fails here would have refused the session.
fn clipboard_timeout(io: &Io) -> Result<std::time::Duration, CliError> {
    let (_, global) = load_global(io)?;
    let dirs = super::dirs(io)?;
    Ok(shell::resolve(&global.shell, &dirs, &std::env::home_dir)?.clipboard_timeout)
}
