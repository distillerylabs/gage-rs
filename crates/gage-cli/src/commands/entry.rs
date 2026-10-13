//! `gage insert`, `cat`, `rm` and `ls`: the one-shot entry commands. Each
//! one resolves its vault, unlocks it, does one thing and drops the
//! identity (nothing is cached between invocations). What they do lives in
//! the library; this is flags, input modes and rendering.
//!
//! `-e/--edit` (P3 · M5) and `ls -H/--header` (P6 · M12) are not
//! registered yet.

use std::collections::BTreeMap;

use clap::{Arg, ArgAction, ArgMatches};
use gage::entry::{self, CandidateList, ListEntry, NewEntry, ResolveError, WriteError};
use gage::exitcode::Code;
use gage::identity::Identity;
use gage::prompter::Prompter;
use gage::vault::Vault;
use zeroize::Zeroizing;

use super::{arg, dirs, fail, load_global, out};
use crate::Io;
use crate::error::CliError;

pub const LS_LONG_ABOUT: &str = "List entries with their ids, dates, and last writer.

Columns: title, short id, created, updated, updated_by.
Dates are UTC, to the day; `gage cat <query>` shows the full entry.";

/// How many characters of an entry's UUID `ls` shows.
const SHORT_ID_LEN: usize = 8;

/// What `ls` shows where an entry has no value for a column.
const MISSING: &str = "-";

fn use_flag() -> Arg {
    Arg::new("use")
        .short('u')
        .long("use")
        .value_name("NAME")
        .help("vault to operate on (default: the current vault)")
}

pub fn flags(name: &str) -> Vec<Arg> {
    match name {
        "insert" => vec![
            Arg::new("description")
                .long("description")
                .value_name("TEXT")
                .help("optional description"),
            Arg::new("field")
                .long("field")
                .value_name("NAME=VALUE")
                .action(ArgAction::Append)
                .help("set a structured field as NAME=VALUE (repeatable)"),
            Arg::new("force")
                .short('f')
                .long("force")
                .action(ArgAction::SetTrue)
                .help("allow inserting a duplicate title"),
            Arg::new("multiline")
                .short('m')
                .long("multiline")
                .action(ArgAction::SetTrue)
                .conflicts_with("value-stdin")
                .help("read the value as multiple lines from the terminal until EOF"),
            use_flag(),
            Arg::new("value-stdin")
                .long("value-stdin")
                .action(ArgAction::SetTrue)
                .help("read the value verbatim from stdin (one trailing newline trimmed)"),
        ],
        _ => vec![use_flag()],
    }
}

// --- the one-shot handler ---------------------------------------------------

/// The vault `--use` names, or the current one. An unregistered name (or no
/// current vault) fails here, before anything is prompted for.
fn open_vault(matches: &ArgMatches, io: &Io) -> Result<Vault, CliError> {
    let (_, global) = load_global(io)?;
    let given = matches.get_one::<String>("use").map(String::as_str);
    let name = global.name_or_current(given)?;
    Ok(Vault::from_registration(
        &dirs(io)?,
        name,
        global.entry(name)?,
    )?)
}

/// Unlocks `vault`, runs `f` with the identity, and drops it. The identity
/// zeroizes and releases its page lock in `Drop`, so every way out of `f`
/// (including an error) closes it: there is no `close` to forget.
pub(crate) fn with_unlocked<T>(
    vault: &Vault,
    prompter: &dyn Prompter,
    f: impl FnOnce(&Identity) -> Result<T, CliError>,
) -> Result<T, CliError> {
    let identity = vault.unlock(prompter)?;
    f(&identity)
}

/// An unresolvable query as an error. An ambiguous one lists the
/// candidates, with full ids so the next attempt can name one exactly.
fn resolve_failure(err: ResolveError) -> CliError {
    match err {
        ResolveError::Ambiguous(list) => ambiguous(&list),
        other => other.into(),
    }
}

fn write_failure(err: WriteError) -> CliError {
    match err {
        WriteError::Resolve(e) => resolve_failure(e),
        other => other.into(),
    }
}

fn ambiguous(list: &CandidateList) -> CliError {
    let mut msg = format!(
        "{:?} matches {} entries; name one by its id:",
        list.query,
        list.candidates.len()
    );
    for c in &list.candidates {
        msg.push_str(&format!("\n  {}  {}", c.id, c.title));
    }
    fail(Code::Ambiguous, msg)
}

// --- insert -----------------------------------------------------------------

pub fn insert(matches: &ArgMatches, io: &mut Io, prompter: &dyn Prompter) -> Result<(), CliError> {
    let title = arg(matches, "title");
    // Everything knowable from the flags fails before any unlock.
    let fields = parse_field_flags(
        &matches
            .get_many::<String>("field")
            .into_iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>(),
    )?;
    let vault = open_vault(matches, io)?;
    // Unlock first, then ask for the value: a wrong passphrase must not
    // make anyone type a secret for nothing.
    let id = with_unlocked(&vault, prompter, |identity| {
        let value = read_value(matches, io, prompter, title)?;
        vault
            .insert(
                identity,
                NewEntry {
                    title: title.to_string(),
                    description: matches
                        .get_one::<String>("description")
                        .cloned()
                        .unwrap_or_default(),
                    value: value.to_string(),
                    fields,
                },
                matches.get_flag("force"),
            )
            .map_err(write_failure)
    })?;
    out(io, &[format!("gage: inserted \"{title}\" ({id})")]);
    Ok(())
}

/// The entry's value, from whichever mode was chosen: stdin verbatim minus
/// one trailing newline, stdin verbatim to EOF, or a masked prompt.
fn read_value(
    matches: &ArgMatches,
    io: &mut Io,
    prompter: &dyn Prompter,
    title: &str,
) -> Result<Zeroizing<String>, CliError> {
    if matches.get_flag("value-stdin") {
        return Ok(trim_one_trailing_newline(read_stdin(io)?));
    }
    if matches.get_flag("multiline") {
        if io.stdin_is_terminal {
            let _ = writeln!(
                io.stderr,
                "gage: enter the value; finish with Ctrl-D on an empty line"
            );
        }
        return read_stdin(io);
    }
    Ok(prompter.value(title)?)
}

/// All of stdin, which must be UTF-8: an entry's value is text.
fn read_stdin(io: &mut Io) -> Result<Zeroizing<String>, CliError> {
    let mut bytes = Zeroizing::new(Vec::new());
    io.stdin
        .read_to_end(&mut bytes)
        .map_err(|e| CliError::internal(anyhow::anyhow!("reading stdin: {e}")))?;
    match std::str::from_utf8(&bytes) {
        Ok(text) => Ok(Zeroizing::new(text.to_string())),
        Err(_) => Err(fail(Code::Usage, "the value on stdin is not valid UTF-8")),
    }
}

/// Drops exactly one trailing `\n` or `\r\n`: the newline `echo` and a
/// shell here-string add, and no more, so a value that really ends in a
/// blank line keeps it.
pub fn trim_one_trailing_newline(mut s: Zeroizing<String>) -> Zeroizing<String> {
    if s.ends_with("\r\n") {
        let len = s.len() - 2;
        s.truncate(len);
    } else if s.ends_with('\n') {
        let len = s.len() - 1;
        s.truncate(len);
    }
    s
}

/// `--field NAME=VALUE` flags as a map. Splits on the first `=` only (a
/// value may contain `=`); every whitespace character is removed from NAME
/// (a stray space must not create a key nobody can visibly ask for) and
/// VALUE is kept verbatim, empty included. A missing `=`, a NAME that is
/// empty once stripped, or one NAME given twice is a usage error: there is
/// no silent last-wins, and no `-f` override, since a map can't hold both.
pub fn parse_field_flags(raw: &[String]) -> Result<BTreeMap<String, String>, CliError> {
    let mut fields = BTreeMap::new();
    for flag in raw {
        let Some((name, value)) = flag.split_once('=') else {
            return Err(fail(
                Code::Usage,
                format!("--field {flag:?} must be NAME=VALUE"),
            ));
        };
        let name: String = name.chars().filter(|c| !c.is_whitespace()).collect();
        if name.is_empty() {
            return Err(fail(
                Code::Usage,
                format!("--field {flag:?} has an empty NAME"),
            ));
        }
        if fields.insert(name.clone(), value.to_string()).is_some() {
            return Err(fail(
                Code::Usage,
                format!("--field {name:?} was given more than once"),
            ));
        }
    }
    Ok(fields)
}

// --- cat / rm ---------------------------------------------------------------

/// The entry's YAML exactly as it is stored inside the ciphertext.
pub fn cat(matches: &ArgMatches, io: &mut Io, prompter: &dyn Prompter) -> Result<(), CliError> {
    let query = arg(matches, "query");
    let vault = open_vault(matches, io)?;
    let yaml = with_unlocked(&vault, prompter, |identity| {
        let id = vault.resolve(identity, query).map_err(resolve_failure)?;
        let e = vault.read_entry(id, identity)?;
        Ok(entry::marshal(&e)?)
    })?;
    let _ = io.stdout.write_all(&yaml);
    Ok(())
}

pub fn rm(matches: &ArgMatches, io: &mut Io, prompter: &dyn Prompter) -> Result<(), CliError> {
    let query = arg(matches, "query");
    let vault = open_vault(matches, io)?;
    let id = with_unlocked(&vault, prompter, |identity| {
        vault.remove(identity, query).map_err(write_failure)
    })?;
    out(io, &[format!("gage: removed {id}")]);
    Ok(())
}

// --- ls ---------------------------------------------------------------------

pub fn ls(matches: &ArgMatches, io: &mut Io, prompter: &dyn Prompter) -> Result<(), CliError> {
    let vault = open_vault(matches, io)?;
    let rows = with_unlocked(&vault, prompter, |identity| Ok(vault.list(identity)?))?;
    out(io, &render_ls(&rows));
    Ok(())
}

/// One line per entry: title padded to the widest (counted in characters),
/// short id, created and updated as UTC days, and the last writer. Two
/// spaces between columns, no trailing spaces, `-` for anything missing.
/// `rows` come sorted; an empty vault prints nothing.
pub fn render_ls(rows: &[ListEntry]) -> Vec<String> {
    let cells: Vec<[String; 5]> = rows
        .iter()
        .map(|r| {
            let day = |t: Option<entry::Timestamp>| {
                t.map(|t| t.to_string().chars().take(10).collect::<String>())
            };
            let present = |s: &str| (!s.is_empty()).then(|| s.to_string());
            let or_dash = |s: Option<String>| s.unwrap_or_else(|| MISSING.to_string());
            [
                or_dash(present(&r.title)),
                short_id(&r.id.to_string()).to_string(),
                or_dash(day(r.created)),
                or_dash(day(r.updated)),
                or_dash(present(&r.updated_by)),
            ]
        })
        .collect();
    let width = cells
        .iter()
        .map(|c| c[0].chars().count())
        .max()
        .unwrap_or(0);
    cells
        .iter()
        .map(|c| {
            let pad = " ".repeat(width - c[0].chars().count());
            format!("{}{pad}  {}  {}  {}  {}", c[0], c[1], c[2], c[3], c[4])
                .trim_end()
                .to_string()
        })
        .collect()
}

/// The leading [`SHORT_ID_LEN`] characters of `id`; anything shorter is
/// left alone.
pub fn short_id(id: &str) -> &str {
    match id.char_indices().nth(SHORT_ID_LEN) {
        Some((end, _)) => &id[..end],
        None => id,
    }
}
