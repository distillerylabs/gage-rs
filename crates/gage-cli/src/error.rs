//! How a command fails: an error message plus the exit code it renders as.
//!
//! Library errors arrive through `From`, carrying the code their
//! `HasKind` impl's kind maps to. Anything else is Internal unless a handler
//! says otherwise, so no command can surface an unenumerated status by
//! accident .

use std::fmt;

use gage::{HasKind, Remedy};

use crate::exitcode::ExitCode;

/// gage names itself in front of everything it reports, exactly once.
pub const ERROR_PREFIX: &str = "gage: ";

#[derive(Debug)]
pub struct CliError {
    code: ExitCode,
    err: anyhow::Error,
    /// What the library error said would fix it, rendered after the
    /// message. Taken at conversion: the chain can't be asked later.
    remedy: Option<Remedy>,
}

impl CliError {
    pub fn new(code: ExitCode, msg: impl fmt::Display) -> CliError {
        CliError {
            code,
            err: anyhow::anyhow!("{msg}"),
            remedy: None,
        }
    }

    /// An error that never opted into a code.
    pub fn internal(err: anyhow::Error) -> CliError {
        CliError {
            code: ExitCode::Internal,
            err,
            remedy: None,
        }
    }

    pub fn code(&self) -> ExitCode {
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

    /// The same error, offering `remedy` after its message: for an error
    /// the CLI builds from a library value that carries one.
    pub fn with_remedy(self, remedy: Option<Remedy>) -> CliError {
        CliError {
            remedy: remedy.or(self.remedy),
            ..self
        }
    }

    /// Adds context in front of the message, keeping the code.
    pub fn context(self, context: impl fmt::Display + Send + Sync + 'static) -> CliError {
        CliError {
            code: self.code,
            err: self.err.context(context),
            remedy: self.remedy,
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
        if let Some(remedy) = &self.remedy {
            rendered.push_str(&crate::remedy::render(remedy));
        }
        f.write_str(&rendered)
    }
}

impl<E> From<E> for CliError
where
    E: HasKind + std::error::Error + Send + Sync + 'static,
{
    fn from(err: E) -> CliError {
        CliError {
            code: ExitCode::from(err.kind()),
            remedy: err.remedy(),
            err: err.into(),
        }
    }
}

/// An error as stderr shows it: [`ERROR_PREFIX`], then the message. No
/// message arrives carrying gage's name (ADR-0054), so this is the one
/// place an error gets it, exactly once.
pub fn prefixed(msg: impl fmt::Display) -> String {
    format!("{ERROR_PREFIX}{msg}")
}

#[cfg(test)]
mod tests {
    use super::*;

    use gage::ErrorKind;

    #[derive(Debug, thiserror::Error)]
    #[error("library failure")]
    struct Coded(ErrorKind);

    impl HasKind for Coded {
        fn kind(&self) -> ErrorKind {
            self.0
        }
    }

    #[test]
    fn bare_error_is_internal() {
        let err = CliError::internal(anyhow::anyhow!("boom"));
        assert_eq!(err.code(), ExitCode::Internal);
    }

    // Every defined code round-trips (CLI half):
    // the constructor, the From conversion, and added context all keep the
    // code.
    #[test]
    fn every_code_round_trips_through_cli_error() {
        for code in ExitCode::ALL {
            assert_eq!(CliError::new(code, "example failure").code(), code);
        }
        for kind in ErrorKind::ALL {
            let code = ExitCode::from(kind);
            let from_library: CliError = Coded(kind).into();
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
            gage::storage::config::save(&dir.join("config.toml"), &Default::default()).unwrap_err();
        let cause = std::error::Error::source(&lib_err).unwrap().to_string();
        let err: CliError = lib_err.into();

        let rendered = err.to_string();

        assert_eq!(rendered.matches(&cause).count(), 1, "{rendered}");
        assert!(rendered.starts_with("writing "), "{rendered}");

        let with_context = err.context("saving");
        assert_eq!(with_context.to_string(), format!("saving: {rendered}"));
    }

    // A typed error is found under any context, and only by its own type.
    #[test]
    fn downcast_ref_finds_a_typed_error_through_context() {
        let err = CliError::from(Coded(ErrorKind::Usage)).context("outer");
        assert_eq!(err.downcast_ref::<Coded>().unwrap().0, ErrorKind::Usage);
        assert!(err.downcast_ref::<std::io::Error>().is_none());
        assert!(
            CliError::new(ExitCode::Usage, "plain")
                .downcast_ref::<Coded>()
                .is_none()
        );
    }

    #[test]
    fn a_library_error_keeps_its_own_code() {
        let err: CliError = gage::storage::paths::Error::InvalidDeviceName("x".into()).into();
        assert_eq!(err.code(), ExitCode::LockedOrAuth);
    }

    // Every kind of error gets gage's name exactly once, at the front: one
    // from the library, one the CLI made, one with context added, and a
    // script line's.
    #[test]
    fn every_error_is_prefixed_exactly_once() {
        let library: CliError = gage::storage::paths::Error::InvalidDeviceName("x".into()).into();
        let made = CliError::new(ExitCode::Usage, "bad flag");
        let wrapped = CliError::new(ExitCode::Usage, "bad flag").context("saving");
        let script = crate::repl::script_error(
            CliError::from(Coded(ErrorKind::NotFound)),
            "commands.gage",
            3,
            "show Foo",
        );
        for err in [&library, &made, &wrapped, &script] {
            let shown = prefixed(err);
            assert!(shown.starts_with(ERROR_PREFIX), "{shown}");
            assert_eq!(shown.matches(ERROR_PREFIX).count(), 1, "{shown}");
        }
        assert_eq!(prefixed(&wrapped), "gage: saving: bad flag");
        assert_eq!(
            prefixed(&script),
            "gage: commands.gage:3: show Foo: library failure"
        );
        // The script's exit code is its command's, from the typed error.
        assert_eq!(script.code(), ExitCode::NotFound);
    }
}
