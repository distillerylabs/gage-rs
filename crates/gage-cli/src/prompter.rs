//! The terminal [`Prompter`]: the one place gage reads a passphrase from a
//! human. The library never does; it asks, through this.
//!
//! Prompts, warnings and confirmations go to the error stream, so `gage
//! show foo > secret.txt` gets only the secret while a human still sees
//! what they're answering. When stdin is a terminal, secrets are read from
//! it with echo off (`rpassword`, which opens the controlling terminal);
//! otherwise a line is read from the injected input, buffered across calls,
//! so a `--stdin` script and the in-process tests drive the same code.

use std::cell::RefCell;
use std::io::{Read, Write};

use gage::entry::CandidateList;
use gage::prompter::{Kind, PromptError, Prompter, Purpose, UnlockRequest, UnlockResponse};
use gage::syncresolve::{ConflictSide, EntryConflict, Resolution};
use gage::trustcache::{RecipientChangeWarning, difference_lines};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::commands::short_id;

/// gage's wrong-passphrase retry policy, which lives here rather than in
/// the library on purpose: the library re-asks for as long as its prompter
/// answers, so "three attempts" is this prompter refusing the fourth.
/// Three because a passphrase is typed blind: one punishes a typo, and an
/// unlimited count turns a lock screen into an unrate-limited oracle. The
/// file is on disk and an attacker with it can ignore gage entirely, so
/// the number is about ergonomics; slowing an attack is scrypt's job.
pub const MAX_PASSPHRASE_ATTEMPTS: u32 = 3;

/// How many times the candidate picker re-asks a mistyped number before
/// giving up. Same shape and reason as the passphrase policy: the library
/// hands over a list and has no opinion on how often a human may answer.
pub const MAX_CHOOSE_ATTEMPTS: u32 = 3;

/// How many times the conflict menu re-asks an answer it doesn't
/// recognize before giving up without choosing (Go's
/// `maxConflictAttempts`).
pub const MAX_CONFLICT_ATTEMPTS: u32 = 5;

/// Where masked secrets come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretInput {
    /// Lines from the injected input: stdin is not a terminal, so there is
    /// no echo to suppress.
    Lines,
    /// A terminal, read with echo off: the controlling terminal (`None`),
    /// or a given terminal device, which is how the PTY test points the
    /// real read at its own terminal.
    Terminal(Option<String>),
}

pub struct TerminalPrompter<R: Read, W: Write> {
    /// Read a byte at a time, never ahead: whatever follows the line
    /// returned belongs to whoever reads the stream next (the next prompt,
    /// the next `--stdin` command, or `insert --value-stdin` reading the
    /// value after the passphrase). `R` is stdin, which std already
    /// buffers process-wide, so this costs nothing.
    input: RefCell<R>,
    out: RefCell<W>,
    secrets: SecretInput,
    /// Whether a human is at the other end of `input`. Conflict resolution
    /// and the recipient-change question consult it: every other prompt
    /// has a sensible answer from a script (a passphrase can be piped, a
    /// confirmation defaults to no), while "which version of this secret
    /// do you keep" has none, and "do you trust this recipient list" must
    /// not spend a line of piped input that belongs to someone else.
    /// Set by `main` from whether stdin is a terminal, not probed per call,
    /// so tests drive both halves without one.
    interactive: bool,
}

impl<R: Read, W: Write> TerminalPrompter<R, W> {
    pub fn new(input: R, out: W, secrets: SecretInput) -> TerminalPrompter<R, W> {
        TerminalPrompter {
            input: RefCell::new(input),
            out: RefCell::new(out),
            secrets,
            interactive: false,
        }
    }

    /// The same prompter, with (`true`) or without a human at the other
    /// end. Without one, it refuses to resolve a conflict.
    pub fn interactive(mut self, interactive: bool) -> TerminalPrompter<R, W> {
        self.interactive = interactive;
        self
    }
}

impl<R: Read, W: Write> TerminalPrompter<R, W> {
    /// Writes to the prompt stream. A failed write is dropped: there is
    /// nowhere better to report it, and the read that follows still
    /// decides the outcome.
    fn say(&self, text: &str) {
        let mut out = self.out.borrow_mut();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }

    /// Prompts and reads one secret without echoing it.
    fn read_secret(&self, prompt: &str) -> Result<Zeroizing<String>, PromptError> {
        self.say(prompt);
        let secret = match &self.secrets {
            SecretInput::Lines => self.read_line()?,
            SecretInput::Terminal(device) => {
                let mut config = rpassword::ConfigBuilder::new().output_discard();
                if let Some(device) = device {
                    config = config.input_file_path(device.clone());
                }
                match rpassword::read_password_with_config(config.build()) {
                    Ok(secret) => Zeroizing::new(secret),
                    // Ctrl-D on an empty line, or Ctrl-C: no answer.
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::Interrupted
                        ) =>
                    {
                        self.say("\n");
                        return Err(PromptError::Cancelled);
                    }
                    Err(e) => return Err(PromptError::Io(e)),
                }
            }
        };
        // Neither a terminal with echo off nor piped input echoes the
        // Enter, so without this the next line would run on from the
        // prompt.
        self.say("\n");
        Ok(secret)
    }

    /// Prompts and reads one echoed line.
    fn ask(&self, prompt: &str) -> Result<Zeroizing<String>, PromptError> {
        self.say(prompt);
        self.read_line()
    }

    /// One line of the injected input, without its line ending. A last
    /// line with no newline is still an answer; end of input with nothing
    /// at all is not. Stops at the newline: nothing after it is consumed.
    fn read_line(&self) -> Result<Zeroizing<String>, PromptError> {
        let mut bytes = Zeroizing::new(Vec::new());
        let mut input = self.input.borrow_mut();
        let mut byte = [0u8; 1];
        loop {
            match input.read(&mut byte) {
                Ok(0) if bytes.is_empty() => return Err(PromptError::Cancelled),
                Ok(0) => break,
                Ok(_) if byte[0] == b'\n' => break,
                Ok(_) => bytes.push(byte[0]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        }
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        match std::str::from_utf8(&bytes) {
            Ok(line) => Ok(Zeroizing::new(line.to_string())),
            Err(e) => Err(PromptError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                e,
            ))),
        }
    }

    /// Asks for a brand-new identity's passphrase, twice: there is nothing
    /// to check a new passphrase against, so a typo would otherwise surface
    /// at the next unlock, when the key it protects is already lost.
    fn new_passphrase(&self, req: &UnlockRequest) -> Result<UnlockResponse, PromptError> {
        self.say(&format!(
            "gage: creating identity {:?} for vault {:?}.\n\
             gage: this passphrase protects this device's private key. It cannot be recovered or reset.\n\
             gage: it stays on this device and is never sent to anyone.\n",
            req.device, req.vault
        ));
        for _ in 0..MAX_PASSPHRASE_ATTEMPTS {
            let first = self.read_secret("Choose a passphrase: ")?;
            if first.is_empty() {
                self.say("gage: an empty passphrase is not accepted.\n");
                continue;
            }
            let second = self.read_secret("Confirm passphrase: ")?;
            if *first == *second {
                return Ok(UnlockResponse::Passphrase(first));
            }
            self.say("gage: the two passphrases did not match.\n");
        }
        Err(PromptError::NoMatchingPassphrase(MAX_PASSPHRASE_ATTEMPTS))
    }
}

impl<R: Read, W: Write> Prompter for TerminalPrompter<R, W> {
    fn unlock(&self, req: &UnlockRequest) -> Result<UnlockResponse, PromptError> {
        match req.kind {
            Kind::Passphrase => {}
            // Kind is non_exhaustive: a method this build can't render is
            // refused, never answered with whatever a passphrase prompt
            // would read (Go's TestPrompterRefusesAMethodItCannotSatisfy;
            // untestable until a second Kind exists).
            other => return Err(PromptError::UnsupportedKind(other.to_string())),
        }
        if req.purpose == Purpose::Create {
            return self.new_passphrase(req);
        }
        if req.attempt > MAX_PASSPHRASE_ATTEMPTS {
            return Err(PromptError::TooManyAttempts);
        }
        if req.attempt > 1 {
            self.say(&format!(
                "gage: wrong passphrase (attempt {} of {MAX_PASSPHRASE_ATTEMPTS})\n",
                req.attempt
            ));
        }
        let passphrase = self.read_secret(&format!(
            "Enter passphrase for vault {:?} (device {}): ",
            req.vault, req.device
        ))?;
        Ok(UnlockResponse::Passphrase(passphrase))
    }

    /// A bare Enter means no: every caller is about to do something a user
    /// might not want, so the safe answer is the default.
    fn confirm(&self, prompt: &str) -> Result<bool, PromptError> {
        let answer = self.ask(&format!("{prompt} [y/N] "))?;
        Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
    }

    /// A bare Enter means yes: the one question whose default is to go
    /// ahead, rendered `[Y/n]` so the default is visible.
    fn confirm_default_yes(&self, prompt: &str) -> Result<bool, PromptError> {
        let answer = self.ask(&format!("{prompt} [Y/n] "))?;
        Ok(!matches!(answer.trim().to_lowercase().as_str(), "n" | "no"))
    }

    /// Masked, like a passphrase: the value is a secret, and it is typed
    /// where a screen or a recording could see it.
    fn value(&self, title: &str) -> Result<Zeroizing<String>, PromptError> {
        self.read_secret(&format!("Value for {title:?}: "))
    }

    /// The design doc's numbered picker. One-shot mode prints the same
    /// list and fails; this is a session asking instead.
    fn choose(&self, list: &CandidateList) -> Result<Uuid, PromptError> {
        if list.candidates.is_empty() {
            return Err(PromptError::EmptyCandidates);
        }
        let mut text = format!("Multiple entries match {:?}:\n", list.query);
        for (i, c) in list.candidates.iter().enumerate() {
            text.push_str(&format!(
                "  {}. {} ({})\n",
                i + 1,
                c.title,
                short_id(&c.id.to_string())
            ));
        }
        self.say(&text);
        let n = list.candidates.len();
        for _ in 0..MAX_CHOOSE_ATTEMPTS {
            let answer = self.ask(&format!("Which one? [1-{n}]: "))?;
            match answer.trim().parse::<usize>() {
                Ok(pick @ 1..) if pick <= n => return Ok(list.candidates[pick - 1].id),
                _ => self.say(&format!("gage: enter a number between 1 and {n}.\n")),
            }
        }
        Err(PromptError::NoChoice(MAX_CHOOSE_ATTEMPTS))
    }

    fn warn(&self, msg: &str) {
        self.say(&format!("{msg}\n"));
    }

    /// Refused without a human at the other end, before drawing or reading
    /// anything (Q-RECIPIENT-PROMPT): Go reads a line regardless, which on
    /// piped input spends a value or a command as the answer. The outcome
    /// is Go's default-no; a script that means yes says `--yes`.
    fn confirm_recipient_change(&self, w: &RecipientChangeWarning) -> Result<bool, PromptError> {
        if !self.interactive {
            return Err(PromptError::NotInteractive);
        }
        self.render_recipient_change(w)
    }

    /// The design doc's `[l/r/b/s/q]` menu ("Resolving an entry
    /// conflict"). Both versions arrive decrypted, but neither value is
    /// printed: the choice is made on provenance, who wrote each and when,
    /// and printing two passwords on a shared screen to ask which to keep
    /// would be its own disclosure.
    ///
    /// Without a human at the other end it refuses rather than choosing,
    /// before drawing anything or reading a line.
    fn resolve_conflict(&self, c: &EntryConflict) -> Result<Resolution, PromptError> {
        if !self.interactive {
            return Err(PromptError::NotInteractive);
        }
        self.say(&format!(
            "Entry conflicts: {:?}\n\n  local    {}\n  remote   {}\n\n\
             \x20 [l] keep local     [r] keep remote\n\
             \x20 [b] keep both      [s] skip this entry     [q] abort sync\n",
            c.title(),
            provenance(&c.local),
            provenance(&c.remote),
        ));
        for _ in 0..MAX_CONFLICT_ATTEMPTS {
            let answer = self.ask("Which? [l/r/b/s/q]: ")?;
            match answer.trim().to_lowercase().as_str() {
                "l" => return Ok(Resolution::KeepLocal),
                "r" => return Ok(Resolution::KeepRemote),
                "b" => return Ok(Resolution::KeepBoth),
                "s" => return Ok(Resolution::Skip),
                "q" => return Ok(Resolution::Abort),
                // No default: an unrecognized keystroke is how a secret
                // gets discarded by someone who meant the key next to it.
                _ => self.say("gage: answer l, r, b, s or q.\n"),
            }
        }
        Err(PromptError::NoResolution(MAX_CONFLICT_ATTEMPTS))
    }
}

impl<R: Read, W: Write> TerminalPrompter<R, W> {
    /// The design doc's recipient-change prompt ("Local trust cache"): the
    /// diff, then whether the two recipient files agree, then the count
    /// and `[y/N]`. A routine change and a mismatched one read differently
    /// on purpose: the mismatch is the tampering signature the cache
    /// exists to surface, and must not read like "someone added a
    /// device". A bare Enter is no.
    fn render_recipient_change(&self, w: &RecipientChangeWarning) -> Result<bool, PromptError> {
        let mut text = format!(
            "\u{26a0} Recipients for {:?} changed since you last encrypted here:\n\n",
            w.vault
        );
        if !w.diff.is_empty() {
            text.push_str(&w.diff);
            text.push('\n');
        }
        if w.mismatched() {
            text.push_str(
                "gage: .age-recipients and .gage/config.toml DISAGREE (gage recipient verify fails):\n",
            );
            text.push_str(&difference_lines(&w.verification));
            // Said before the question, because it changes what yes means:
            // this write goes through, and nothing else does.
            text.push_str(
                "\ngage: confirming proceeds with this write; it does not clear the mismatch.\n",
            );
        } else {
            text.push_str(
                "gage: .age-recipients matches .gage/config.toml. (gage recipient verify passes)\n",
            );
        }
        self.say(&text);
        self.confirm(&format!(
            "{}. Proceed and trust this recipient list?",
            recipient_change_count(w)
        ))
    }
}

/// The design doc's "1 recipient added." line.
pub(crate) fn recipient_change_count(w: &RecipientChangeWarning) -> String {
    let noun = |n: usize| if n == 1 { "recipient" } else { "recipients" };
    match (w.added.len(), w.removed.len()) {
        // config.toml shows no recipient change: the .age-recipients-only
        // edit, with nothing to count and everything to worry about.
        (0, 0) => "The recipient list changed".to_string(),
        (a, 0) => format!("{a} {} added", noun(a)),
        (0, r) => format!("{r} {} removed", noun(r)),
        (a, r) => format!("{a} recipient(s) added, {r} removed"),
    }
}

/// One side's line of the conflict menu: when and by which device it was
/// written, or that it deleted the entry.
fn provenance(side: &ConflictSide) -> String {
    let Some(e) = &side.entry else {
        return "deleted this entry".to_string();
    };
    // UTC, to the minute: two edits to one entry are routinely made on the
    // same day, so the day alone would distinguish nothing.
    let when = e.updated.map_or_else(
        || "-".to_string(),
        |t| t.as_jiff().strftime("%Y-%m-%d %H:%M").to_string(),
    );
    let by = if e.updated_by.is_empty() {
        "-"
    } else {
        &e.updated_by
    };
    format!("updated {when}  by {by}")
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use gage::entry::{Candidate, Entry};
    use gage::exitcode::{Code, HasCode};
    use gage::recipient::RecipientVerification;
    use gage::syncresolve::{ConflictSide, EntryConflict, Resolution};

    use super::*;

    /// An output buffer a test can read while the prompter still owns it.
    #[derive(Clone, Default)]
    pub(crate) struct SharedBuf(Rc<RefCell<Vec<u8>>>);

    impl SharedBuf {
        pub(crate) fn text(&self) -> String {
            String::from_utf8(self.0.borrow().clone()).unwrap()
        }

        fn clear(&self) {
            self.0.borrow_mut().clear();
        }
    }

    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn prompter(
        input: &str,
    ) -> (
        TerminalPrompter<std::io::Cursor<Vec<u8>>, SharedBuf>,
        SharedBuf,
    ) {
        let out = SharedBuf::default();
        let p = TerminalPrompter::new(
            std::io::Cursor::new(input.as_bytes().to_vec()),
            out.clone(),
            SecretInput::Lines,
        );
        (p, out)
    }

    fn request(purpose: Purpose, attempt: u32) -> UnlockRequest {
        UnlockRequest {
            kind: Kind::Passphrase,
            purpose,
            vault: "personal".into(),
            device: "laptop-1".into(),
            attempt,
        }
    }

    fn unlock_req(attempt: u32) -> UnlockRequest {
        request(Purpose::Unlock, attempt)
    }

    fn passphrase(resp: UnlockResponse) -> String {
        let UnlockResponse::Passphrase(p) = resp else {
            panic!("not a passphrase response")
        };
        p.to_string()
    }

    #[test]
    fn value_is_read_as_a_line_and_prompts_with_the_title() {
        let (p, out) = prompter("hunter2\nnext\n");
        assert_eq!(&*p.value("ProtonMail").unwrap(), "hunter2");
        assert!(
            out.text().contains("Value for \"ProtonMail\": "),
            "{}",
            out.text()
        );
        assert!(!out.text().contains("hunter2"), "the value was echoed");
        // Buffered like every other answer: the line after it survives.
        assert_eq!(&*p.value("x").unwrap(), "next");
        let err = p.value("x").unwrap_err();
        assert!(matches!(err, PromptError::Cancelled), "{err}");
    }

    // `printf 'pass\nvalue' | gage insert x --value-stdin`: the prompter
    // reads the passphrase line and the command then reads the rest of the
    // same stream. A prompter that read ahead (a buffered reader) would
    // swallow the value, and `insert` would store an empty secret without
    // complaint, so it must stop at the end of the line it was asked for.
    #[test]
    fn a_line_read_never_consumes_input_beyond_its_newline() {
        let mut stream = std::io::Cursor::new(b"pass\nthe value\nmore".to_vec());
        {
            let p = TerminalPrompter::new(&mut stream, SharedBuf::default(), SecretInput::Lines);
            assert_eq!(passphrase(p.unlock(&unlock_req(1)).unwrap()), "pass");
        }
        let mut rest = String::new();
        std::io::Read::read_to_string(&mut stream, &mut rest).unwrap();
        assert_eq!(rest, "the value\nmore");
    }

    #[test]
    fn a_line_that_is_not_utf8_is_an_io_error() {
        let p = TerminalPrompter::new(
            std::io::Cursor::new(vec![0xff, 0xfe, b'\n']),
            SharedBuf::default(),
            SecretInput::Lines,
        );
        let err = p.unlock(&unlock_req(1)).unwrap_err();
        assert!(matches!(err, PromptError::Io(_)), "{err}");
    }

    // Go: TestReadingBufferedInputDoesNotSwallowFollowingLines
    #[test]
    fn reading_buffered_input_does_not_swallow_following_lines() {
        let (p, _) = prompter("first\nsecond\n");
        assert_eq!(passphrase(p.unlock(&unlock_req(1)).unwrap()), "first");
        assert_eq!(passphrase(p.unlock(&unlock_req(2)).unwrap()), "second");
    }

    // A final line without a newline is still an answer, and CRLF input
    // (a Windows pipe) loses its CR.
    #[test]
    fn a_last_line_without_a_newline_and_crlf_are_answers() {
        let (p, _) = prompter("first\r\nlast");
        assert_eq!(passphrase(p.unlock(&unlock_req(1)).unwrap()), "first");
        assert_eq!(passphrase(p.unlock(&unlock_req(2)).unwrap()), "last");
    }

    // Go: TestReadingPastEndOfInputFails
    #[test]
    fn reading_past_end_of_input_fails() {
        let (p, _) = prompter("");
        let err = p.unlock(&unlock_req(1)).unwrap_err();
        assert!(matches!(err, PromptError::Cancelled), "{err}");
        assert_eq!(err.code(), Code::LockedOrAuth);
    }

    // Go: TestRetryPolicyLivesInThePrompter: the library re-asks while it
    // is answered, so the policy is this prompter refusing attempt 4.
    #[test]
    fn retry_policy_lives_in_the_prompter() {
        let (p, _) = prompter("a\nb\nc\nd\n");
        for attempt in 1..=MAX_PASSPHRASE_ATTEMPTS {
            assert!(!passphrase(p.unlock(&unlock_req(attempt)).unwrap()).is_empty());
        }
        let err = p
            .unlock(&unlock_req(MAX_PASSPHRASE_ATTEMPTS + 1))
            .unwrap_err();
        assert!(matches!(err, PromptError::TooManyAttempts), "{err}");
        assert_eq!(err.code(), Code::LockedOrAuth);
        assert_eq!(MAX_PASSPHRASE_ATTEMPTS, 3);
    }

    // Go: TestRetryPromptTellsTheUserWhichAttemptThisIs
    #[test]
    fn retry_prompt_tells_the_user_which_attempt_this_is() {
        let (p, out) = prompter("first\nsecond\n");
        p.unlock(&unlock_req(1)).unwrap();
        assert!(!out.text().contains("wrong passphrase"), "{}", out.text());
        assert!(
            out.text()
                .contains("Enter passphrase for vault \"personal\" (device laptop-1)"),
            "{}",
            out.text()
        );

        out.clear();
        p.unlock(&unlock_req(2)).unwrap();
        assert!(out.text().contains("wrong passphrase"), "{}", out.text());
        assert!(out.text().contains("attempt 2 of 3"), "{}", out.text());
    }

    // Go: TestNewPassphraseIsConfirmed
    #[test]
    fn new_passphrase_is_confirmed() {
        let create = request(Purpose::Create, 1);

        // Matching entries are accepted.
        let (p, out) = prompter("s3cret\ns3cret\n");
        assert_eq!(passphrase(p.unlock(&create).unwrap()), "s3cret");
        for want in [
            "Choose a passphrase",
            "Confirm passphrase",
            "cannot be recovered",
            "never sent",
        ] {
            assert!(
                out.text().contains(want),
                "{want:?} missing:\n{}",
                out.text()
            );
        }

        // A mismatch is re-asked, not accepted.
        let (p, out) = prompter("typo\ntypa\ns3cret\ns3cret\n");
        assert_eq!(passphrase(p.unlock(&create).unwrap()), "s3cret");
        assert!(out.text().contains("did not match"), "{}", out.text());

        // Repeated mismatches give up rather than looping.
        let (p, _) = prompter("a\nb\nc\nd\ne\nf\n");
        let err = p.unlock(&create).unwrap_err();
        assert!(matches!(err, PromptError::NoMatchingPassphrase(3)), "{err}");
        assert_eq!(err.code(), Code::LockedOrAuth);
    }

    // Go: TestNewPassphraseRejectsAnEmptyOne
    #[test]
    fn new_passphrase_rejects_an_empty_one() {
        let (p, out) = prompter("\ns3cret\ns3cret\n");
        assert_eq!(
            passphrase(p.unlock(&request(Purpose::Create, 1)).unwrap()),
            "s3cret"
        );
        assert!(
            out.text().contains("an empty passphrase is not accepted"),
            "{}",
            out.text()
        );
    }

    // Go: TestConfirmDefaultsToNo
    #[test]
    fn confirm_defaults_to_no() {
        for (input, want) in [
            ("y\n", true),
            ("Y\n", true),
            ("yes\n", true),
            (" YES \n", true),
            ("n\n", false),
            ("\n", false),
            ("garbage\n", false),
        ] {
            let (p, _) = prompter(input);
            assert_eq!(p.confirm("do the thing?").unwrap(), want, "{input:?}");
        }
    }

    // Go: TestConfirmDefaultYesDefaultsToYes
    #[test]
    fn confirm_default_yes_defaults_to_yes() {
        for (input, want) in [
            ("\n", true),
            ("y\n", true),
            ("yes\n", true),
            ("garbage\n", true),
            ("n\n", false),
            (" NO \n", false),
        ] {
            let (p, _) = prompter(input);
            assert_eq!(p.confirm_default_yes("enroll?").unwrap(), want, "{input:?}");
        }
        let (p, out) = prompter("\n");
        assert!(p.confirm_default_yes("enroll?").unwrap());
        assert!(out.text().contains("enroll? [Y/n] "), "{}", out.text());
        // The default-no question is unchanged beside it.
        let (p, _) = prompter("\n");
        assert!(!p.confirm("enroll?").unwrap());
    }

    // Go: TestConfirmStillRendersDefaultNo
    #[test]
    fn confirm_still_renders_default_no() {
        let (p, out) = prompter("\n");
        assert!(!p.confirm("do the thing?").unwrap());
        assert!(
            out.text().contains("do the thing? [y/N] "),
            "{}",
            out.text()
        );
    }

    #[test]
    fn confirm_past_end_of_input_fails() {
        let (p, _) = prompter("");
        assert!(matches!(
            p.confirm("do the thing?"),
            Err(PromptError::Cancelled)
        ));
    }

    // Go: TestWarnIsSurfacedAndReturnsNothing
    #[test]
    fn warn_is_surfaced_and_returns_nothing() {
        let (p, out) = prompter("");
        let () = p.warn("gage: could not lock key material into memory");
        assert_eq!(
            out.text(),
            "gage: could not lock key material into memory\n"
        );
    }

    fn mail_candidates() -> CandidateList {
        let id = |s: &str| uuid::Uuid::try_parse(s).unwrap();
        CandidateList {
            query: "mail".into(),
            candidates: vec![
                Candidate {
                    id: id("11111111-1111-4111-8111-111111111111"),
                    title: "Personal mail".into(),
                },
                Candidate {
                    id: id("22222222-2222-4222-8222-222222222222"),
                    title: "Work mail".into(),
                },
            ],
        }
    }

    // Go: TestChooseOnAnEmptyCandidateListIsInternal
    #[test]
    fn choose_on_an_empty_candidate_list_is_internal() {
        let (p, _) = prompter("1\n");
        let err = p
            .choose(&CandidateList {
                query: "anything".into(),
                candidates: Vec::new(),
            })
            .unwrap_err();
        assert!(matches!(err, PromptError::EmptyCandidates), "{err}");
        assert_eq!(err.code(), Code::Internal);
    }

    // Go: TestChooseRetriesThenGivesUp. A number outside the list and a
    // non-number are both re-asked with the range spelled out, and running
    // out of attempts is Ambiguous, never a pick made on the human's
    // behalf: that would hand them a different secret than they meant.
    #[test]
    fn choose_retries_then_gives_up() {
        let list = mail_candidates();

        let (p, out) = prompter("9\nnot-a-number\n2\n");
        assert_eq!(p.choose(&list).unwrap(), list.candidates[1].id);
        let text = out.text();
        assert!(text.contains("Multiple entries match \"mail\":"), "{text}");
        assert!(text.contains("  1. Personal mail (11111111)"), "{text}");
        assert!(text.contains("  2. Work mail (22222222)"), "{text}");
        assert!(text.contains("Which one? [1-2]: "), "{text}");
        assert_eq!(
            text.matches("enter a number between 1 and 2").count(),
            2,
            "once per bad answer:\n{text}"
        );

        let (p, _) = prompter(&"0\n".repeat(MAX_CHOOSE_ATTEMPTS as usize));
        let err = p.choose(&list).unwrap_err();
        assert!(
            matches!(err, PromptError::NoChoice(MAX_CHOOSE_ATTEMPTS)),
            "{err}"
        );
        assert_eq!(err.code(), Code::Ambiguous);
    }

    // End of input at the picker is the human declining to pick, which
    // the session turns back into the candidate list.
    #[test]
    fn choose_past_end_of_input_is_cancelled() {
        let (p, _) = prompter("");
        let err = p.choose(&mail_candidates()).unwrap_err();
        assert!(matches!(err, PromptError::Cancelled), "{err}");
    }

    // --- conflict resolution ------------------------------------------------

    /// A prompter with a human at the other end, reading `input`.
    fn interactive(
        input: &str,
    ) -> (
        TerminalPrompter<std::io::Cursor<Vec<u8>>, SharedBuf>,
        SharedBuf,
    ) {
        let (p, out) = prompter(input);
        (p.interactive(true), out)
    }

    /// A conflict built by hand, for the tests of the menu alone (Go's
    /// `sampleConflict`).
    fn sample_conflict() -> EntryConflict {
        let side = |updated: &str, by: &str, value: &str| ConflictSide {
            entry: Some(Entry {
                title: "ProtonMail".into(),
                updated: Some(updated.parse().unwrap()),
                updated_by: by.into(),
                value: value.into(),
                ..Entry::default()
            }),
        };
        EntryConflict {
            id: Uuid::nil(),
            path: "entries/4b9d7710-0000-0000-0000-000000000000.age".into(),
            local: side("2026-08-30T14:22:00Z", "laptop-1", "my-rotated-password"),
            remote: side("2026-08-29T09:03:00Z", "phone", "their-rotated-password"),
        }
    }

    // Go: TestConflictPromptRendersTheDesignDocsMenu. The title, both sides'
    // provenance and the five options; never either secret value.
    #[test]
    fn conflict_prompt_renders_the_design_docs_menu() {
        let (p, out) = interactive("l\n");
        p.resolve_conflict(&sample_conflict()).unwrap();
        let shown = out.text();
        for want in [
            "\"ProtonMail\"",
            "local",
            "remote",
            "laptop-1",
            "phone",
            "2026-08-30 14:22",
            "2026-08-29 09:03",
            "[l] keep local",
            "[r] keep remote",
            "[b] keep both",
            "[s] skip this entry",
            "[q] abort sync",
            "[l/r/b/s/q]",
        ] {
            assert!(shown.contains(want), "{want:?} missing from:\n{shown}");
        }
        for secret in ["my-rotated-password", "their-rotated-password"] {
            assert!(!shown.contains(secret), "printed {secret:?}:\n{shown}");
        }
    }

    // Go: TestConflictPromptMapsEveryKey
    #[test]
    fn conflict_prompt_maps_every_key() {
        for (typed, want) in [
            ("l", Resolution::KeepLocal),
            ("r", Resolution::KeepRemote),
            ("b", Resolution::KeepBoth),
            ("s", Resolution::Skip),
            ("q", Resolution::Abort),
            (" B ", Resolution::KeepBoth),
        ] {
            let (p, _) = interactive(&format!("{typed}\n"));
            assert_eq!(
                p.resolve_conflict(&sample_conflict()).unwrap(),
                want,
                "{typed:?}"
            );
        }
    }

    // Go: TestConflictPromptReAsksOnAnUnknownAnswer. No default: a guessed
    // keystroke is how a secret gets discarded.
    #[test]
    fn conflict_prompt_re_asks_on_an_unknown_answer() {
        let (p, out) = interactive("x\n\nb\n");
        assert_eq!(
            p.resolve_conflict(&sample_conflict()).unwrap(),
            Resolution::KeepBoth
        );
        assert_eq!(
            out.text().matches("[l/r/b/s/q]").count(),
            3,
            "{}",
            out.text()
        );
        assert!(
            out.text().contains("answer l, r, b, s or q"),
            "{}",
            out.text()
        );

        // And it gives up, without choosing, after its retry policy.
        let (p, _) = interactive(&"x\n".repeat(MAX_CONFLICT_ATTEMPTS as usize));
        let err = p.resolve_conflict(&sample_conflict()).unwrap_err();
        assert!(matches!(err, PromptError::NoResolution(_)), "{err}");
        assert_eq!(err.code(), Code::Conflict);
        // End of input is no answer either.
        let (p, _) = interactive("");
        let err = p.resolve_conflict(&sample_conflict()).unwrap_err();
        assert!(matches!(err, PromptError::Cancelled), "{err}");
    }

    // Go: TestDeleteModifyPromptSaysOneSideDeletedIt
    #[test]
    fn delete_modify_prompt_says_one_side_deleted_it() {
        let (p, out) = interactive("r\n");
        let mut c = sample_conflict();
        c.local = ConflictSide { entry: None };
        assert_eq!(p.resolve_conflict(&c).unwrap(), Resolution::KeepRemote);
        let shown = out.text();
        assert!(shown.contains("local    deleted this entry"), "{shown}");
        assert!(
            shown.contains("\"ProtonMail\""),
            "named by the survivor:\n{shown}"
        );
    }

    // Go: TestNonInteractivePrompterRefusesToResolve. Refused by the
    // frontend, before any menu is drawn or any input read.
    #[test]
    fn non_interactive_prompter_refuses_to_resolve() {
        let (p, out) = prompter("l\n");
        let err = p.resolve_conflict(&sample_conflict()).unwrap_err();
        assert!(matches!(err, PromptError::NotInteractive), "{err}");
        assert_eq!(err.code(), Code::Conflict);
        assert_eq!(out.text(), "", "a refusing prompter rendered a menu");
        // The line it was given is still there for whoever reads next.
        assert_eq!(&*p.value("x").unwrap(), "l");
    }

    // --- the recipient-change question --------------------------------------

    /// A recipient change as the library hands one over, with the design
    /// doc's own illustration for its diff.
    fn sample_change(verification: RecipientVerification) -> RecipientChangeWarning {
        RecipientChangeWarning {
            vault: "personal".into(),
            diff: "--- known-config.toml (last confirmed 2026-08-14)\n\
                   +++ .gage/config.toml (current)\n\
                   +[[recipients]]\n\
                   +device = \"unknown-device\""
                .into(),
            verification,
            added: vec![gage::vaultconfig::Recipient {
                device: "unknown-device".into(),
                pubkey: "age1qz8x2".into(),
            }],
            ..Default::default()
        }
    }

    fn in_sync() -> RecipientVerification {
        RecipientVerification {
            in_sync: true,
            ..Default::default()
        }
    }

    // Go: TestTerminalPrompterRendersTheRecipientChangeAsTheDesignDocPrompt.
    // The library hands over a value; this is the CLI turning it into the
    // design doc's diff and [y/N]. A mismatch gets its own, more severe
    // lines; a bare Enter is no.
    #[test]
    fn terminal_prompter_renders_the_recipient_change_as_the_design_doc_prompt() {
        let (p, out) = interactive("y\n");
        assert!(
            p.confirm_recipient_change(&sample_change(in_sync()))
                .unwrap()
        );
        let text = out.text();
        for want in [
            "Recipients for \"personal\" changed since you last encrypted here",
            "known-config.toml",
            "unknown-device",
            "recipient verify passes",
            "1 recipient added. Proceed and trust this recipient list? [y/N]",
        ] {
            assert!(text.contains(want), "missing {want:?}:\n{text}");
        }

        let (p, out) = interactive("y\n");
        let mismatched = RecipientVerification {
            only_in_recipients_file: vec!["age1stray".into()],
            ..Default::default()
        };
        p.confirm_recipient_change(&sample_change(mismatched))
            .unwrap();
        let text = out.text();
        assert!(text.contains("age1stray"), "{text}");
        assert!(text.contains(".age-recipients"), "{text}");
        assert!(!text.contains("verify passes"), "{text}");

        let (p, _) = interactive("\n");
        assert!(
            !p.confirm_recipient_change(&sample_change(in_sync()))
                .unwrap(),
            "a bare Enter approved an unreviewed list"
        );
    }

    // Go: TestConfirmRecipientChangeNamesBothDirectionsOfAMismatch (from P2).
    // A key only .age-recipients has and a key only config.toml has are
    // different problems, and the question lists both.
    #[test]
    fn confirm_recipient_change_names_both_directions_of_a_mismatch() {
        let (p, out) = interactive("n\n");
        let w = RecipientChangeWarning {
            vault: "personal".into(),
            verification: RecipientVerification {
                in_sync: false,
                only_in_recipients_file: vec!["age1onlyinrecipientsfile".into()],
                only_in_config: vec!["age1onlyinconfig".into()],
            },
            ..Default::default()
        };
        assert!(!p.confirm_recipient_change(&w).unwrap(), "`n` approved");
        let text = out.text();
        for want in [
            "age1onlyinrecipientsfile is in .age-recipients but not in .gage/config.toml",
            "age1onlyinconfig is in .gage/config.toml but not in .age-recipients",
            "DISAGREE",
            "does not clear the mismatch",
        ] {
            assert!(text.contains(want), "missing {want:?}:\n{text}");
        }
    }

    // Go: TestRecipientChangeCountSummarizesEveryShape (from P2): the design
    // doc's "1 recipient added." line, for every shape of change.
    #[test]
    fn recipient_change_count_summarizes_every_shape() {
        let r = |n: usize| {
            vec![
                gage::vaultconfig::Recipient {
                    device: "d".into(),
                    pubkey: "k".into(),
                };
                n
            ]
        };
        let shape = |added: usize, removed: usize| RecipientChangeWarning {
            added: r(added),
            removed: r(removed),
            ..Default::default()
        };
        for (w, want) in [
            (shape(1, 0), "1 recipient added"),
            (shape(2, 0), "2 recipients added"),
            (shape(0, 1), "1 recipient removed"),
            (shape(0, 2), "2 recipients removed"),
            (shape(1, 2), "1 recipient(s) added, 2 removed"),
            (shape(0, 0), "The recipient list changed"),
        ] {
            assert_eq!(recipient_change_count(&w), want);
        }
    }

    // With nobody at the terminal the question is refused before anything
    // is drawn or read (Q-RECIPIENT-PROMPT): on piped input the next line
    // belongs to someone else, a value or a session command.
    #[test]
    fn non_interactive_prompter_refuses_the_recipient_question() {
        let (p, out) = prompter("y\n");
        let err = p
            .confirm_recipient_change(&sample_change(in_sync()))
            .unwrap_err();
        assert!(matches!(err, PromptError::NotInteractive), "{err}");
        assert_eq!(out.text(), "", "a refusing prompter rendered the diff");
        assert_eq!(&*p.value("x").unwrap(), "y", "the answer line was consumed");
    }

    // Go: TestRealPrompterReadsAPassphraseWithoutEchoing. One of the few
    // tests that needs a real terminal: "not echoed" is a property of the
    // tty's line discipline, so an in-memory reader would pass whether or
    // not the echo-off read were wired up at all. Unix only, as Go's is
    // (`linux || darwin`): a pty is a POSIX object.
    #[cfg(unix)]
    #[test]
    #[allow(unsafe_code)]
    fn real_prompter_reads_a_passphrase_without_echoing() {
        use std::os::fd::AsRawFd;
        use std::sync::{Arc, Mutex, mpsc};
        use std::time::{Duration, Instant};

        use portable_pty::{PtySize, native_pty_system};

        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let tty = pair
            .master
            .tty_name()
            .expect("a unix pty has a device path");
        let tty = tty.to_str().unwrap().to_string();

        // Everything the terminal shows (the prompt, and any echo of what
        // is typed) arrives on the master side.
        let emitted = Arc::new(Mutex::new(Vec::new()));
        let mut reader = pair.master.try_clone_reader().unwrap();
        let sink = emitted.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 256];
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                sink.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        let seen = || String::from_utf8_lossy(&emitted.lock().unwrap()).into_owned();

        // The production wiring: the terminal is both where the prompt is
        // written and where the answer is read from.
        let (done_tx, done) = mpsc::channel();
        let device = tty.clone();
        std::thread::spawn(move || {
            let out = std::fs::OpenOptions::new()
                .write(true)
                .open(&device)
                .unwrap();
            let p =
                TerminalPrompter::new(std::io::empty(), out, SecretInput::Terminal(Some(device)));
            let _ = done_tx.send(p.unlock(&unlock_req(1)).map(passphrase));
        });

        // Wait for the read to turn echo off before typing, rather than
        // sleeping and hoping: typing while echo is still on would fail
        // for a reason that has nothing to do with gage.
        let probe = std::fs::File::open(&tty).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            // SAFETY: termios is plain data, filled in by tcgetattr on an
            // fd this test holds open.
            let lflag = unsafe {
                let mut t: libc::termios = std::mem::zeroed();
                assert_eq!(
                    libc::tcgetattr(probe.as_raw_fd(), &mut t),
                    0,
                    "reading terminal settings"
                );
                t.c_lflag
            };
            if lflag & libc::ECHO == 0 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the prompter never disabled terminal echo"
            );
            std::thread::sleep(Duration::from_millis(2));
        }

        const SECRET: &str = "hunter2";
        let mut writer = pair.master.take_writer().unwrap();
        writer.write_all(format!("{SECRET}\n").as_bytes()).unwrap();

        let got = done
            .recv_timeout(Duration::from_secs(10))
            .expect("the prompter never returned");
        assert_eq!(got.unwrap(), SECRET);

        // Any echo is emitted the moment the line is written; this settle
        // only lets the copier thread catch up.
        std::thread::sleep(Duration::from_millis(100));
        let out = seen();
        assert!(
            out.contains("Enter passphrase"),
            "the terminal never showed a prompt:\n{out:?}"
        );
        assert!(!out.contains(SECRET), "the passphrase was echoed:\n{out:?}");
        assert!(
            out.contains("personal"),
            "the prompt doesn't name the vault:\n{out:?}"
        );
        drop(pair.slave);
    }
}
