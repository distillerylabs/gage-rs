//! How a command fails: an error message plus the exit code it renders as.
//!
//! Library errors arrive through `From`, carrying the code their
//! `HasCode` impl gives them. Anything else is Internal unless a handler
//! says otherwise, so no command can surface an unenumerated status by
//! accident .

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

    /// The first error of type `E` in the chain, so a caller (a test,
    /// mostly) can match a typed refusal,
    /// through any context added above it.
    pub fn downcast_ref<E>(&self) -> Option<&E>
    where
        E: std::error::Error + 'static,
    {
        self.err.chain().find_map(|cause| cause.downcast_ref::<E>())
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
    // The whole chain, `outer: inner`, as one line, except a cause the
    // layer above already put in its own message: library errors name
    // their source (`reading x: permission denied`) and expose it as
    // `source()` too, and saying it twice helps nobody.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut rendered = String::new();
        for cause in self.err.chain() {
            let msg = cause.to_string();
            if rendered.contains(&msg) {
                continue;
            }
            if !rendered.is_empty() {
                rendered.push_str(": ");
            }
            rendered.push_str(&msg);
        }
        f.write_str(&rendered)
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

    #[test]
    fn bare_error_is_internal() {
        let err = CliError::internal(anyhow::anyhow!("boom"));
        assert_eq!(err.code(), Code::Internal);
    }

    // Every defined code round-trips (CLI half):
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

    // Library errors name their source in their own message (`reading x:
    // permission denied`) and also expose it as `source()`. Rendering the
    // chain must not say the same cause twice.
    #[test]
    fn a_cause_already_in_the_message_is_not_repeated() {
        let dir = std::env::temp_dir().join("gage-cli-error-test-missing-dir");
        let lib_err =
            gage::config::write(&dir.join("config.toml"), &Default::default()).unwrap_err();
        let cause = std::error::Error::source(&lib_err).unwrap().to_string();
        let err: CliError = lib_err.into();

        let rendered = err.to_string();

        assert_eq!(rendered.matches(&cause).count(), 1, "{rendered}");
        assert!(rendered.starts_with("config: writing "), "{rendered}");

        let with_context = err.context("saving");
        assert_eq!(with_context.to_string(), format!("saving: {rendered}"));
    }

    // A typed error is found under any context, and only by its own type.
    #[test]
    fn downcast_ref_finds_a_typed_error_through_context() {
        let err = CliError::from(Coded(Code::Usage)).context("outer");
        assert_eq!(err.downcast_ref::<Coded>().unwrap().0, Code::Usage);
        assert!(err.downcast_ref::<std::io::Error>().is_none());
        assert!(
            CliError::new(Code::Usage, "plain")
                .downcast_ref::<Coded>()
                .is_none()
        );
    }

    #[test]
    fn a_library_error_keeps_its_own_code() {
        let err: CliError = gage::paths::Error::InvalidVaultId("x".into()).into();
        assert_eq!(err.code(), Code::LockedOrAuth);
    }

    // Rendering half. The
    // end-to-end half (a real warning through Prompter::warn) waits for
    // the milestone that adds warn.
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
