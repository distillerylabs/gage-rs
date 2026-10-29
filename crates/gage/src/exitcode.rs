//! gage's process exit-code taxonomy, and the way library errors carry one.
//!
//! The library never exits the process: its error enums implement
//! [`HasCode`], and `gage-cli` is the only thing that turns a [`Code`] into
//! an actual exit status. Anything that never opted into a code renders as
//! [`Code::Internal`], so no command can surface an unenumerated status by
//! accident.

use std::fmt;

/// One of gage's well-known process exit codes. Every command's exit status
/// is one of these, never a bare integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Code {
    /// The command did what it was asked.
    Success = 0,
    /// A state a human must resolve: a sync divergence, a recipient-file
    /// mismatch (`gage recipient verify`), a dirty `entries/` working
    /// tree, and so on.
    Conflict = 1,
    /// The command line itself was invalid: bad flags, an unknown
    /// (sub)command, a mutually exclusive flag combination.
    Usage = 2,
    /// A query resolved to no entry.
    NotFound = 3,
    /// A query resolved to more than one candidate and there was nobody to
    /// ask (one-shot mode).
    Ambiguous = 4,
    /// An unlock failed: wrong passphrase, no local identity registered for
    /// this device, an expired auth token, and so on.
    LockedOrAuth = 5,
    /// An unexpected error that fits none of the other categories.
    Internal = 6,
    /// A remote could not be reached at all: DNS failure, connection
    /// refused, a timeout. Deliberately not `Conflict`: nothing about
    /// either side's state is wrong, so the same command is worth retrying
    /// unchanged once the network is back, and a script can act on that.
    ///
    /// gage's automatic sync paths never surface this (being offline warns
    /// once and the triggering operation still succeeds); only an
    /// explicitly invoked sync verb reaches it.
    Unreachable = 7,
}

impl Code {
    /// The process exit status for this code.
    pub const fn as_i32(self) -> i32 {
        self as i32
    }

    /// The stable name error messages and tests refer to the code by.
    pub const fn name(self) -> &'static str {
        match self {
            Code::Success => "success",
            Code::Conflict => "conflict",
            Code::Usage => "usage",
            Code::NotFound => "not-found",
            Code::Ambiguous => "ambiguous",
            Code::LockedOrAuth => "locked-or-auth",
            Code::Internal => "internal",
            Code::Unreachable => "unreachable",
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Every defined code, in the order commands most often reach them. Tests
/// use it to assert each code is produced by at least one path.
pub const fn all() -> [Code; 8] {
    [
        Code::Success,
        Code::Conflict,
        Code::Usage,
        Code::NotFound,
        Code::Ambiguous,
        Code::LockedOrAuth,
        Code::Internal,
        Code::Unreachable,
    ]
}

/// Implemented by every library error type: the exit code a frontend should
/// render the error as.
pub trait HasCode {
    fn code(&self) -> Code;
}

/// The exit code of an operation's outcome: `Ok` is [`Code::Success`], an
/// error is whatever code it carries.
pub fn code_of<T, E: HasCode>(result: &Result<T, E>) -> Code {
    match result {
        Ok(_) => Code::Success,
        Err(e) => e.code(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[derive(Debug)]
    struct Coded(Code);

    impl HasCode for Coded {
        fn code(&self) -> Code {
            self.0
        }
    }

    #[test]
    fn success_is_zero() {
        assert_eq!(Code::Success.as_i32(), 0);
    }

    // Exit codes 0-7 are a script-facing
    // contract owned by doc/cli/exit-codes.md; renumbering one breaks every
    // caller that branches on it.
    #[test]
    fn all_codes_are_distinct_and_match_the_documented_values() {
        let values: Vec<i32> = all().iter().map(|c| c.as_i32()).collect();
        assert_eq!(values, [0, 1, 2, 3, 4, 5, 6, 7]);
        let distinct: HashSet<i32> = values.iter().copied().collect();
        assert_eq!(distinct.len(), all().len());
    }

    #[test]
    fn ok_is_success() {
        assert_eq!(code_of::<(), Coded>(&Ok(())), Code::Success);
    }

    // Every defined code round-trips through its constructors (library half;
    // the CLI half, through CliError and added context, is in gage-cli).
    // This round-trips the conversion; it does not show that
    // any command reaches each code. That becomes checkable as the
    // commands that produce NotFound, Ambiguous, Conflict, ... land.
    #[test]
    fn every_code_round_trips_through_has_code() {
        for code in all() {
            assert_eq!(code_of::<(), _>(&Err(Coded(code))), code, "{code}");
        }
    }

    // Wrapping nil is a compile-time property: in Rust a code is attached only by converting an error value
    // (gage-cli's CliError: From<E: HasCode>), so there is no nil to wrap.

    // An out-of-range Code
    // can't be constructed in Rust, so what's left to prove is that every
    // variant renders as its stable, non-empty name.
    #[test]
    fn every_code_displays_its_stable_name() {
        let names: Vec<String> = all().iter().map(ToString::to_string).collect();
        assert_eq!(
            names,
            [
                "success",
                "conflict",
                "usage",
                "not-found",
                "ambiguous",
                "locked-or-auth",
                "internal",
                "unreachable",
            ]
        );
    }
}
