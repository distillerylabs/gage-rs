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
use std::io::{BufRead, BufReader, Read, Write};

use gage::prompter::{Kind, PromptError, Prompter, Purpose, UnlockRequest, UnlockResponse};
use zeroize::Zeroizing;

/// gage's wrong-passphrase retry policy, which lives here rather than in
/// the library on purpose: the library re-asks for as long as its prompter
/// answers, so "three attempts" is this prompter refusing the fourth.
/// Three because a passphrase is typed blind: one punishes a typo, and an
/// unlimited count turns a lock screen into an unrate-limited oracle. The
/// file is on disk and an attacker with it can ignore gage entirely, so
/// the number is about ergonomics; slowing an attack is scrypt's job.
pub const MAX_PASSPHRASE_ATTEMPTS: u32 = 3;

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
    /// Buffered across calls: a fresh reader per call would read ahead and
    /// drop whatever followed the line it returned, which in a `--stdin`
    /// script is the next command.
    input: RefCell<BufReader<R>>,
    out: RefCell<W>,
    secrets: SecretInput,
}

impl<R: Read, W: Write> TerminalPrompter<R, W> {
    pub fn new(input: R, out: W, secrets: SecretInput) -> TerminalPrompter<R, W> {
        TerminalPrompter {
            input: RefCell::new(BufReader::new(input)),
            out: RefCell::new(out),
            secrets,
        }
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
    /// at all is not.
    fn read_line(&self) -> Result<Zeroizing<String>, PromptError> {
        let mut line = Zeroizing::new(String::new());
        if self.input.borrow_mut().read_line(&mut line)? == 0 {
            return Err(PromptError::Cancelled);
        }
        let len = line.trim_end_matches(['\r', '\n']).len();
        line.truncate(len);
        Ok(line)
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

    fn warn(&self, msg: &str) {
        self.say(&format!("{msg}\n"));
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use gage::exitcode::{Code, HasCode};

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
