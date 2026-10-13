//! How the library asks for a human decision without touching a terminal:
//! no printing, no reading stdin. `gage-cli` implements [`Prompter`]
//! against the terminal, library tests against [`FakePrompter`], and a
//! future GUI/TUI against dialogs. See "Library architecture" and
//! "Interactive decisions become data, not printed text" in the design doc.
//!
//! M0b fixes the shape with the two questions it needs; each later question
//! (a free-form secret value, a default-yes confirmation, choosing among
//! ambiguous candidates, a recipient-change warning) is added as a method
//! by the milestone that first asks it.

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

/// What `Vault::unlock` hands a [`Prompter`] when it needs a human (or a
/// GUI, or a script) to prove this device can decrypt a vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnlockRequest {
    pub kind: Kind,
    pub vault: String,
    pub device: String,
    /// Counts tries within one unlock, from 1, so a frontend can render
    /// "wrong passphrase, try again" without the library deciding a retry
    /// policy for it.
    pub attempt: u32,
}

/// A [`Prompter`]'s answer to an [`UnlockRequest`]: one variant per
/// [`Kind`], each carrying what that method needs.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnlockResponse {
    // A zeroizing secret type replaces the String when M2 brings real key
    // material.
    Passphrase(String),
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
pub enum PromptError {
    /// The human declined to answer (end of input, an interrupt, a closed
    /// dialog).
    #[error("cancelled")]
    Cancelled,
    #[error("prompt failed: {0}")]
    Io(#[from] std::io::Error),
}

impl HasCode for PromptError {
    fn code(&self) -> Code {
        Code::Internal
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
    }

    impl FakePrompter {
        pub fn new() -> FakePrompter {
            FakePrompter::default()
        }

        pub fn with_passphrase(self, passphrase: &str) -> FakePrompter {
            self.unlocks
                .lock()
                .unwrap()
                .push_back(Ok(UnlockResponse::Passphrase(passphrase.to_string())));
            self
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> UnlockRequest {
        UnlockRequest {
            kind: Kind::Passphrase,
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
        assert_eq!(first, UnlockResponse::Passphrase("one".into()));
        assert_eq!(first.kind(), Kind::Passphrase);
        assert!(matches!(p.unlock(&request()), Err(PromptError::Cancelled)));
        assert!(p.confirm("remove it?").unwrap());
        assert_eq!(p.unlock_requests(), vec![request(), request()]);
        assert_eq!(p.confirm_prompts(), vec!["remove it?".to_string()]);
    }

    #[test]
    #[should_panic(expected = "unscripted confirm")]
    fn an_unscripted_question_fails_the_test() {
        let _ = FakePrompter::new().confirm("delete everything?");
    }

    #[test]
    fn a_passphrase_never_reaches_debug_output() {
        let r = UnlockResponse::Passphrase("hunter2".into());
        assert!(!format!("{r:?}").contains("hunter2"));
    }
}
