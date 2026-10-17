//! Where a non-interactive run's passphrase comes from: not a library
//! concept, but this frontend deciding what to do when nobody is at the
//! prompt (Q-SCRIPT-PASSPHRASE). The library still just asks through a
//! [`Prompter`] and has no idea an environment variable exists.
//!
//! Three wrappers, each answering one thing and delegating the rest, shaped
//! like [`crate::assumeyes::AssumeYes`]:
//!
//! - [`EnvPrompter`] answers an unlock from `GAGE_PASSPHRASE`, once.
//! - [`RefusingPrompter`] refuses every unlock, naming the variable.
//! - [`NoCreatePrompter`] refuses a *new* identity's passphrase, for a run
//!   with no human to type it.
//!
//! [`for_script`] picks among them in Go's fixed order. What every branch
//! has in common is the property that matters: no path reads the command
//! stream as a passphrase. Under `--stdin` the prompter's input *is* that
//! stream, so a read there takes the next command line, and for a new
//! identity that line would become the passphrase protecting a device key
//! nobody can ever type again.

use gage::entry::CandidateList;
use gage::prompter::{Kind, PromptError, Prompter, Purpose, UnlockRequest, UnlockResponse};
use gage::syncresolve::{EntryConflict, Resolution};
use gage::trustcache::RecipientChangeWarning;
use uuid::Uuid;
use zeroize::Zeroizing;

/// The variable a non-interactive run reads its passphrase from.
pub const ENV_VAR: &str = "GAGE_PASSPHRASE";

/// What `--stdin`'s command stream is called in messages.
pub const STDIN_SOURCE: &str = "<stdin>";

/// The prompter methods a wrapper never answers, passed to the prompter it
/// wraps. Written once, so a method added to the trait is added here and
/// not forgotten in three places.
macro_rules! delegate_the_rest {
    ($inner:tt) => {
        fn confirm(&self, prompt: &str) -> Result<bool, PromptError> {
            self.$inner.confirm(prompt)
        }

        fn confirm_default_yes(&self, prompt: &str) -> Result<bool, PromptError> {
            self.$inner.confirm_default_yes(prompt)
        }

        fn value(&self, title: &str) -> Result<Zeroizing<String>, PromptError> {
            self.$inner.value(title)
        }

        fn choose(&self, list: &CandidateList) -> Result<Uuid, PromptError> {
            self.$inner.choose(list)
        }

        fn resolve_conflict(&self, conflict: &EntryConflict) -> Result<Resolution, PromptError> {
            self.$inner.resolve_conflict(conflict)
        }

        fn confirm_recipient_change(
            &self,
            warning: &RecipientChangeWarning,
        ) -> Result<bool, PromptError> {
            self.$inner.confirm_recipient_change(warning)
        }

        fn warn(&self, msg: &str) {
            self.$inner.warn(msg);
        }
    };
}

/// Hands every call to the prompter it wraps. Where a new identity's
/// passphrase goes when a human can be asked.
pub struct Passthrough<'p>(pub &'p dyn Prompter);

impl Prompter for Passthrough<'_> {
    fn unlock(&self, req: &UnlockRequest) -> Result<UnlockResponse, PromptError> {
        self.0.unlock(req)
    }

    delegate_the_rest!(0);
}

/// Answers an unlock from `GAGE_PASSPHRASE`.
///
/// Never for a *new* identity: there is nothing to check a create-purpose
/// answer against, so a typo in the variable would be unrecoverable, and
/// nobody would find out until the next time the vault had to be opened.
/// `init` and `identity add` keep asking a human, twice. `Purpose` already
/// draws that line, so this is a switch on data that is there.
pub struct EnvPrompter<'p> {
    inner: &'p dyn Prompter,
    /// Where a create-purpose request goes: a human, or a refusal. See
    /// [`for_script`].
    create: Box<dyn Prompter + 'p>,
    passphrase: Zeroizing<String>,
}

impl<'p> EnvPrompter<'p> {
    pub fn new(
        inner: &'p dyn Prompter,
        create: Box<dyn Prompter + 'p>,
        passphrase: Zeroizing<String>,
    ) -> EnvPrompter<'p> {
        EnvPrompter {
            inner,
            create,
            passphrase,
        }
    }
}

impl Prompter for EnvPrompter<'_> {
    fn unlock(&self, req: &UnlockRequest) -> Result<UnlockResponse, PromptError> {
        if req.purpose == Purpose::Create {
            return self.create.unlock(req);
        }
        // The variable gives the same answer every time, so a second
        // attempt could only fail identically. Refusing it is how this
        // frontend says "no retry", as the terminal prompter's ceiling
        // says "three".
        if req.attempt > 1 {
            return Err(PromptError::Refused(format!(
                "{ENV_VAR} did not unlock vault {:?}",
                req.vault
            )));
        }
        match req.kind {
            Kind::Passphrase => Ok(UnlockResponse::passphrase(self.passphrase.as_str())),
            other => Err(PromptError::UnsupportedKind(other.to_string())),
        }
    }

    delegate_the_rest!(inner);
}

/// What a non-interactive run with no passphrase source unlocks through:
/// it doesn't.
///
/// The alternative, letting the ordinary prompter read a stdin that is
/// either exhausted or the command stream, produces a hang or an empty
/// passphrase reported as a wrong one. Both are worse than being told
/// plainly that there was nowhere to get a passphrase from.
pub struct RefusingPrompter<'p> {
    inner: &'p dyn Prompter,
    create: Box<dyn Prompter + 'p>,
}

impl<'p> RefusingPrompter<'p> {
    pub fn new(inner: &'p dyn Prompter, create: Box<dyn Prompter + 'p>) -> RefusingPrompter<'p> {
        RefusingPrompter { inner, create }
    }
}

impl Prompter for RefusingPrompter<'_> {
    fn unlock(&self, req: &UnlockRequest) -> Result<UnlockResponse, PromptError> {
        if req.purpose == Purpose::Create {
            return self.create.unlock(req);
        }
        Err(PromptError::Refused(format!(
            "no way to unlock vault {:?} without a terminal: set {ENV_VAR}",
            req.vault
        )))
    }

    delegate_the_rest!(inner);
}

/// Refuses to invent a passphrase for a *new* identity when there is no
/// human to type one.
///
/// `GAGE_PASSPHRASE` deliberately doesn't answer a create-purpose request,
/// which passes it to a human. That works under `--script FILE` on a
/// terminal and is exactly wrong under `--stdin`, where the ordinary
/// prompter's input is the command stream: `identity add` would print
/// "Choose a passphrase:" and then read the script, taking a command line
/// as the passphrase for a new device key.
///
/// Every other request goes through: an ordinary unlock has a passphrase
/// that can be checked, and refusing those would break every scripted run.
pub struct NoCreatePrompter<'p>(pub &'p dyn Prompter);

impl Prompter for NoCreatePrompter<'_> {
    fn unlock(&self, req: &UnlockRequest) -> Result<UnlockResponse, PromptError> {
        if req.purpose != Purpose::Create {
            return self.0.unlock(req);
        }
        Err(PromptError::Refused(format!(
            "creating an identity for vault {:?} needs a passphrase typed by a human, \
             and this run has no terminal to ask on; run it from a shell instead \
             ({ENV_VAR} is deliberately not used for a new identity)",
            req.vault
        )))
    }

    delegate_the_rest!(0);
}

/// Declines to pick among an ambiguous query's candidates when there is no
/// human to pick, so the query fails with its list, as one-shot's does.
///
/// The ordinary prompter would read a number off its input, which under
/// `--stdin` is the command stream: it would take the next command line
/// for an answer, ask again, and swallow up to five lines before giving
/// up. `Cancelled` is what the session already reads as "the human
/// declined to pick": the query is unresolved and the error carries the
/// candidates. Every other request goes through.
pub struct NoChoosePrompter<'p>(pub &'p dyn Prompter);

impl Prompter for NoChoosePrompter<'_> {
    fn unlock(&self, req: &UnlockRequest) -> Result<UnlockResponse, PromptError> {
        self.0.unlock(req)
    }

    fn choose(&self, _: &CandidateList) -> Result<Uuid, PromptError> {
        Err(PromptError::Cancelled)
    }

    fn confirm(&self, prompt: &str) -> Result<bool, PromptError> {
        self.0.confirm(prompt)
    }

    fn confirm_default_yes(&self, prompt: &str) -> Result<bool, PromptError> {
        self.0.confirm_default_yes(prompt)
    }

    fn value(&self, title: &str) -> Result<Zeroizing<String>, PromptError> {
        self.0.value(title)
    }

    fn resolve_conflict(&self, conflict: &EntryConflict) -> Result<Resolution, PromptError> {
        self.0.resolve_conflict(conflict)
    }

    fn confirm_recipient_change(
        &self,
        warning: &RecipientChangeWarning,
    ) -> Result<bool, PromptError> {
        self.0.confirm_recipient_change(warning)
    }

    fn warn(&self, msg: &str) {
        self.0.warn(msg);
    }
}

/// The prompter a script run unlocks through, from where its commands come
/// from and what it can ask. In Go's fixed order:
///
/// 1. `GAGE_PASSPHRASE`, if set (`passphrase_env`; set-but-empty counts).
/// 2. Otherwise a prompt, but only when a human can be asked.
/// 3. Otherwise a refusal that names the variable.
///
/// `can_prompt` is whether there is a terminal to prompt on *and* the
/// command lines aren't arriving on it: `--script FILE` leaves stdin free
/// for a human to answer on, `--stdin` has spent it. It is one question for
/// an unlock and for a new identity's passphrase, so it is answered once by
/// the caller and applied to both.
pub fn for_script<'p>(
    base: &'p dyn Prompter,
    passphrase_env: Option<&Zeroizing<String>>,
    can_prompt: bool,
) -> Box<dyn Prompter + 'p> {
    // Neither wrapper answers a create-purpose request itself: it goes to
    // a human when there is one, and is refused outright when there isn't,
    // rather than to a prompter whose input is the command stream.
    let create = || -> Box<dyn Prompter + 'p> {
        if can_prompt {
            Box::new(Passthrough(base))
        } else {
            Box::new(NoCreatePrompter(base))
        }
    };
    match passphrase_env {
        Some(passphrase) => Box::new(EnvPrompter::new(base, create(), passphrase.clone())),
        None if can_prompt => Box::new(Passthrough(base)),
        None => Box::new(RefusingPrompter::new(base, create())),
    }
}

#[cfg(test)]
mod tests {
    use gage::entry::{Candidate, Entry};
    use gage::exitcode::{Code, HasCode};
    use gage::prompter::FakePrompter;
    use gage::syncresolve::ConflictSide;

    use super::*;
    use crate::assumeyes::AssumeYes;

    const PASSPHRASE: &str = "correct horse battery staple";

    fn request(purpose: Purpose, attempt: u32) -> UnlockRequest {
        UnlockRequest {
            kind: Kind::Passphrase,
            purpose,
            vault: "personal".into(),
            device: "laptop-1".into(),
            attempt,
        }
    }

    fn passphrase_of(resp: UnlockResponse) -> String {
        let UnlockResponse::Passphrase(p) = resp else {
            panic!("not a passphrase answer");
        };
        p.to_string()
    }

    fn env(inner: &FakePrompter) -> EnvPrompter<'_> {
        EnvPrompter::new(
            inner,
            Box::new(Passthrough(inner)),
            Zeroizing::new("from-env".into()),
        )
    }

    // Go: TestEnvPrompterAnswersOnceThenRefuses. The retry policy at the
    // unit it lives in, since a CLI run can't tell "asked once" from "asked
    // three times and gave up".
    #[test]
    fn env_prompter_answers_once_then_refuses() {
        let inner = FakePrompter::new();
        let p = env(&inner);

        let first = p.unlock(&request(Purpose::Unlock, 1)).unwrap();
        assert_eq!(passphrase_of(first), "from-env");

        let err = p.unlock(&request(Purpose::Unlock, 2)).unwrap_err();
        assert_eq!(err.code(), Code::LockedOrAuth);
        assert!(err.to_string().contains(ENV_VAR), "{err}");
        assert!(inner.unlock_requests().is_empty(), "the inner was asked");
    }

    // Go: TestGagePassphraseIsIgnoredForANewIdentity, at the unit. A
    // create-purpose answer can't be checked against anything, so a typo in
    // the variable would be unrecoverable.
    #[test]
    fn env_prompter_never_answers_a_new_identity() {
        let inner = FakePrompter::new().with_passphrase(PASSPHRASE);
        let p = env(&inner);

        let got = p.unlock(&request(Purpose::Create, 1)).unwrap();

        assert_eq!(passphrase_of(got), PASSPHRASE, "the variable answered");
        assert_eq!(inner.unlock_requests(), vec![request(Purpose::Create, 1)]);
    }

    // The same, for a variable that is set but empty: still "set", still
    // not an answer for a new identity.
    #[test]
    fn env_prompter_answers_an_unlock_with_an_empty_variable_too() {
        let inner = FakePrompter::new();
        let p = EnvPrompter::new(&inner, Box::new(Passthrough(&inner)), Zeroizing::default());
        let got = p.unlock(&request(Purpose::Unlock, 1)).unwrap();
        assert_eq!(passphrase_of(got), "");
    }

    #[test]
    fn refusing_prompter_names_the_variable_and_exits_5() {
        let inner = FakePrompter::new();
        let p = RefusingPrompter::new(&inner, Box::new(Passthrough(&inner)));
        for attempt in [1, 2] {
            let err = p.unlock(&request(Purpose::Unlock, attempt)).unwrap_err();
            assert_eq!(err.code(), Code::LockedOrAuth);
            assert!(err.to_string().contains(ENV_VAR), "{err}");
            assert!(err.to_string().contains("personal"), "{err}");
        }
        assert!(inner.unlock_requests().is_empty(), "the inner was asked");
    }

    #[test]
    fn refusing_prompter_passes_a_new_identity_to_where_create_goes() {
        let inner = FakePrompter::new().with_passphrase(PASSPHRASE);
        let p = RefusingPrompter::new(&inner, Box::new(Passthrough(&inner)));
        let got = p.unlock(&request(Purpose::Create, 1)).unwrap();
        assert_eq!(passphrase_of(got), PASSPHRASE);
    }

    // Go: TestNoCreatePrompterPassesOrdinaryUnlocksThrough. It refuses only
    // `Create`: an ordinary unlock has a passphrase that can be checked, so
    // it goes to the prompter underneath.
    #[test]
    fn no_create_prompter_passes_ordinary_unlocks_through() {
        let inner = FakePrompter::new().with_passphrase(PASSPHRASE);
        let p = NoCreatePrompter(&inner);

        let got = p.unlock(&request(Purpose::Unlock, 1)).unwrap();
        assert_eq!(passphrase_of(got), PASSPHRASE);

        // And the one it does refuse, for contrast: exit 5, saying why.
        let err = p.unlock(&request(Purpose::Create, 1)).unwrap_err();
        assert_eq!(err.code(), Code::LockedOrAuth);
        assert!(err.to_string().contains("human"), "{err}");
        assert!(err.to_string().contains(ENV_VAR), "{err}");
        assert_eq!(
            inner.unlock_requests().len(),
            1,
            "the refused create reached the inner prompter"
        );
    }

    fn conflict() -> EntryConflict {
        let side = ConflictSide {
            entry: Some(Entry {
                title: "ProtonMail".into(),
                ..Entry::default()
            }),
        };
        EntryConflict {
            id: Uuid::nil(),
            path: "entries/x.age".into(),
            local: side.clone(),
            remote: side,
        }
    }

    fn candidates() -> CandidateList {
        CandidateList {
            query: "mail".into(),
            candidates: vec![Candidate {
                id: Uuid::from_u128(1),
                title: "Personal mail".into(),
            }],
        }
    }

    /// A fake scripted with one answer for everything a wrapper doesn't
    /// answer.
    fn scripted() -> FakePrompter {
        FakePrompter::new()
            .with_confirm(true)
            .with_confirm_default_yes(true)
            .with_value("v")
            .with_choice(0)
            .with_resolutions(&[Resolution::KeepRemote])
            .with_recipient_answers(&[false])
    }

    /// Calls each method a wrapper doesn't answer and checks the answer is
    /// the inner prompter's, and that the inner was the one asked.
    fn assert_passes_the_rest_through(p: &dyn Prompter, inner: &FakePrompter) {
        assert!(p.confirm("remove it?").unwrap());
        assert!(p.confirm_default_yes("enroll?").unwrap());
        assert_eq!(&*p.value("Router").unwrap(), "v");
        assert_eq!(p.choose(&candidates()).unwrap(), Uuid::from_u128(1));
        assert_eq!(
            p.resolve_conflict(&conflict()).unwrap(),
            Resolution::KeepRemote
        );
        // The inner said no, so a wrapper that answered would show.
        assert!(
            !p.confirm_recipient_change(&RecipientChangeWarning::default())
                .unwrap()
        );
        p.warn("gage: careful");

        assert_eq!(inner.confirm_prompts(), vec!["remove it?".to_string()]);
        assert_eq!(
            inner.confirm_default_yes_prompts(),
            vec!["enroll?".to_string()]
        );
        assert_eq!(inner.value_prompts(), vec!["Router".to_string()]);
        assert_eq!(inner.choose_lists().len(), 1);
        assert_eq!(inner.conflicts_presented().len(), 1);
        assert_eq!(inner.recipient_changes().len(), 1);
        assert_eq!(inner.warnings(), vec!["gage: careful".to_string()]);
    }

    // Replaces Go's TestTerminalPrompterOfWalksDecoratorChains (Q-SCRIPT-
    // PASSPHRASE): nothing here unwraps a prompter, so what needs proving
    // is that no wrapper answers, or loses, a question it isn't for.
    #[test]
    fn every_wrapper_passes_through_each_method_it_does_not_answer() {
        let inner = scripted();
        assert_passes_the_rest_through(&Passthrough(&inner), &inner);
        let inner = scripted();
        assert_passes_the_rest_through(&env(&inner), &inner);
        let inner = scripted();
        assert_passes_the_rest_through(
            &RefusingPrompter::new(&inner, Box::new(Passthrough(&inner))),
            &inner,
        );
        let inner = scripted();
        assert_passes_the_rest_through(&NoCreatePrompter(&inner), &inner);
    }

    // Go: TestNoNonInteractivePrompterAnswersConfirmDefaultYes. Clone's
    // offer is a decision for a human: no wrapper, and not `--yes`, says
    // yes for one, so over a base that declines every one returns no.
    #[test]
    fn no_non_interactive_prompter_answers_confirm_default_yes() {
        fn check(p: &dyn Prompter, inner: &FakePrompter, who: &str) {
            assert!(!p.confirm_default_yes("enroll?").unwrap(), "{who} said yes");
            assert_eq!(inner.confirm_default_yes_prompts().len(), 1, "{who}");
        }
        let declining = || FakePrompter::new().with_confirm_default_yes(false);

        let inner = declining();
        check(&Passthrough(&inner), &inner, "Passthrough");
        let inner = declining();
        check(&env(&inner), &inner, "EnvPrompter");
        let inner = declining();
        check(
            &RefusingPrompter::new(&inner, Box::new(Passthrough(&inner))),
            &inner,
            "RefusingPrompter",
        );
        let inner = declining();
        check(&NoCreatePrompter(&inner), &inner, "NoCreatePrompter");
        let inner = declining();
        check(&NoChoosePrompter(&inner), &inner, "NoChoosePrompter");
        let inner = declining();
        check(&AssumeYes(&inner), &inner, "AssumeYes");
    }

    // Beyond Go's rows (see M12d's decisions): with nobody to pick, an
    // ambiguous query is declined without a read, and nothing else changes.
    #[test]
    fn no_choose_prompter_declines_a_pick_and_passes_everything_else() {
        let inner = FakePrompter::new().with_passphrase(PASSPHRASE);
        let p = NoChoosePrompter(&inner);

        let err = p.choose(&candidates()).unwrap_err();
        assert!(matches!(err, PromptError::Cancelled), "{err}");
        assert!(inner.choose_lists().is_empty(), "the inner was asked");

        let got = p.unlock(&request(Purpose::Unlock, 1)).unwrap();
        assert_eq!(passphrase_of(got), PASSPHRASE);

        // The rest, as the other wrappers: scripted for one of each, minus
        // `choose`, which the line above showed it keeps.
        let inner = scripted();
        let p = NoChoosePrompter(&inner);
        assert!(p.confirm("remove it?").unwrap());
        assert!(p.confirm_default_yes("enroll?").unwrap());
        assert_eq!(&*p.value("Router").unwrap(), "v");
        assert_eq!(
            p.resolve_conflict(&conflict()).unwrap(),
            Resolution::KeepRemote
        );
        assert!(
            !p.confirm_recipient_change(&RecipientChangeWarning::default())
                .unwrap()
        );
        p.warn("gage: careful");
        assert_eq!(inner.confirm_prompts().len(), 1);
        assert_eq!(inner.value_prompts().len(), 1);
        assert_eq!(inner.conflicts_presented().len(), 1);
        assert_eq!(inner.recipient_changes().len(), 1);
        assert_eq!(inner.warnings().len(), 1);
    }

    // Go: TestAssumeYesAnswersTheTrustQuestionAndNothingElse, over the new
    // wrappers too. `--yes` answers the recipient-change question and never
    // `confirm` (locking a device out) or a conflict, whichever side of the
    // script layer it sits.
    #[test]
    fn assume_yes_answers_the_trust_question_and_nothing_else_over_the_wrappers() {
        fn check(p: &dyn Prompter, inner: &FakePrompter) {
            assert!(
                p.confirm_recipient_change(&RecipientChangeWarning::default())
                    .unwrap(),
                "--yes did not answer the trust question"
            );
            assert!(
                inner.recipient_changes().is_empty(),
                "--yes reached the prompter underneath"
            );
            assert!(!p.confirm("remove this device's own key?").unwrap());
            assert_eq!(
                p.resolve_conflict(&conflict()).unwrap(),
                Resolution::KeepLocal
            );
            assert_eq!(inner.confirm_prompts().len(), 1, "--yes answered confirm");
            assert_eq!(inner.conflicts_presented().len(), 1, "--yes answered");
        }
        let fake = || {
            FakePrompter::new()
                .with_confirm(false)
                .with_resolutions(&[Resolution::KeepLocal])
        };

        // --yes innermost, the script layer over it: how a run builds it.
        let inner = fake();
        let yes = AssumeYes(&inner);
        check(&NoCreatePrompter(&yes), &inner);
        let inner = fake();
        let yes = AssumeYes(&inner);
        check(
            &EnvPrompter::new(&yes, Box::new(Passthrough(&yes)), Zeroizing::default()),
            &inner,
        );
        let inner = fake();
        let yes = AssumeYes(&inner);
        check(
            &RefusingPrompter::new(&yes, Box::new(Passthrough(&yes))),
            &inner,
        );

        // And the other way round.
        let inner = fake();
        let wrapped = NoCreatePrompter(&inner);
        check(&AssumeYes(&wrapped), &inner);
    }

    // Go: TestAssumeYesOverANonConflictPrompterStaysNonConflict. Wrapping
    // must not make a frontend that can't resolve conflicts look like one
    // that can: here, `--yes` over a prompter whose `resolve_conflict`
    // refuses still refuses, through every wrapper.
    #[test]
    fn assume_yes_over_a_prompter_that_refuses_conflicts_still_refuses() {
        let inner = FakePrompter::new();
        let yes = AssumeYes(&inner);
        let layers: Vec<Box<dyn Prompter + '_>> = vec![
            Box::new(Passthrough(&yes)),
            Box::new(NoCreatePrompter(&yes)),
            Box::new(RefusingPrompter::new(&yes, Box::new(Passthrough(&yes)))),
            Box::new(EnvPrompter::new(
                &yes,
                Box::new(Passthrough(&yes)),
                Zeroizing::default(),
            )),
        ];
        for p in &layers {
            let err = p.resolve_conflict(&conflict()).unwrap_err();
            assert!(matches!(err, PromptError::NotInteractive), "{err}");
        }
    }

    // for_script's three branches, by what each does to an unlock and to a
    // new identity (the source order, Go's scriptPrompter).
    #[test]
    fn for_script_picks_the_source_in_go_s_order() {
        let var = Some(Zeroizing::new("from-env".to_string()));

        // 1. The variable, with or without a terminal.
        for can_prompt in [false, true] {
            let inner = FakePrompter::new();
            let p = for_script(&inner, var.as_ref(), can_prompt);
            let got = p.unlock(&request(Purpose::Unlock, 1)).unwrap();
            assert_eq!(passphrase_of(got), "from-env");
        }

        // 2. No variable, a human: the ordinary prompter, every attempt.
        let inner = FakePrompter::new().with_passphrases(&["a", "b"]);
        let p = for_script(&inner, None, true);
        assert_eq!(
            passphrase_of(p.unlock(&request(Purpose::Unlock, 1)).unwrap()),
            "a"
        );
        assert_eq!(
            passphrase_of(p.unlock(&request(Purpose::Unlock, 2)).unwrap()),
            "b"
        );

        // 3. No variable, no human: a refusal, and the inner never asked.
        let inner = FakePrompter::new();
        let p = for_script(&inner, None, false);
        let err = p.unlock(&request(Purpose::Unlock, 1)).unwrap_err();
        assert_eq!(err.code(), Code::LockedOrAuth);
        assert!(err.to_string().contains(ENV_VAR), "{err}");
        assert!(inner.unlock_requests().is_empty());
    }

    #[test]
    fn for_script_sends_a_new_identity_to_a_human_or_refuses_it() {
        let var = Some(Zeroizing::new("from-env".to_string()));
        for passphrase_env in [var.as_ref(), None] {
            // A human: the prompter underneath answers, not the variable.
            let inner = FakePrompter::new().with_passphrase(PASSPHRASE);
            let p = for_script(&inner, passphrase_env, true);
            let got = p.unlock(&request(Purpose::Create, 1)).unwrap();
            assert_eq!(passphrase_of(got), PASSPHRASE);

            // Nobody to ask: refused, naming why, without asking.
            let inner = FakePrompter::new();
            let p = for_script(&inner, passphrase_env, false);
            let err = p.unlock(&request(Purpose::Create, 1)).unwrap_err();
            assert_eq!(err.code(), Code::LockedOrAuth);
            assert!(err.to_string().contains("human"), "{err}");
            assert!(inner.unlock_requests().is_empty(), "the inner was asked");
        }
    }

    #[test]
    fn a_refusal_never_prints_the_passphrase() {
        let inner = FakePrompter::new();
        let p = env(&inner);
        let err = p.unlock(&request(Purpose::Unlock, 2)).unwrap_err();
        assert!(!err.to_string().contains("from-env"), "{err}");
    }
}
