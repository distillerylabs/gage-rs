//! `--yes`: not a library concept at all, but this frontend deciding that in
//! a scripted run there is nobody to show a recipient diff to and the
//! answer is already known (Q-RECIPIENT-PROMPT).

use gage::entry::CandidateList;
use gage::prompter::{PromptError, Prompter, UnlockRequest, UnlockResponse};
use gage::syncresolve::{EntryConflict, Resolution};
use gage::trustcache::RecipientChangeWarning;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::CliError;
use gage::exitcode::Code;

/// The flag's id, as clap registers it on the root.
pub const FLAG: &str = "yes";

/// Answers the recipient-change question yes, without rendering it, and
/// passes every other call to the prompter it wraps.
///
/// Only that question. `confirm` is passed through, so M9b's "remove this
/// device's own key?" (a question about locking a device out, not about
/// reviewing a list) is still a real question in a script, and still
/// defaults to no. So is `resolve_conflict`: `sync --yes` answers no
/// conflict on anyone's behalf.
///
/// What it can't do is change what yes means. The two-outcome rule (a
/// routine change regenerates the cache, a mismatched one only records
/// that it was seen) is decided in the library from `verify`, so "--yes
/// never launders a mismatch" follows from where that decision lives, not
/// from a special case here.
pub struct AssumeYes<'p>(pub &'p dyn Prompter);

impl Prompter for AssumeYes<'_> {
    fn unlock(&self, req: &UnlockRequest) -> Result<UnlockResponse, PromptError> {
        self.0.unlock(req)
    }

    fn confirm(&self, prompt: &str) -> Result<bool, PromptError> {
        self.0.confirm(prompt)
    }

    fn value(&self, title: &str) -> Result<Zeroizing<String>, PromptError> {
        self.0.value(title)
    }

    fn choose(&self, list: &CandidateList) -> Result<Uuid, PromptError> {
        self.0.choose(list)
    }

    fn resolve_conflict(&self, conflict: &EntryConflict) -> Result<Resolution, PromptError> {
        self.0.resolve_conflict(conflict)
    }

    /// The whole of `--yes`.
    fn confirm_recipient_change(&self, _: &RecipientChangeWarning) -> Result<bool, PromptError> {
        Ok(true)
    }

    fn warn(&self, msg: &str) {
        self.0.warn(msg);
    }
}

/// Whether `--yes` was given anywhere on the command line. It is a global
/// flag, so a leaf's matches carry it wherever it was typed.
pub fn given(matches: &clap::ArgMatches) -> bool {
    matches!(matches.try_get_one::<bool>(FLAG), Ok(Some(true)))
}

/// The refusal in a session, typed on a line or on the way in.
///
/// A session is a human at a prompt, the case a recipient diff exists to
/// be shown to, and there the flag would either do nothing (a line's
/// writes ask through the prompter the session holds, not through the
/// flag) or far too much (wrapping that prompter for the session's whole
/// life would approve every change silently). Both are worse than saying
/// so. Go lets it through in a `--script`/`--stdin` session; gage-rs has no
/// script mode yet, so every session is refused.
pub fn refused_in_session() -> CliError {
    CliError::new(
        Code::Usage,
        "gage: --yes is for scripted, one-shot runs and isn't available in a session; \
         answer the prompt, or run the command from your shell with --yes",
    )
}
