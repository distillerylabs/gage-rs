//! Handing a freshly generated recovery key to its human: deciding how
//! from the flags before anything exists (`plan_recovery_key`), refusing a
//! destination that can't be written or that gage would delete
//! (`check_recovery_key_out_path`), and, once the vault holds the key as
//! a recipient, showing it or writing it (`deliver_recovery_key`).
//! `init`, `recovery rotate` and `recovery enroll` share all of it. See
//! "Recovery key at `init`" in the design doc.
//!
//! The library makes the key and registers it; everything here is the
//! frontend's: which stream it goes to, what is said beside it, and the
//! re-type that asks a human to prove they have it.

use std::io::Write;
use std::path::{Component, Path, PathBuf};

use gage::atomicfile;
use gage::exitcode::Code;
use gage::prompter::Prompter;
use gage::recovery::RecoveryKey;
use gage::xdgpaths::Dirs;
use zeroize::Zeroizing;

use super::fail;
use crate::error::{CliError, ERROR_PREFIX};
use crate::{Io, qr};

/// How many of the key's own last characters the re-type asks for. An
/// attention check, not authentication: enough that it can't be answered
/// without the key in front of you, few enough that answering from paper
/// isn't a chore. Nothing rides on the answer but the exit code: the key
/// is already a recipient by the time it is asked.
pub(crate) const RECOVERY_CONFIRM_CHARS: usize = 6;

/// A typo deserves another go; an endless loop helps nobody.
pub(crate) const MAX_RECOVERY_CONFIRM_ATTEMPTS: usize = 3;

/// `--recovery-key-out`'s mode: the identity file's, not a config file's,
/// since what it holds is an unencrypted private key.
const RECOVERY_KEY_FILE_MODE: u32 = 0o600;

/// What a command will do with the recovery key it is about to generate,
/// settled from the flags before anything is created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeyPlan {
    /// The opt-out flag: no key is generated at all.
    None,
    /// Shown on stderr, then re-typed.
    Show,
    /// Written to this new file. As given by `plan_recovery_key`; absolute
    /// once `check_recovery_key_out_path` has passed it.
    File(PathBuf),
}

/// Settles what a command does about the recovery key, refusing now if it
/// can do nothing sensible.
///
/// The refusal that matters is a default run with no terminal: showing a
/// key to a log or a CI transcript is the exposure keeping it off disk
/// exists to avoid, and quietly choosing "no key" creates the
/// unrecoverable vault this whole feature exists to prevent. So it fails,
/// naming the flags that resolve it.
///
/// `skip_flag` is the caller's own opt-out (`init`'s `--no-recovery-key`,
/// enroll's `--no-new-recovery-key`), so the refusal names a flag the user
/// can actually pass; `None` for rotate, which exists to produce a key.
pub(crate) fn plan_recovery_key(
    io: &Io,
    skip_flag: Option<&str>,
    no_key: bool,
    out: Option<&str>,
) -> Result<KeyPlan, CliError> {
    match (no_key, out) {
        (true, Some(_)) => Err(fail(
            Code::Usage,
            format!(
                "{} and --recovery-key-out are mutually exclusive; the first says not to make a key, the second says where to put one",
                skip_flag.unwrap_or("--no-recovery-key")
            ),
        )),
        (true, None) => Ok(KeyPlan::None),
        // Not "unset": a user who typed the flag asked to keep the key off
        // the screen, and an empty path would otherwise name the working
        // directory (ADR-0051).
        (false, Some("")) => Err(fail(
            Code::Usage,
            "--recovery-key-out needs a file name to write the recovery key to",
        )),
        // The destination is checked separately, once the caller knows
        // which vault it must stay out of.
        (false, Some(out)) => Ok(KeyPlan::File(PathBuf::from(out))),
        // Both, for two reasons: the key is written to stderr and the
        // re-type is read from stdin. `gage init foo 2>build.log` has one
        // and not the other, and is exactly the run that would file a
        // private key into a build log.
        (false, None) if io.stdin_is_terminal && io.stderr_is_terminal => Ok(KeyPlan::Show),
        (false, None) => Err(fail(
            Code::Usage,
            match skip_flag {
                None => {
                    "this generates a recovery key and shows it once, which needs a terminal.\n\
                         gage: pass --recovery-key-out FILE to write it to a file instead."
                        .to_string()
                }
                Some(flag) => format!(
                    "this generates a recovery key and shows it once, which needs a terminal.\n\
                     gage: pass --recovery-key-out FILE to write it to a file instead, or {flag} to\n\
                     gage: go without one (which leaves this device's identity file as the only way\n\
                     gage: in — if it or its passphrase is lost, the vault is unreadable forever)."
                ),
            },
        )),
    }
}

/// Refuses an output path the key couldn't be written to, or shouldn't
/// be, and returns it absolute (against `cwd`), which is where the write
/// must then go. Runs before anything is created, so a destination that
/// fails is found while there is still no key riding on it.
///
/// `vault_path` is the vault the key belongs to: for `init`, one that
/// doesn't exist yet and so can't be found any other way.
pub(crate) fn check_recovery_key_out_path(
    path: &Path,
    cwd: &Path,
    vault_path: &Path,
    dirs: &Dirs,
) -> Result<PathBuf, CliError> {
    let dest = absolute(cwd, path);
    let shown = path.display();

    // First: the only check that holds when nothing on the path exists
    // yet, as for the vault being created, where "cannot write" below
    // would answer instead and say nothing useful.
    check_not_gages(&dest, &shown.to_string(), &absolute(cwd, vault_path), dirs)?;

    match std::fs::symlink_metadata(&dest) {
        // A dangling symlink exists too: writing through it would land
        // the key wherever it points.
        Ok(_) => {
            return Err(fail(
                Code::Usage,
                format!("{shown} already exists; refusing to overwrite it with a new recovery key"),
            ));
        }
        // Through a regular file a path doesn't exist either; the parent
        // check below says why in words.
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) => {}
        Err(e) => return Err(CliError::internal(anyhow::anyhow!("checking {shown}: {e}"))),
    }

    let parent = dest.parent().unwrap_or(&dest);
    match std::fs::metadata(parent) {
        Err(e) => return Err(fail(Code::Usage, format!("cannot write {shown}: {e}"))),
        Ok(meta) if !meta.is_dir() => {
            return Err(fail(
                Code::Usage,
                format!(
                    "cannot write {shown}: {} is not a directory",
                    parent.display()
                ),
            ));
        }
        Ok(_) => {}
    }

    // Stat says nothing about a read-only or full directory. Creating a
    // file is the only proof short of writing the real one, by which time
    // the vault exists and the key being lost is its recipient.
    tempfile::Builder::new()
        .prefix(".gage-write-probe-")
        .tempfile_in(parent)
        .map_err(|e| fail(Code::Usage, format!("cannot write {shown}: {e}")))?;
    Ok(dest)
}

/// Refuses a destination gage manages, because gage deletes those.
///
/// Inside a vault is the sharp one: the next write after an interrupted
/// one resets the working tree, which removes untracked files, so the
/// user's only copy of an unencrypted key would go under a message about
/// discarding an interrupted write. The data dir is the blunt one: it is
/// gage's to rewrite, and `reset-local-state` deletes all of it
/// (ADR-0051). Both are refused rather than warned about.
fn check_not_gages(dest: &Path, shown: &str, vault: &Path, dirs: &Dirs) -> Result<(), CliError> {
    const RESET: &str = "gage resets a vault's working tree after an\n\
                         gage: interrupted write, which would delete the key. Write it outside any vault.";
    if path_is_inside(dest, vault) {
        return Err(fail(
            Code::Usage,
            format!("{shown} is inside the vault being created; {RESET}"),
        ));
    }
    // Any other vault, found the way a vault is recognisable from outside,
    // so unregistered ones too. From the resolved path, so a `..` or a
    // symlink can't walk the check past a vault the write would land in.
    let resolved = resolve_existing(dest);
    for dir in resolved.ancestors().skip(1) {
        if dir.join(".gage").join("config.toml").exists() {
            return Err(fail(
                Code::Usage,
                format!("{shown} is inside the vault at {}; {RESET}", dir.display()),
            ));
        }
    }
    if path_is_inside(dest, &dirs.data) {
        return Err(fail(
            Code::Usage,
            format!(
                "{shown} is inside {}, which gage manages and rewrites; a key there is not safely\n\
                 gage: stored. Write it outside gage's own directories.",
                dirs.data.display()
            ),
        ));
    }
    Ok(())
}

/// Whether `path` is `root` or beneath it, compared component by
/// component after symlinks are resolved as far as each path exists: on
/// macOS `/var` is `/private/var`, so comparing the paths as given would
/// quietly answer no. Both paths are expected absolute. An empty root
/// contains nothing.
pub(crate) fn path_is_inside(path: &Path, root: &Path) -> bool {
    if root.as_os_str().is_empty() {
        return false;
    }
    resolve_existing(path).starts_with(resolve_existing(root))
}

/// `path` against `cwd` when relative, with `.` and `..` folded away
/// lexically. Without that, an ancestor walk from a
/// bare `recovery.key` never reaches the directories it is really in.
fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                // At the root, `..` is the root.
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Resolves symlinks in the deepest existing ancestor of `path` and
/// reattaches the rest, so a file not yet created still compares against
/// a resolved root. `path` is returned as is if
/// no ancestor resolves.
fn resolve_existing(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut dir = path;
    loop {
        if let Ok(resolved) = std::fs::canonicalize(dir) {
            return rest.iter().rev().fold(resolved, |p, c| p.join(c));
        }
        match (dir.parent(), dir.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                dir = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Hands the key to its human by the route `plan` settled on.
///
/// Called only once the vault holds the key as a recipient, is committed
/// and (for `init`) registered: a key shown for a vault that then failed
/// to exist is a real secret someone now has to decide what to do with.
/// The other side of that is that a failure here can't unmake anything,
/// so the caller holds the error, finishes its summary and its push, and
/// only then exits on it.
pub(crate) fn deliver_recovery_key(
    io: &mut Io,
    prompter: &dyn Prompter,
    plan: &KeyPlan,
    vault: &str,
    key: &RecoveryKey,
) -> Result<(), CliError> {
    match plan {
        KeyPlan::None => Ok(()),
        KeyPlan::Show => show_recovery_key(io, prompter, vault, key),
        KeyPlan::File(dest) => write_recovery_key_file(io, vault, dest, key),
    }
}

/// The key, its QR and the custody rules that are the only protection an
/// unencrypted key has, then the re-type. All on stderr, with the
/// prompts: stdout is for a value someone asked for and may be
/// redirecting, and this is a one-time notice for the human at the
/// terminal.
///
/// The secret is written straight from the key's own zeroizing buffer,
/// never formatted into an intermediate `String`; the QR render is
/// zeroized on drop like it. What isn't covered is the terminal's own
/// copy, and that the buffers aren't page-locked
/// (ADR-0027).
fn show_recovery_key(
    io: &mut Io,
    prompter: &dyn Prompter,
    vault: &str,
    key: &RecoveryKey,
) -> Result<(), CliError> {
    let secret: &str = &key.secret;
    let err = &mut io.stderr;
    let _ = writeln!(
        err,
        "\ngage: {vault:?} now has a recovery key. This is the only time it will be shown.\n"
    );
    let _ = err.write_all(b"    ");
    let _ = err.write_all(secret.as_bytes());
    let _ = err.write_all(b"\n\n");
    // A failed QR is not a failed delivery: the key is already on screen,
    // and stopping here would skip the warnings and the re-type, which
    // are the parts that matter.
    match qr::render_qr(secret) {
        Ok(art) => {
            let _ = err.write_all(art.as_bytes());
        }
        Err(e) => prompter.warn(&format!(
            "gage: could not draw the key as a QR code: {}",
            e.to_string().trim_start_matches(ERROR_PREFIX)
        )),
    }
    let _ = writeln!(
        err,
        "\n\
         gage: this key is not encrypted, and anyone who has it can read every secret in\n\
         gage: this vault. gage keeps no copy, so it cannot be shown again.\n\
         gage:\n\
         gage: store it offline: print it on a printer you trust, or write it out by hand,\n\
         gage: and keep the paper somewhere you would keep a passport. An offline encrypted\n\
         gage: drive works too. Do not put it in cloud-synced notes, a password manager,\n\
         gage: email, or a photo. Clear this terminal's scrollback when you are done.\n"
    );
    let _ = err.flush();
    confirm_recovery_key_saved(prompter, secret)
}

/// Asks for the key's last characters back, which can't be done without
/// it in front of you. Running out of attempts fails the command and
/// changes nothing: the vault already has this key as a recipient, and
/// unmaking a good vault over a fumbled prompt would be worse than what
/// the prompt guards against. So the error is loud about what is now
/// true. The key isn't reprinted: it is still on screen, and a second copy
/// in the scrollback is one more place to clear.
fn confirm_recovery_key_saved(prompter: &dyn Prompter, secret: &str) -> Result<(), CliError> {
    let want = &secret[secret.len() - RECOVERY_CONFIRM_CHARS..];
    let title = format!(
        "the last {RECOVERY_CONFIRM_CHARS} characters of the recovery key, to confirm you have saved it"
    );
    for attempt in 1..=MAX_RECOVERY_CONFIRM_ATTEMPTS {
        let got = prompter
            .value(&title)
            .map_err(|e| fail(Code::LockedOrAuth, e))?;
        if got.trim().eq_ignore_ascii_case(want) {
            return Ok(());
        }
        if attempt < MAX_RECOVERY_CONFIRM_ATTEMPTS {
            prompter.warn("gage: that doesn't match the key above; try again.");
        }
    }
    Err(fail(
        Code::Conflict,
        "the recovery key was not confirmed.\n\
         gage: the change is committed and that key is one of this vault's recipients, so it\n\
         gage: is worth saving from the screen above — gage kept no copy and it cannot be shown\n\
         gage: again. If it is already gone, `gage recovery rotate` replaces it with a new one.",
    ))
}

/// `--recovery-key-out`: the key goes to a new file instead of a screen,
/// for runs that have no screen. No re-type: there is nobody to ask, and a
/// file isn't transient. What replaces it is saying plainly what is now
/// sitting on this disk.
fn write_recovery_key_file(
    io: &mut Io,
    vault: &str,
    dest: &Path,
    key: &RecoveryKey,
) -> Result<(), CliError> {
    // The newline makes it the file `age -i` reads directly.
    let mut data = Zeroizing::new(Vec::with_capacity(key.secret.len() + 1));
    data.extend_from_slice(key.secret.as_bytes());
    data.push(b'\n');
    // The path was checked before anything was created, so reaching here
    // means the disk changed underneath. Says only what is true of every
    // caller: init has just made the vault and rotate or enroll has just
    // changed one, and naming the wrong one would describe an operation
    // the user never ran.
    atomicfile::write_new(dest, &data, RECOVERY_KEY_FILE_MODE).map_err(|e| {
        fail(
            Code::Internal,
            format!(
                "{vault:?}'s recipient list is committed, but writing its recovery key to {} failed: {e}\n\
                 gage: that key is lost — `gage recovery rotate` replaces it with a new one",
                dest.display()
            ),
        )
    })?;
    let _ = writeln!(
        io.stderr,
        "gage: wrote {vault:?}'s recovery key to {}\n\
         gage: it is not encrypted, and anyone who has it can read every secret in this vault.\n\
         gage: gage keeps no other copy. Move it somewhere offline and delete this file.",
        dest.display()
    );
    Ok(())
}
