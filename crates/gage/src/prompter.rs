//! How the library asks for a human decision without touching a terminal:
//! no printing, no reading stdin. `gage-cli` implements [`Prompter`]
//! against the terminal, library tests against [`FakePrompter`], and a
//! future GUI/TUI against dialogs. See "Library architecture" and
//! "Interactive decisions become data, not printed text" in the design doc.
//!
//! M0b fixed the shape; M2 adds the unlock purpose and `warn`. Each later
//! question (a free-form secret value, a default-yes confirmation, choosing
//! among ambiguous candidates, a recipient-change warning) is added as a
//! method by the milestone that first asks it.

use zeroize::Zeroizing;

use crate::exitcode::{Code, HasCode};

/// The identity method an [`UnlockRequest`] is asking about. Unlocking is
/// one method-agnostic exchange rather than a passphrase-specific call, so
/// a second method (a hardware key) is an added variant, not a new trait
/// method. Only `Passphrase` exists today; see "Decryption methods are
/// per-device" in the design doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Kind {
    Passphrase,
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Kind::Passphrase => f.write_str("passphrase"),
        }
    }
}

/// Which direction an [`UnlockRequest`] runs in. Both need the same answer
/// type but not the same presentation: a CLI asks for a new passphrase
/// twice and checks the two match, a GUI shows a strength meter. So the
/// difference is data a frontend renders on, not a second method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Open an identity that already exists.
    Unlock,
    /// Choose the secret a brand-new identity will be protected with.
    /// There is nothing to check the answer against, so a typo here is
    /// unrecoverable, which is why it is marked rather than left
    /// indistinguishable from an unlock.
    Create,
}

/// What `Vault::unlock` hands a [`Prompter`] when it needs a human (or a
/// GUI, or a script) to prove this device can decrypt a vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnlockRequest {
    pub kind: Kind,
    pub purpose: Purpose,
    pub vault: String,
    pub device: String,
    /// Counts tries within one unlock, from 1, so a frontend can render
    /// "wrong passphrase, try again" without the library deciding a retry
    /// policy for it.
    pub attempt: u32,
}

/// A [`Prompter`]'s answer to an [`UnlockRequest`]: one variant per
/// [`Kind`], each carrying what that method needs. Secrets are wiped when
/// the response is dropped.
///
/// Its variants mirror [`Kind`]'s one to one, so a response of a different
/// kind than the request asked for can't be built today; the library's
/// match on it becomes a refusal the day a second kind exists (Go's
/// `TestUnlockRejectsAPrompterAnsweringTheWrongKind`).
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnlockResponse {
    Passphrase(Zeroizing<String>),
}

impl UnlockResponse {
    /// A passphrase answer.
    pub fn passphrase(p: impl Into<String>) -> UnlockResponse {
        UnlockResponse::Passphrase(Zeroizing::new(p.into()))
    }
}

impl UnlockResponse {
    pub fn kind(&self) -> Kind {
        match self {
            UnlockResponse::Passphrase(_) => Kind::Passphrase,
        }
    }
}

// Never print the secret, not even in a test failure.
impl std::fmt::Debug for UnlockResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnlockResponse::Passphrase(_) => f.write_str("Passphrase(<redacted>)"),
        }
    }
}

/// Why a [`Prompter`] produced no answer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PromptError {
    /// The human declined to answer (end of input, an interrupt, a closed
    /// dialog).
    #[error("cancelled")]
    Cancelled,
    /// The frontend's retry policy ended an unlock: its way of telling a
    /// library that keeps asking for as long as it is answered to stop.
    #[error("too many incorrect passphrase attempts")]
    TooManyAttempts,
    /// A new passphrase was never entered the same way twice.
    #[error("gave up after {0} attempts to enter a matching passphrase")]
    NoMatchingPassphrase(u32),
    /// The frontend can't satisfy this kind of unlock request.
    #[error("this build cannot satisfy a {0:?} unlock request")]
    UnsupportedKind(String),
    #[error("prompt failed: {0}")]
    Io(#[from] std::io::Error),
}

impl HasCode for PromptError {
    // No answer to an unlock is an authentication failure, as Go's prompter
    // reports end of input; a broken terminal or a request this build can't
    // render is not.
    fn code(&self) -> Code {
        match self {
            PromptError::Cancelled
            | PromptError::TooManyAttempts
            | PromptError::NoMatchingPassphrase(_) => Code::LockedOrAuth,
            PromptError::UnsupportedKind(_) | PromptError::Io(_) => Code::Internal,
        }
    }
}

/// Every human decision the library needs, as typed calls.
pub trait Prompter {
    /// Asks for whatever proves this device holds the private key the
    /// vault's method expects.
    fn unlock(&self, req: &UnlockRequest) -> Result<UnlockResponse, PromptError>;

    /// Asks a yes/no question whose default is no. `false` means the
    /// caller should abort what it was about to do.
    fn confirm(&self, prompt: &str) -> Result<bool, PromptError>;

    /// Delivers a non-fatal advisory to surface once: today, that
    /// page-locking key material failed and the unlock is proceeding
    /// without it, or that an existing identity file was reused. It
    /// returns nothing on purpose: every caller has already decided to
    /// continue, and a warning that could fail would let the library turn
    /// a degraded-but-working unlock into a failed one.
    ///
    /// Messages name gage themselves (`gage: ...`); a frontend shows them
    /// as they are.
    fn warn(&self, msg: &str);
}

#[cfg(any(test, feature = "test-support"))]
pub use fake::FakePrompter;

#[cfg(any(test, feature = "test-support"))]
mod fake {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    /// A scripted [`Prompter`] for tests: answers come from queues the test
    /// fills, and every question asked is recorded. An unscripted question
    /// panics, so a test can't pass by accident on a prompt it never
    /// expected.
    #[derive(Debug, Default)]
    pub struct FakePrompter {
        unlocks: Mutex<VecDeque<Result<UnlockResponse, PromptError>>>,
        confirms: Mutex<VecDeque<bool>>,
        asked_unlock: Mutex<Vec<UnlockRequest>>,
        asked_confirm: Mutex<Vec<String>>,
        warnings: Mutex<Vec<String>>,
    }

    impl FakePrompter {
        pub fn new() -> FakePrompter {
            FakePrompter::default()
        }

        pub fn with_passphrase(self, passphrase: &str) -> FakePrompter {
            self.unlocks
                .lock()
                .unwrap()
                .push_back(Ok(UnlockResponse::passphrase(passphrase)));
            self
        }

        /// One answer per passphrase, in order.
        pub fn with_passphrases(self, passphrases: &[&str]) -> FakePrompter {
            passphrases
                .iter()
                .fold(self, |p, pass| p.with_passphrase(pass))
        }

        pub fn with_unlock_error(self, err: PromptError) -> FakePrompter {
            self.unlocks.lock().unwrap().push_back(Err(err));
            self
        }

        pub fn with_confirm(self, answer: bool) -> FakePrompter {
            self.confirms.lock().unwrap().push_back(answer);
            self
        }

        pub fn unlock_requests(&self) -> Vec<UnlockRequest> {
            self.asked_unlock.lock().unwrap().clone()
        }

        pub fn confirm_prompts(&self) -> Vec<String> {
            self.asked_confirm.lock().unwrap().clone()
        }

        /// Every warning delivered, in order.
        pub fn warnings(&self) -> Vec<String> {
            self.warnings.lock().unwrap().clone()
        }
    }

    impl Prompter for FakePrompter {
        fn unlock(&self, req: &UnlockRequest) -> Result<UnlockResponse, PromptError> {
            self.asked_unlock.lock().unwrap().push(req.clone());
            self.unlocks
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("FakePrompter: unscripted unlock request {req:?}"))
        }

        fn confirm(&self, prompt: &str) -> Result<bool, PromptError> {
            self.asked_confirm.lock().unwrap().push(prompt.to_string());
            Ok(self
                .confirms
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("FakePrompter: unscripted confirm {prompt:?}")))
        }

        fn warn(&self, msg: &str) {
            self.warnings.lock().unwrap().push(msg.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> UnlockRequest {
        UnlockRequest {
            kind: Kind::Passphrase,
            purpose: Purpose::Unlock,
            vault: "personal".into(),
            device: "laptop-1".into(),
            attempt: 1,
        }
    }

    #[test]
    fn fake_prompter_answers_in_order_and_records_questions() {
        let p = FakePrompter::new()
            .with_passphrase("one")
            .with_unlock_error(PromptError::Cancelled)
            .with_confirm(true);

        let first = p.unlock(&request()).unwrap();
        assert_eq!(first, UnlockResponse::passphrase("one"));
        assert_eq!(first.kind(), Kind::Passphrase);
        assert!(matches!(p.unlock(&request()), Err(PromptError::Cancelled)));
        assert!(p.confirm("remove it?").unwrap());
        assert_eq!(p.unlock_requests(), vec![request(), request()]);
        assert_eq!(p.confirm_prompts(), vec!["remove it?".to_string()]);
        p.warn("gage: careful");
        assert_eq!(p.warnings(), vec!["gage: careful".to_string()]);
    }

    #[test]
    fn prompt_errors_carry_their_exit_codes() {
        for (err, code) in [
            (PromptError::Cancelled, Code::LockedOrAuth),
            (PromptError::TooManyAttempts, Code::LockedOrAuth),
            (PromptError::NoMatchingPassphrase(3), Code::LockedOrAuth),
            (
                PromptError::UnsupportedKind("yubikey".into()),
                Code::Internal,
            ),
            (
                PromptError::Io(std::io::Error::other("tty gone")),
                Code::Internal,
            ),
        ] {
            assert_eq!(err.code(), code, "{err}");
        }
        assert_eq!(Kind::Passphrase.to_string(), "passphrase");
    }

    #[test]
    #[should_panic(expected = "unscripted confirm")]
    fn an_unscripted_question_fails_the_test() {
        let _ = FakePrompter::new().confirm("delete everything?");
    }

    #[test]
    fn a_passphrase_never_reaches_debug_output() {
        let r = UnlockResponse::passphrase("hunter2");
        assert!(!format!("{r:?}").contains("hunter2"));
    }
}
