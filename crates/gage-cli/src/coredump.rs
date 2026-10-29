//! Process-wide crash-dump suppression: the other half of the memory
//! protection in "Session model". Page-locking keeps a key out of swap;
//! this keeps a crash from writing the process's memory, unlocked keys
//! included, to a core file. A policy for the whole process, so it belongs
//! to the binary, and `main` calls it before anything else.
//!
//! Linux `prctl(PR_SET_DUMPABLE)` is deliberately not added: it blocks
//! debuggers and `/proc` access, which is a behavior
//! change rather than a hardening of this one.

#![allow(unsafe_code)]

use std::io;

/// What this platform actually promises, for the place a user could ask.
#[cfg(unix)]
pub const GUARANTEE: &str = "core dumps are disabled for this process (RLIMIT_CORE = 0)";

/// Windows has no Unix core dump to switch off, and a non-elevated process
/// can't guarantee no crash dump is ever written (Windows Error Reporting
/// can be configured machine-wide to collect them regardless). What gage
/// can do is stop the crash dialog that would offer to produce one: a
/// documented gap, not a claimed guarantee.
#[cfg(windows)]
pub const GUARANTEE: &str = "the Windows Error Reporting crash dialog is suppressed (SetErrorMode); \
    unlike POSIX's RLIMIT_CORE this cannot guarantee no crash dump is ever written";

/// Whether the suppression is one-way for the life of the process. POSIX's
/// is (the hard limit can't be raised again by an unprivileged process);
/// `SetErrorMode` has no one-way form, so anything in the process can
/// clear it.
pub const IRREVOCABLE: bool = cfg!(unix);

/// Disables core dumps for this process. Idempotent.
#[cfg(unix)]
pub fn disable_core_dumps() -> io::Result<()> {
    // Both limits: lowering only the soft one would leave anything in the
    // process free to raise it again.
    let zero = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: setrlimit reads the struct it is handed.
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &zero) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Reads the setting back from the OS, so tests assert what the kernel
/// holds rather than what this module believes it asked for.
#[cfg(unix)]
pub fn core_dumps_disabled() -> io::Result<bool> {
    Ok(core_limit()?.rlim_cur == 0)
}

#[cfg(unix)]
fn core_limit() -> io::Result<libc::rlimit> {
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit fills the struct it is handed.
    if unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut lim) } == 0 {
        Ok(lim)
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
const ERROR_MODE_MASK: u32 = windows_sys::Win32::System::Diagnostics::Debug::SEM_FAILCRITICALERRORS
    | windows_sys::Win32::System::Diagnostics::Debug::SEM_NOGPFAULTERRORBOX;

/// Disables the crash dialog for this process (see [`GUARANTEE`]).
/// Idempotent.
#[cfg(windows)]
pub fn disable_core_dumps() -> io::Result<()> {
    // SAFETY: SetErrorMode takes a flag word and cannot fail.
    unsafe { windows_sys::Win32::System::Diagnostics::Debug::SetErrorMode(ERROR_MODE_MASK) };
    Ok(())
}

/// Reads the process error mode back from the OS.
#[cfg(windows)]
pub fn core_dumps_disabled() -> io::Result<bool> {
    // SAFETY: GetErrorMode has no preconditions.
    let mode = unsafe { windows_sys::Win32::System::Diagnostics::Debug::GetErrorMode() };
    Ok(mode & ERROR_MODE_MASK == ERROR_MODE_MASK)
}

#[cfg(test)]
#[allow(clippy::disallowed_macros)]
mod tests {
    use super::*;

    const CHILD_ENV: &str = "GAGE_COREDUMP_TEST_CHILD";

    /// Undoes any suppression already in force, so `disable_core_dumps`
    /// has something to do: core dumps are off by default on macOS and
    /// under most shells' `ulimit -c 0`, so a test that only disabled and
    /// read back would pass against an implementation that did nothing.
    /// False if it can't be undone (an outer process already lowered the
    /// hard limit).
    #[cfg(unix)]
    fn undo_suppression() -> bool {
        let Ok(lim) = core_limit() else { return false };
        if lim.rlim_max == 0 {
            return false;
        }
        let raised = libc::rlimit {
            rlim_cur: 1,
            rlim_max: lim.rlim_max,
        };
        // SAFETY: setrlimit reads the struct it is handed.
        unsafe { libc::setrlimit(libc::RLIMIT_CORE, &raised) == 0 }
    }

    #[cfg(windows)]
    fn undo_suppression() -> bool {
        // SAFETY: as in disable_core_dumps.
        unsafe { windows_sys::Win32::System::Diagnostics::Debug::SetErrorMode(0) };
        !core_dumps_disabled().unwrap()
    }

    /// The child side: one process, one order, whatever the parent's test
    /// threads do, since lowering the hard limit is permanent for the
    /// process and inherited by its children.
    #[test]
    fn core_dump_child() {
        let Some(which) = std::env::var_os(CHILD_ENV) else {
            return;
        };
        match which.to_str().unwrap() {
            "disabled" => {
                if !undo_suppression() {
                    eprintln!("RESULT skip");
                    return;
                }
                // Guard the guard: if undoing didn't take, everything
                // below is vacuous.
                assert!(
                    !core_dumps_disabled().unwrap(),
                    "still disabled after undoing"
                );
                disable_core_dumps().unwrap();
                assert!(
                    core_dumps_disabled().unwrap(),
                    "still enabled; promised: {GUARANTEE}"
                );
                if IRREVOCABLE {
                    #[cfg(unix)]
                    assert_eq!(
                        core_limit().unwrap().rlim_max,
                        0,
                        "the hard limit was left above zero"
                    );
                    assert!(!undo_suppression(), "the suppression could be undone");
                }
                eprintln!("RESULT ok");
            }
            "idempotent" => {
                for i in 1..=3 {
                    disable_core_dumps().unwrap_or_else(|e| panic!("call {i}: {e}"));
                }
                assert!(core_dumps_disabled().unwrap());
                eprintln!("RESULT ok");
            }
            other => panic!("unknown child mode {other:?}"),
        }
    }

    /// Runs the child in `mode` and returns what it reported.
    fn run_child(mode: &str) -> String {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "coredump::tests::core_dump_child",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, mode)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(out.status.success(), "child {mode} failed:\n{stderr}");
        stderr
            .lines()
            .find_map(|l| l.strip_prefix("RESULT "))
            .unwrap_or_else(|| panic!("child {mode} reported nothing:\n{stderr}"))
            .to_string()
    }

    // The effect, read back from the OS after
    // first undoing whatever suppression the environment already applied;
    // irrevocability only where the platform offers it.
    #[test]
    fn core_dumps_are_disabled() {
        match run_child("disabled").as_str() {
            "ok" => {}
            "skip" => eprintln!(
                "skipping: core dumps are already irrevocably disabled for this process tree, \
                 so disable_core_dumps has nothing to change"
            ),
            other => panic!("unexpected child result {other:?}"),
        }
    }

    #[test]
    fn disabling_core_dumps_is_idempotent() {
        assert_eq!(run_child("idempotent"), "ok");
    }

    // Guards the wiring: the call
    // can be correct and never made, and a missing call is silent. It must
    // be main's first statement, before anything that could hold a key.
    #[test]
    fn main_disables_core_dumps_at_startup() {
        let main = include_str!("main.rs");
        let body = &main[main.find("fn main() {").expect("main.rs has a main")..];
        let first = body
            .lines()
            .skip(1)
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with("//"))
            .unwrap();
        assert!(
            first.starts_with("if let Err(e) = gage_cli::coredump::disable_core_dumps()"),
            "main's first statement is {first:?}, not the core-dump suppression"
        );
    }
}
