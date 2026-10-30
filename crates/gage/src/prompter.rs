//! How the library asks for a human decision without touching a terminal:
//! no printing, no reading stdin. `gage-cli` implements [`Prompter`]
//! against the terminal, library tests against [`FakePrompter`], and a
//! future GUI/TUI against dialogs. See "Library architecture" and
//! "Interactive decisions become data, not printed text" in the design doc.
//!
//! The shape is fixed; the unlock purpose and `warn` are its first uses. Each later
//! question (a free-form secret value, a default-yes confirmation, choosing
//! among ambiguous candidates, which version of a conflicted entry to keep,
//! a recipient-change warning) is added as a method by the feature that
//! first asks it.

use uuid::Uuid;
use zeroize::Zeroizing;

use crate::entry::CandidateList;
use crate::error::{ErrorKind, HasKind, Remedy};
use crate::sync::syncresolve::{EntryConflict, Resolution};
use crate::trust::trustcache::RecipientChangeWarning;

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
/// match on it becomes a refusal the day a second kind exists.
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
    /// The frontend's retry policy ended a candidate pick without one.
    #[error("no entry chosen after {0} attempts")]
    NoChoice(u32),
    /// Asked to choose from nothing: a caller bug, since an ambiguous
    /// query always has at least two candidates.
    #[error("asked to choose from an empty candidate list")]
    EmptyCandidates,
    /// A question only a human can answer, with nobody to ask: a script, a
    /// `--stdin` session, CI. Which version of a conflicted entry to keep
    /// (gage never picks a side on its own; a silent choice is the
    /// last-write-wins outcome the sync model exists to refuse), or whether
    /// to trust a changed recipient list.
    #[error("this question needs someone to ask, and nobody is at a terminal")]
    NotInteractive,
    /// The frontend's retry policy ended a conflict question without an
    /// answer it recognized.
    #[error("no version chosen after {0} attempts")]
    NoResolution(u32),
    /// The frontend declined to ask, for a reason of its own that it
    /// words for the person reading the failure: a scripted run with no
    /// passphrase source, or a new identity's passphrase with nobody to
    /// type it. Distinct from [`PromptError::Cancelled`], which is a human
    /// declining, so the message can say what to do about it.
    #[error("{0}")]
    Refused(String),
    #[error("prompt failed: {0}")]
    Io(#[from] std::io::Error),
}

impl HasKind for PromptError {
    // No answer to an unlock is an authentication failure, as end of input
    // is reported; a broken terminal or a request this build can't
    // render is not.
    fn kind(&self) -> ErrorKind {
        match self {
            PromptError::Cancelled
            | PromptError::TooManyAttempts
            | PromptError::NoMatchingPassphrase(_)
            | PromptError::Refused(_) => ErrorKind::LockedOrAuth,
            PromptError::NoChoice(_) => ErrorKind::Ambiguous,
            // A human must resolve it, and nobody did.
            PromptError::NotInteractive | PromptError::NoResolution(_) => ErrorKind::Conflict,
            PromptError::UnsupportedKind(_) | PromptError::EmptyCandidates | PromptError::Io(_) => {
                ErrorKind::Internal
            }
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

    /// Asks a yes/no question whose default is yes: a bare Enter accepts.
    /// `clone`'s enrollment offer is the one such question in gage, which
    /// is why it is not a flag on [`Prompter::confirm`], whose default-no
    /// every other caller relies on. There is deliberately no default body:
    /// every wrapper must delegate, and none may answer yes for a human
    /// (ADR-0033).
    fn confirm_default_yes(&self, prompt: &str) -> Result<bool, PromptError>;

    /// Asks for the secret value of the entry titled `title`, without
    /// echoing it. It is its own call rather than an unlock request: it
    /// answers with free text the library never checks, and a GUI shows a
    /// text field where an unlock shows a passphrase box.
    fn value(&self, title: &str) -> Result<Zeroizing<String>, PromptError>;

    /// Asks which of an ambiguous query's candidates was meant, and
    /// returns its id. Only a session asks: one-shot mode has nobody to
    /// ask and fails with the list instead. `Cancelled` means the human
    /// declined to pick, and the caller then treats the query as
    /// unresolved, as one-shot does. An id outside the list is refused by
    /// the caller, not trusted.
    fn choose(&self, list: &CandidateList) -> Result<Uuid, PromptError>;

    /// Asks which version of an entry both sides of a divergence changed
    /// to keep. Both versions arrive decrypted, with their provenance; a
    /// CLI renders a menu, a GUI a dialog.
    ///
    /// The default refuses with [`PromptError::NotInteractive`]: a
    /// frontend with nobody to ask implements nothing and refuses, rather
    /// than every caller remembering a flag (ADR-0021). Only a
    /// frontend that really can ask a human overrides it.
    fn resolve_conflict(&self, conflict: &EntryConflict) -> Result<Resolution, PromptError> {
        let _ = conflict;
        Err(PromptError::NotInteractive)
    }

    /// Asks whether to trust a recipient list that changed since this
    /// device last confirmed one, before anything is encrypted to it. The
    /// warning carries the diff and whether the vault's two recipient
    /// files agree; a CLI renders the design doc's diff and `[y/N]`, a GUI
    /// a dialog. `false` refuses the write.
    ///
    /// The default refuses with [`PromptError::NotInteractive`], as
    /// [`Prompter::resolve_conflict`]'s does (ADR-0033): a
    /// frontend with nobody to ask can't approve, and an unapproved list is
    /// exactly what the check won't encrypt to. What confirming *means*
    /// (regenerate the cache, or only note a mismatch was seen) is decided
    /// by the library, never by the frontend.
    fn confirm_recipient_change(
        &self,
        warning: &RecipientChangeWarning,
    ) -> Result<bool, PromptError> {
        let _ = warning;
        Err(PromptError::NotInteractive)
    }

    /// Delivers a non-fatal advisory to surface once: today, that
    /// page-locking key material failed and the unlock is proceeding
    /// without it, or that an existing identity file was reused. It
    /// returns nothing on purpose: every caller has already decided to
    /// continue, and a warning that could fail would let the library turn
    /// a degraded-but-working unlock into a failed one.
    ///
    /// A warning names no frontend: no `gage:`, no command to run. The
    /// frontend adds its own name and renders the remedy (ADR-0054).
    fn warn(&self, warning: &Warning);
}

/// A non-fatal advisory: what happened, and what would address it if
/// anything does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    pub text: String,
    pub remedy: Option<Remedy>,
}

impl Warning {
    /// The same warning, offering `remedy`.
    pub fn with_remedy(self, remedy: Option<Remedy>) -> Warning {
        Warning { remedy, ..self }
    }
}

impl From<String> for Warning {
    fn from(text: String) -> Warning {
        Warning { text, remedy: None }
    }
}

impl From<&str> for Warning {
    fn from(text: &str) -> Warning {
        Warning::from(text.to_string())
    }
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
        confirms: Mutex<VecDeque<Result<bool, PromptError>>>,
        confirms_default_yes: Mutex<VecDeque<Result<bool, PromptError>>>,
        values: Mutex<VecDeque<Result<String, PromptError>>>,
        choices: Mutex<VecDeque<Choice>>,
        /// `None` until a resolution is scripted: a fake nobody scripted
        /// answers is a frontend with nobody to ask, and refuses as the
        /// trait's default does.
        resolutions: Mutex<Option<Resolutions>>,
        /// `None` until an answer is scripted: unscripted, the fake is a
        /// frontend with nobody to ask, and refuses as the default does.
        recipient_answers: Mutex<Option<VecDeque<bool>>>,
        asked_recipient: Mutex<Vec<RecipientChangeWarning>>,
        asked_unlock: Mutex<Vec<UnlockRequest>>,
        asked_confirm: Mutex<Vec<String>>,
        asked_confirm_default_yes: Mutex<Vec<String>>,
        asked_value: Mutex<Vec<String>>,
        asked_choose: Mutex<Vec<CandidateList>>,
        asked_resolve: Mutex<Vec<EntryConflict>>,
        warnings: Mutex<Vec<Warning>>,
    }

    /// The scripted answers to [`Prompter::resolve_conflict`]: in order, or
    /// by the conflicted entry's title. Conflicts are presented in path
    /// order, which is UUID order, which no test can predict, so a test
    /// giving two conflicts different answers addresses them by title.
    #[derive(Debug, Default)]
    struct Resolutions {
        in_order: VecDeque<Result<Resolution, PromptError>>,
        by_title: std::collections::BTreeMap<String, Resolution>,
    }

    /// One scripted answer to [`Prompter::choose`].
    #[derive(Debug)]
    enum Choice {
        /// The candidate at this position in the list offered, as a human
        /// typing its number would pick it.
        Index(usize),
        /// This id, whether or not it was offered: what a buggy frontend
        /// looks like from the library's side.
        Id(Uuid),
        Err(PromptError),
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
            self.confirms.lock().unwrap().push_back(Ok(answer));
            self
        }

        /// Answers the next default-yes confirmation.
        pub fn with_confirm_default_yes(self, answer: bool) -> FakePrompter {
            self.confirms_default_yes
                .lock()
                .unwrap()
                .push_back(Ok(answer));
            self
        }

        /// The default-yes questions asked, in order.
        pub fn confirm_default_yes_prompts(&self) -> Vec<String> {
            self.asked_confirm_default_yes.lock().unwrap().clone()
        }

        /// Fails the next confirmation with `err`.
        pub fn with_confirm_error(self, err: PromptError) -> FakePrompter {
            self.confirms.lock().unwrap().push_back(Err(err));
            self
        }

        /// Answers the next recipient-change questions with `answers`, in
        /// order. Scripting none still marks the fake as a frontend that
        /// can be asked: a question then fails the test.
        pub fn with_recipient_answers(self, answers: &[bool]) -> FakePrompter {
            self.recipient_answers
                .lock()
                .unwrap()
                .get_or_insert_with(VecDeque::new)
                .extend(answers);
            self
        }

        /// Every recipient-change warning handed over, in order.
        pub fn recipient_changes(&self) -> Vec<RecipientChangeWarning> {
            self.asked_recipient.lock().unwrap().clone()
        }

        pub fn with_value(self, value: &str) -> FakePrompter {
            self.values.lock().unwrap().push_back(Ok(value.to_string()));
            self
        }

        pub fn with_value_error(self, err: PromptError) -> FakePrompter {
            self.values.lock().unwrap().push_back(Err(err));
            self
        }

        /// Picks the candidate at `index` (from 0) of the next list offered.
        pub fn with_choice(self, index: usize) -> FakePrompter {
            self.choices.lock().unwrap().push_back(Choice::Index(index));
            self
        }

        /// Answers the next pick with `id`, offered or not.
        pub fn with_chosen_id(self, id: Uuid) -> FakePrompter {
            self.choices.lock().unwrap().push_back(Choice::Id(id));
            self
        }

        pub fn with_choose_error(self, err: PromptError) -> FakePrompter {
            self.choices.lock().unwrap().push_back(Choice::Err(err));
            self
        }

        /// Answers the next conflicts with `answers`, in order.
        pub fn with_resolutions(self, answers: &[Resolution]) -> FakePrompter {
            self.resolutions_mut(|r| r.in_order.extend(answers.iter().map(|a| Ok(*a))));
            self
        }

        /// Answers any conflict over an entry titled `title` with `answer`.
        pub fn with_resolution_for(self, title: &str, answer: Resolution) -> FakePrompter {
            self.resolutions_mut(|r| {
                r.by_title.insert(title.to_string(), answer);
            });
            self
        }

        /// Fails the next conflict question with `err`.
        pub fn with_resolution_error(self, err: PromptError) -> FakePrompter {
            self.resolutions_mut(|r| r.in_order.push_back(Err(err)));
            self
        }

        fn resolutions_mut(&self, f: impl FnOnce(&mut Resolutions)) {
            f(self
                .resolutions
                .lock()
                .unwrap()
                .get_or_insert_with(Resolutions::default));
        }

        /// Every conflict presented, in order.
        pub fn conflicts_presented(&self) -> Vec<EntryConflict> {
            self.asked_resolve.lock().unwrap().clone()
        }

        /// Every candidate list offered, in order.
        pub fn choose_lists(&self) -> Vec<CandidateList> {
            self.asked_choose.lock().unwrap().clone()
        }

        /// The title of every entry whose value was asked for.
        pub fn value_prompts(&self) -> Vec<String> {
            self.asked_value.lock().unwrap().clone()
        }

        pub fn unlock_requests(&self) -> Vec<UnlockRequest> {
            self.asked_unlock.lock().unwrap().clone()
        }

        pub fn confirm_prompts(&self) -> Vec<String> {
            self.asked_confirm.lock().unwrap().clone()
        }

        /// Every warning's text, in order.
        pub fn warnings(&self) -> Vec<String> {
            self.warned().into_iter().map(|w| w.text).collect()
        }

        /// Every warning delivered, remedy included, in order.
        pub fn warned(&self) -> Vec<Warning> {
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
            self.confirms
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("FakePrompter: unscripted confirm {prompt:?}"))
        }

        fn confirm_default_yes(&self, prompt: &str) -> Result<bool, PromptError> {
            self.asked_confirm_default_yes
                .lock()
                .unwrap()
                .push(prompt.to_string());
            self.confirms_default_yes
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| {
                    panic!("FakePrompter: unscripted default-yes confirm {prompt:?}")
                })
        }

        fn value(&self, title: &str) -> Result<Zeroizing<String>, PromptError> {
            self.asked_value.lock().unwrap().push(title.to_string());
            self.values
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("FakePrompter: unscripted value request for {title:?}"))
                .map(Zeroizing::new)
        }

        fn choose(&self, list: &CandidateList) -> Result<Uuid, PromptError> {
            self.asked_choose.lock().unwrap().push(list.clone());
            let answer = self
                .choices
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("FakePrompter: unscripted choose over {list:?}"));
            match answer {
                Choice::Index(i) => Ok(list
                    .candidates
                    .get(i)
                    .unwrap_or_else(|| panic!("FakePrompter: no candidate {i} in {list:?}"))
                    .id),
                Choice::Id(id) => Ok(id),
                Choice::Err(e) => Err(e),
            }
        }

        fn warn(&self, warning: &Warning) {
            self.warnings.lock().unwrap().push(warning.clone());
        }

        fn confirm_recipient_change(
            &self,
            w: &RecipientChangeWarning,
        ) -> Result<bool, PromptError> {
            self.asked_recipient.lock().unwrap().push(w.clone());
            let mut scripted = self.recipient_answers.lock().unwrap();
            let Some(scripted) = scripted.as_mut() else {
                return Err(PromptError::NotInteractive);
            };
            Ok(scripted.pop_front().unwrap_or_else(|| {
                panic!(
                    "FakePrompter: unscripted recipient change for {:?}",
                    w.vault
                )
            }))
        }

        fn resolve_conflict(&self, c: &EntryConflict) -> Result<Resolution, PromptError> {
            self.asked_resolve.lock().unwrap().push(c.clone());
            let mut scripted = self.resolutions.lock().unwrap();
            let Some(scripted) = scripted.as_mut() else {
                return Err(PromptError::NotInteractive);
            };
            let title = c.title();
            if let Some(answer) = scripted.by_title.get(title) {
                return Ok(*answer);
            }
            scripted
                .in_order
                .pop_front()
                .unwrap_or_else(|| panic!("FakePrompter: unscripted conflict over {title:?}"))
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
        p.warn(&Warning::from("careful").with_remedy(Some(Remedy::Sync)));
        assert_eq!(p.warnings(), vec!["careful".to_string()]);
        assert_eq!(p.warned()[0].remedy, Some(Remedy::Sync));
    }

    #[test]
    fn prompt_errors_carry_their_exit_codes() {
        for (err, code) in [
            (PromptError::Cancelled, ErrorKind::LockedOrAuth),
            (PromptError::TooManyAttempts, ErrorKind::LockedOrAuth),
            (
                PromptError::NoMatchingPassphrase(3),
                ErrorKind::LockedOrAuth,
            ),
            (
                PromptError::Refused("no way to ask".into()),
                ErrorKind::LockedOrAuth,
            ),
            (
                PromptError::UnsupportedKind("yubikey".into()),
                ErrorKind::Internal,
            ),
            (PromptError::NoChoice(3), ErrorKind::Ambiguous),
            (PromptError::EmptyCandidates, ErrorKind::Internal),
            (PromptError::NotInteractive, ErrorKind::Conflict),
            (PromptError::NoResolution(5), ErrorKind::Conflict),
            (
                PromptError::Io(std::io::Error::other("tty gone")),
                ErrorKind::Internal,
            ),
        ] {
            assert_eq!(err.kind(), code, "{err}");
        }
        assert_eq!(Kind::Passphrase.to_string(), "passphrase");
    }

    #[test]
    fn fake_prompter_scripts_values_and_records_whose_were_asked() {
        let p = FakePrompter::new()
            .with_value("s3cret")
            .with_value_error(PromptError::Cancelled);
        assert_eq!(&*p.value("ProtonMail").unwrap(), "s3cret");
        assert!(matches!(p.value("Other"), Err(PromptError::Cancelled)));
        assert_eq!(p.value_prompts(), vec!["ProtonMail", "Other"]);
    }

    #[test]
    #[should_panic(expected = "unscripted value request")]
    fn an_unscripted_value_request_fails_the_test() {
        let _ = FakePrompter::new().value("x");
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
