//! How a command fails: an error message plus the exit code it renders as.
//!
//! Library errors arrive through `From`, carrying the code their
//! `HasCode` impl gives them. Anything else is Internal unless a handler
//! says otherwise, so no command can surface an unenumerated status by
//! accident (Go: `exitcode.CodeOf`).

use std::fmt;

use gage::exitcode::{Code, HasCode};

/// gage names itself in front of everything it reports, exactly once.
pub const ERROR_PREFIX: &str = "gage: ";

#[derive(Debug)]
pub struct CliError {
    code: Code,
    err: anyhow::Error,
}

impl CliError {
    pub fn new(code: Code, msg: impl fmt::Display) -> CliError {
        CliError {
            code,
            err: anyhow::anyhow!("{msg}"),
        }
    }

    /// An error that never opted into a code.
    pub fn internal(err: anyhow::Error) -> CliError {
        CliError {
            code: Code::Internal,
            err,
        }
    }

    pub fn code(&self) -> Code {
        self.code
    }

    /// Adds context in front of the message, keeping the code.
    pub fn context(self, context: impl fmt::Display + Send + Sync + 'static) -> CliError {
        CliError {
            code: self.code,
            err: self.err.context(context),
        }
    }
}

impl fmt::Display for CliError {
    // The whole chain, `outer: inner`, as one line.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#}", self.err)
    }
}

impl<E> From<E> for CliError
where
    E: HasCode + std::error::Error + Send + Sync + 'static,
{
    fn from(err: E) -> CliError {
        CliError {
            code: err.code(),
            err: err.into(),
        }
    }
}

/// Adds [`ERROR_PREFIX`] to a message that doesn't already open with it.
/// The one place errors and warnings get gage's name, so a message that
/// already carries it is never doubled to `gage: gage: ...`.
pub fn prefix_once(msg: &str) -> String {
    if msg.starts_with(ERROR_PREFIX) {
        msg.to_string()
    } else {
        format!("{ERROR_PREFIX}{msg}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("library failure")]
    struct Coded(Code);

    impl HasCode for Coded {
        fn code(&self) -> Code {
            self.0
        }
    }

    // Go: TestBareErrorIsInternal
    #[test]
    fn bare_error_is_internal() {
        let err = CliError::internal(anyhow::anyhow!("boom"));
        assert_eq!(err.code(), Code::Internal);
    }

    // Go: TestEveryDefinedCodeRoundTripsThroughConstructors (CLI half):
    // the constructor, the From conversion, and added context all keep the
    // code.
    #[test]
    fn every_code_round_trips_through_cli_error() {
        for code in gage::exitcode::all() {
            assert_eq!(CliError::new(code, "example failure").code(), code);
            let from_library: CliError = Coded(code).into();
            assert_eq!(from_library.code(), code);
            let with_context = from_library.context("context");
            assert_eq!(with_context.code(), code);
            assert_eq!(with_context.to_string(), "context: library failure");
        }
    }

    #[test]
    fn a_library_error_keeps_its_own_code() {
        let err: CliError = gage::paths::Error::InvalidVaultId("x".into()).into();
        assert_eq!(err.code(), Code::LockedOrAuth);
    }

    // Go: TestWarningsDoNotDoubleGagesOwnPrefix, rendering half. The
    // end-to-end half (a real warning through Prompter::warn) waits for
    // the milestone that adds warn; see the M0b doc.
    #[test]
    fn prefix_once_never_doubles() {
        assert_eq!(prefix_once("boom"), "gage: boom");
        assert_eq!(prefix_once("gage: boom"), "gage: boom");
        assert_eq!(
            prefix_once(&prefix_once("continuing without x")),
            "gage: continuing without x"
        );
    }
}
