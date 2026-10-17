//! How the REPL reads a command line: through `rustyline` on a real
//! terminal, or from `Io.stdin` otherwise (in-process tests, and anything
//! else that isn't a terminal). Both behind [`LineReader`], which is what
//! keeps the whole REPL testable without a pty (Q-LINE-EDITOR).
//!
//! Only command lines come through here. A passphrase, a value or a
//! candidate number is the prompter's to read: `rustyline` holds the
//! terminal only for the duration of one `readline` call, so the two
//! never read at the same time, and an answer typed at a prompt never
//! reaches the history.

use std::io::Read;

use rustyline::error::ReadlineError;

use crate::Io;
use crate::complete::SessionHelper;

/// What one read produced.
#[derive(Debug, PartialEq, Eq)]
pub enum ReadOutcome {
    Line(String),
    /// Ctrl-C: abandon the line being typed, keep the session.
    Interrupted,
    /// Ctrl-D, or the end of the input: leave the session.
    Eof,
}

pub trait LineReader {
    /// Shows `prompt` and reads one command line.
    fn read_line(&mut self, io: &mut Io, prompt: &str) -> std::io::Result<ReadOutcome>;

    /// Offers a submitted line to the editor's recall list.
    fn add_history(&mut self, _line: &str) {}
}

/// Lines from `Io.stdin`, the prompt written to `Io.stdout`. No editing
/// and no recall.
pub struct PlainLineReader;

impl LineReader for PlainLineReader {
    fn read_line(&mut self, io: &mut Io, prompt: &str) -> std::io::Result<ReadOutcome> {
        let _ = io.stdout.write_all(prompt.as_bytes());
        let _ = io.stdout.flush();
        Ok(match read_one_line(io.stdin)? {
            Some(line) => ReadOutcome::Line(line),
            None => ReadOutcome::Eof,
        })
    }
}

/// One line of `input` without its line ending, read a byte at a time:
/// whatever follows belongs to the prompter, which reads the same stream
/// (a passphrase on the line after `use personal`). A last line with no
/// newline is still a line; end of input with nothing read is `None`.
pub(crate) fn read_one_line(input: &mut dyn Read) -> std::io::Result<Option<String>> {
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match input.read(&mut byte) {
            Ok(0) if bytes.is_empty() => return Ok(None),
            Ok(0) => break,
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) => bytes.push(byte[0]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// How many lines the editor keeps for recall in one session. The file
/// itself is never truncated to it (see `history`).
const HISTORY_LIMIT: usize = 1000;

/// The real terminal: `rustyline`'s line editing, recall and Ctrl-C/Ctrl-D
/// handling, on Windows as well as Unix.
pub struct RustylineReader<'a> {
    editor: rustyline::Editor<SessionHelper<'a>, rustyline::history::DefaultHistory>,
}

impl<'a> RustylineReader<'a> {
    /// An editor whose recall starts from `previous` (the history file's
    /// lines), and whose Tab completes through `helper`. It never saves:
    /// the history file is gage's to write, and only submitted lines reach
    /// it, so completing never records anything.
    pub fn new(
        previous: &[String],
        helper: SessionHelper<'a>,
    ) -> rustyline::Result<RustylineReader<'a>> {
        let config = rustyline::Config::builder()
            .max_history_size(HISTORY_LIMIT)?
            .auto_add_history(false)
            // List the candidates on an ambiguous Tab (after filling their
            // common prefix) rather than cycling through them.
            .completion_type(rustyline::CompletionType::List)
            .build();
        let mut editor = rustyline::Editor::with_config(config)?;
        editor.set_helper(Some(helper));
        for line in previous {
            editor.add_history_entry(line.as_str())?;
        }
        Ok(RustylineReader { editor })
    }
}

impl LineReader for RustylineReader<'_> {
    fn read_line(&mut self, io: &mut Io, prompt: &str) -> std::io::Result<ReadOutcome> {
        let outcome = map_readline(self.editor.readline(prompt));
        if let Ok(ReadOutcome::Eof) = outcome {
            // Ctrl-D leaves the cursor on the prompt line; say what it did.
            let _ = io.stdout.write_all(b"exit\n");
        }
        outcome
    }

    fn add_history(&mut self, line: &str) {
        // Recall is a convenience; failing to record a line changes
        // nothing about the session.
        let _ = self.editor.add_history_entry(line);
    }
}

/// `rustyline`'s result as an outcome: Ctrl-C and Ctrl-D become theirs,
/// and every other error passes through as an I/O error, so a real
/// failure is never read as a cancellation.
pub fn map_readline(result: Result<String, ReadlineError>) -> std::io::Result<ReadOutcome> {
    match result {
        Ok(line) => Ok(ReadOutcome::Line(line)),
        Err(ReadlineError::Interrupted) => Ok(ReadOutcome::Interrupted),
        Err(ReadlineError::Eof) => Ok(ReadOutcome::Eof),
        Err(ReadlineError::Io(e)) => Err(e),
        Err(other) => Err(std::io::Error::other(other.to_string())),
    }
}
