//! gage's process exit codes (ADR-0037), and the mapping from the
//! library's [`ErrorKind`] to them. The numbers are a script-facing
//! contract owned by `doc/cli/exit-codes.md`; the library never sees them.

use std::fmt;

use gage::ErrorKind;

/// One of gage's well-known process exit codes. Every command's exit status
/// is one of these, never a bare integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExitCode {
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

impl ExitCode {
    /// Every code, in numeric order.
    pub const ALL: [ExitCode; 8] = [
        ExitCode::Success,
        ExitCode::Conflict,
        ExitCode::Usage,
        ExitCode::NotFound,
        ExitCode::Ambiguous,
        ExitCode::LockedOrAuth,
        ExitCode::Internal,
        ExitCode::Unreachable,
    ];

    /// The process exit status for this code.
    pub const fn as_i32(self) -> i32 {
        self as i32
    }

    /// The stable name error messages and tests refer to the code by.
    pub const fn name(self) -> &'static str {
        match self {
            ExitCode::Success => "success",
            ExitCode::Conflict => "conflict",
            ExitCode::Usage => "usage",
            ExitCode::NotFound => "not-found",
            ExitCode::Ambiguous => "ambiguous",
            ExitCode::LockedOrAuth => "locked-or-auth",
            ExitCode::Internal => "internal",
            ExitCode::Unreachable => "unreachable",
        }
    }
}

impl From<ErrorKind> for ExitCode {
    fn from(kind: ErrorKind) -> ExitCode {
        match kind {
            ErrorKind::Conflict => ExitCode::Conflict,
            ErrorKind::Usage => ExitCode::Usage,
            ErrorKind::NotFound => ExitCode::NotFound,
            ErrorKind::Ambiguous => ExitCode::Ambiguous,
            ErrorKind::LockedOrAuth => ExitCode::LockedOrAuth,
            ErrorKind::Internal => ExitCode::Internal,
            ErrorKind::Unreachable => ExitCode::Unreachable,
        }
    }
}

impl fmt::Display for ExitCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn success_is_zero() {
        assert_eq!(ExitCode::Success.as_i32(), 0);
    }

    // Exit codes 0-7 are a script-facing contract owned by
    // doc/cli/exit-codes.md; renumbering one breaks every caller that
    // branches on it.
    #[test]
    fn all_codes_are_distinct_and_match_the_documented_values() {
        let values: Vec<i32> = ExitCode::ALL.iter().map(|c| c.as_i32()).collect();
        assert_eq!(values, [0, 1, 2, 3, 4, 5, 6, 7]);
        let distinct: HashSet<i32> = values.iter().copied().collect();
        assert_eq!(distinct.len(), ExitCode::ALL.len());
    }

    // Every kind maps to the number doc/cli/exit-codes.md gives it.
    #[test]
    fn every_kind_maps_to_its_documented_number() {
        let mapped: Vec<i32> = ErrorKind::ALL
            .iter()
            .map(|&k| ExitCode::from(k).as_i32())
            .collect();
        assert_eq!(mapped, [1, 2, 3, 4, 5, 6, 7]);
    }

    // An out-of-range code can't be constructed in Rust, so what's left to
    // prove is that every variant renders as its stable, non-empty name.
    #[test]
    fn every_code_displays_its_stable_name() {
        let names: Vec<String> = ExitCode::ALL.iter().map(ToString::to_string).collect();
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
