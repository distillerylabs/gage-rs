//! The entry commands: `insert`, `show`, `cat`, `edit`, `rename`,
//! `generate`, `rm`, `ls`, `search` and `reindex`. One-shot, each resolves its vault, unlocks
//! it, does one thing and drops the identity; in a session it borrows the
//! key the session holds instead (see [`Keys`]). What they do lives in the
//! library; this is flags, input modes and rendering. Every query goes
//! through the library's one resolver. An ambiguous one fails listing its
//! candidates one-shot, where there is nobody to ask, and goes to the
//! session's picker in a session. In a session, reads that need every
//! entry's metadata (`ls`, resolution, `search`'s title half) come from its
//! metadata index, and every write tells the index what changed.
//!
//! `-c/--clip` and `-q/--qr` (on `show` and `generate`) and `ls
//! -H/--header` are P6 · M12's.

use std::collections::BTreeMap;

use clap::{Arg, ArgAction, ArgMatches};
use gage::entry::{
    self, CandidateList, Entry, ListEntry, NewEntry, ResolveError, Timestamp, WriteError,
};
use gage::exitcode::Code;
use gage::generate::{self, GenerateOptions};
use gage::identity::Identity;
use gage::prompter::Prompter;
use gage::session::SessionError;
use gage::vault::Vault;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{Keys, arg, catfmt, dirs, fail, load_global, out};
use crate::Io;
use crate::editor;
use crate::error::CliError;

pub const LS_LONG_ABOUT: &str = "List entries with their ids, dates, and last writer.

Columns: title, short id, created, updated, updated_by.
Dates are UTC, to the day; `gage cat <query>` shows the full entry.
--header prints a labelled table instead of plain rows.";

/// How many characters of an entry's UUID `ls` shows.
const SHORT_ID_LEN: usize = 8;

/// What `ls` shows where an entry has no value for a column.
const MISSING: &str = "-";

pub(super) fn use_flag() -> Arg {
    Arg::new("use")
        .short('u')
        .long("use")
        .value_name("NAME")
        .help("vault to operate on (default: the current vault)")
}

fn description_flag() -> Arg {
    Arg::new("description")
        .long("description")
        .value_name("TEXT")
        .help("optional description")
}

fn force_flag(help: &'static str) -> Arg {
    Arg::new("force")
        .short('f')
        .long("force")
        .action(ArgAction::SetTrue)
        .help(help)
}

pub fn flags(name: &str) -> Vec<Arg> {
    match name {
        "insert" => vec![
            description_flag(),
            // One of the three ways to supply the value, and the one that
            // also sets fields, so it excludes the other two and --field.
            Arg::new("edit")
                .short('e')
                .long("edit")
                .action(ArgAction::SetTrue)
                .conflicts_with_all(["multiline", "value-stdin", "field"])
                .help(
                    "open a template in $EDITOR to fill in value/fields (and optionally title/description)",
                ),
            Arg::new("field")
                .long("field")
                .value_name("NAME=VALUE")
                .action(ArgAction::Append)
                .help(
                    "set a structured field as NAME=VALUE (repeatable; mutually exclusive with -e/--edit)",
                ),
            force_flag("allow inserting a duplicate title"),
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
        "show" => vec![
            Arg::new("field")
                .long("field")
                .value_name("NAME")
                .help("print one entry from fields instead of the value"),
            use_flag(),
        ],
        "rename" => vec![force_flag("allow renaming to a duplicate title"), use_flag()],
        "generate" => vec![
            description_flag(),
            force_flag("allow inserting a duplicate title"),
            Arg::new("length")
                .short('l')
                .long("length")
                .value_name("N")
                .value_parser(clap::value_parser!(usize))
                .help(format!(
                    "length of the generated value (default {}, minimum {})",
                    generate::DEFAULT_LENGTH,
                    generate::MIN_LENGTH
                )),
            Arg::new("no-symbols")
                .long("no-symbols")
                .action(ArgAction::SetTrue)
                .help("draw from letters and digits only"),
            use_flag(),
        ],
        "ls" => vec![
            Arg::new("header")
                .short('H')
                .long("header")
                .action(ArgAction::SetTrue)
                .help("print a labelled, |-delimited table instead of plain unlabelled rows"),
            use_flag(),
        ],
        _ => vec![use_flag()],
    }
}

// --- the handler: both modes ------------------------------------------------

/// The vault `--use` names, or the current one. An unregistered name (or no
/// current vault) fails here, before anything is prompted for.
pub(super) fn open_vault(matches: &ArgMatches, io: &Io) -> Result<Vault, CliError> {
    let (_, global) = load_global(io)?;
    let given = matches.get_one::<String>("use").map(String::as_str);
    let name = global.name_or_current(given)?;
    Ok(Vault::from_registration(
        &dirs(io)?,
        name,
        global.entry(name)?,
    )?)
}

/// `--use`, if given: in a session, the vault a command uses without
/// changing which one is current.
pub(super) fn use_name(matches: &ArgMatches) -> Option<&str> {
    matches.get_one::<String>("use").map(String::as_str)
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

impl Keys<'_, '_> {
    /// Runs `f` with the vault `--use` names (or the current one) and its
    /// identity: unlocked for this command and dropped after it one-shot,
    /// borrowed from the session (unlocking there once, if needed) in one.
    pub(super) fn with_vault<T>(
        &mut self,
        matches: &ArgMatches,
        io: &mut Io,
        f: impl FnOnce(&mut Io, &Vault, &Identity) -> Result<T, CliError>,
    ) -> Result<T, CliError> {
        match self {
            Keys::OneShot(prompter) => {
                let vault = open_vault(matches, io)?;
                with_unlocked(&vault, *prompter, |identity| f(io, &vault, identity))
            }
            Keys::Session(session) => {
                let (vault, identity) =
                    session.vault(use_name(matches)).map_err(session_failure)?;
                f(io, vault, identity)
            }
        }
    }

    /// Runs `f` with the entry `query` names, decrypted. An ambiguous
    /// query fails listing its candidates one-shot, and asks which was
    /// meant in a session.
    fn with_resolved<T>(
        &mut self,
        matches: &ArgMatches,
        io: &mut Io,
        query: &str,
        f: impl FnOnce(&mut Io, &Vault, &Identity, Uuid, Entry) -> Result<T, CliError>,
    ) -> Result<T, CliError> {
        if let Keys::Session(session) = self {
            let name = use_name(matches);
            let (id, e) = session.resolve(name, query).map_err(session_failure)?;
            let (vault, identity) = session.vault(name).map_err(session_failure)?;
            return f(io, vault, identity, id, e);
        }
        self.with_vault(matches, io, |io, vault, identity| {
            let (id, e) = vault.resolve(identity, query).map_err(resolve_failure)?;
            f(io, vault, identity, id, e)
        })
    }

    /// What `rm` and `rename` hand the library, which resolves it again
    /// under the write lock. One-shot, the query as typed. In a session,
    /// the id the query resolved to, after asking on ambiguity: the
    /// library's exact-id stage can then only match the entry chosen, so
    /// these two prompt like every other command instead of failing.
    fn mutation_query(&mut self, matches: &ArgMatches, query: &str) -> Result<String, CliError> {
        match self {
            Keys::OneShot(_) => Ok(query.to_string()),
            Keys::Session(session) => {
                let (id, _) = session
                    .resolve(use_name(matches), query)
                    .map_err(session_failure)?;
                Ok(id.to_string())
            }
        }
    }
}

impl Keys<'_, '_> {
    /// Tells a session's index about an entry a write produced, reading it
    /// back by id. A no-op one-shot, and for a vault with no index.
    fn refresh(&mut self, matches: &ArgMatches, id: Uuid) {
        if let Keys::Session(session) = self {
            session.refresh_entry(use_name(matches), id);
        }
    }

    /// Tells a session's index about an entry a write has in hand.
    fn note(&mut self, matches: &ArgMatches, id: Uuid, e: &Entry) {
        if let Keys::Session(session) = self {
            session.note_entry(use_name(matches), id, e);
        }
    }

    /// Tells a session's index an entry is gone.
    fn forget(&mut self, matches: &ArgMatches, id: Uuid) {
        if let Keys::Session(session) = self {
            session.forget_entry(use_name(matches), id);
        }
    }
}

/// An unresolvable query as an error. An ambiguous one lists the
/// candidates, with full ids so the next attempt can name one exactly.
pub(super) fn resolve_failure(err: ResolveError) -> CliError {
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

/// A session call's failure. A candidate list comes back only when the
/// human declined to pick, and renders as one-shot's does.
pub(crate) fn session_failure(err: SessionError) -> CliError {
    match err {
        SessionError::Resolve(e) => resolve_failure(e),
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

pub fn insert(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    let title = arg(matches, "title");
    // Both read stdin to EOF, and in a session stdin is the session's own
    // input: they would swallow the rest of it and end the session as a
    // side effect of one insert.
    if keys.in_session() && (matches.get_flag("multiline") || matches.get_flag("value-stdin")) {
        return Err(fail(
            Code::Usage,
            "-m/--multiline and --value-stdin read stdin to EOF, which is this session's own input; use -e/--edit here, or run `gage insert` from your shell",
        ));
    }
    if matches.get_flag("edit") {
        return insert_edit(matches, io, keys, title);
    }
    // Everything knowable from the flags fails before any unlock.
    let fields = parse_field_flags(
        &matches
            .get_many::<String>("field")
            .into_iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>(),
    )?;
    let prompter = keys.prompter();
    // Unlock first, then ask for the value: a wrong passphrase must not
    // make anyone type a secret for nothing.
    let id = keys.with_vault(matches, io, |io, vault, identity| {
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
                    ..NewEntry::default()
                },
                matches.get_flag("force"),
                prompter,
            )
            .map_err(write_failure)
    })?;
    keys.refresh(matches, id);
    out(io, &[format!("gage: inserted \"{title}\" ({id})")]);
    Ok(())
}

/// `insert -e`: the template (title and description from the command line,
/// value and fields empty) goes through `$EDITOR`, and what comes back is
/// inserted, under the edited title for both the entry and the
/// duplicate-title check. Nothing is written if the file comes back
/// unchanged, or with value and fields both still empty (git commit's
/// "empty message aborts" convention; a title or description alone is not
/// something to save).
fn insert_edit(
    matches: &ArgMatches,
    io: &mut Io,
    keys: &mut Keys,
    title: &str,
) -> Result<(), CliError> {
    let prompter = keys.prompter();
    let (id, title) = keys.with_vault(matches, io, |io, vault, identity| {
        let now = Timestamp::now();
        let stub = Entry {
            title: title.to_string(),
            description: matches
                .get_one::<String>("description")
                .cloned()
                .unwrap_or_default(),
            created: Some(now),
            updated: Some(now),
            updated_by: identity.device().to_string(),
            ..Entry::default()
        };
        let edited = editor::edit_yaml(io.editor.as_deref(), &stub)?;
        let e = edited.entry;
        if edited.unchanged || (e.value.is_empty() && e.fields.is_empty()) {
            return Err(fail(
                Code::Usage,
                "insert -e aborted: nothing to save (the file was unchanged, or value and fields were both left empty)",
            ));
        }
        let id = vault
            .insert(
                identity,
                NewEntry {
                    title: e.title.clone(),
                    description: e.description,
                    value: e.value,
                    fields: e.fields,
                    extra: e.extra,
                },
                matches.get_flag("force"),
                prompter,
            )
            .map_err(write_failure)?;
        Ok((id, e.title))
    })?;
    keys.refresh(matches, id);
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

// --- show / cat / edit / rename / rm ---------------------------------------

/// The entry's value alone, or with `--field NAME` one of its fields, for
/// piping. Exactly one trailing newline: none is added to a value that
/// already ends in one, so a multi-line value prints as stored.
pub fn show(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    let query = arg(matches, "query");
    let field = matches.get_one::<String>("field");
    let value = keys.with_resolved(matches, io, query, |_, _, _, _, e| {
        let value = match field {
            Some(name) => e.field(name)?,
            None => &e.value,
        };
        Ok(Zeroizing::new(value.to_string()))
    })?;
    let _ = io.stdout.write_all(value.as_bytes());
    if !value.ends_with('\n') {
        let _ = io.stdout.write_all(b"\n");
    }
    Ok(())
}

/// The entry's full plaintext YAML, for scripting and piping, with the
/// display changes of [`catfmt`]: an `id` line, aligned keys, and block
/// values flush left.
pub fn cat(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    let query = arg(matches, "query");
    let shown = keys.with_resolved(matches, io, query, |_, _, _, id, e| {
        let yaml = entry::marshal(&e)?;
        Ok(Zeroizing::new(catfmt::align_cat_output(
            short_id(&id.to_string()),
            &yaml,
        )))
    })?;
    let _ = io.stdout.write_all(&shown);
    Ok(())
}

/// The entry's whole YAML through `$EDITOR`. Commits whenever the file
/// parses, changed or not (as `git commit --amend` with no new message
/// still amends), with `created` kept from the original and
/// `updated`/`updated_by` stamped fresh whatever the file says.
pub fn edit(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    let query = arg(matches, "query");
    let prompter = keys.prompter();
    let (id, edited) =
        keys.with_resolved(matches, io, query, |io, vault, identity, id, original| {
            let mut edited = editor::edit_yaml(io.editor.as_deref(), &original)?.entry;
            edited.restamp_after_edit(&original, identity);
            vault.update(id, &edited, prompter).map_err(write_failure)?;
            Ok((id, edited))
        })?;
    keys.note(matches, id, &edited);
    out(
        io,
        &[format!(
            "gage: updated \"{}\" ({})",
            edited.title,
            short_id(&id.to_string())
        )],
    );
    Ok(())
}

/// A title-only edit, no `$EDITOR`.
pub fn rename(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    let query = keys.mutation_query(matches, arg(matches, "query"))?;
    let new_title = arg(matches, "new-title");
    let prompter = keys.prompter();
    let id = keys.with_vault(matches, io, |_, vault, identity| {
        vault
            .rename(
                identity,
                &query,
                new_title,
                matches.get_flag("force"),
                prompter,
            )
            .map_err(write_failure)
    })?;
    keys.refresh(matches, id);
    out(
        io,
        &[format!(
            "gage: renamed to \"{new_title}\" ({})",
            short_id(&id.to_string())
        )],
    );
    Ok(())
}

pub fn rm(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    let query = keys.mutation_query(matches, arg(matches, "query"))?;
    let prompter = keys.prompter();
    let id = keys.with_vault(matches, io, |_, vault, identity| {
        vault
            .remove(identity, &query, prompter)
            .map_err(write_failure)
    })?;
    keys.forget(matches, id);
    out(io, &[format!("gage: removed {id}")]);
    Ok(())
}

// --- generate ---------------------------------------------------------------

/// Like `insert`, with the value drawn by the library. The confirmation
/// never includes the value: a fresh secret nobody has read yet is the last
/// thing that belongs in a scrollback (`-c`/`-q`, the ways to see it at
/// creation, are P6's).
pub fn generate(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    let title = arg(matches, "title");
    // Drawn first, so a too-short length is refused before any unlock.
    let value = Zeroizing::new(generate::generate_value(GenerateOptions {
        length: matches.get_one::<usize>("length").copied().unwrap_or(0),
        no_symbols: matches.get_flag("no-symbols"),
    })?);
    let prompter = keys.prompter();
    let id = keys.with_vault(matches, io, |_, vault, identity| {
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
                    ..NewEntry::default()
                },
                matches.get_flag("force"),
                prompter,
            )
            .map_err(write_failure)
    })?;
    keys.refresh(matches, id);
    out(io, &[format!("gage: generated \"{title}\" ({id})")]);
    Ok(())
}

// --- ls ---------------------------------------------------------------------

/// One-shot, a fresh decrypt of the vault; in a session, its index. The
/// rows are the same either way, through one renderer.
pub fn ls(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    let rows = match keys {
        Keys::Session(session) => session.list(use_name(matches)).map_err(session_failure)?,
        Keys::OneShot(_) => {
            keys.with_vault(matches, io, |_, vault, identity| Ok(vault.list(identity)?))?
        }
    };
    let lines = if matches.get_flag("header") {
        render_ls_table(&rows)
    } else {
        render_ls(&rows)
    };
    out(io, &lines);
    Ok(())
}

// --- search / reindex -------------------------------------------------------

/// `title  shortid` per match, sorted, and nothing at all for no matches
/// (so `| wc -l` counts them). Never the matched text: a body match is
/// where a secret matched.
pub fn search(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    let pattern = arg(matches, "pattern");
    let results = match keys {
        Keys::Session(session) => session
            .search(use_name(matches), pattern)
            .map_err(session_failure)?,
        Keys::OneShot(_) => keys.with_vault(matches, io, |_, vault, identity| {
            Ok(vault.search(identity, pattern)?)
        })?,
    };
    let lines: Vec<String> = results
        .iter()
        .map(|r| format!("{}  {}", r.title, short_id(&r.id.to_string())))
        .collect();
    out(io, &lines);
    Ok(())
}

/// Rebuilds a session's index from a full decrypt. One-shot there is no
/// cache: it checks the vault resolves (without unlocking) and says so.
pub fn reindex(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    match keys {
        Keys::Session(session) => {
            session
                .reindex(use_name(matches))
                .map_err(session_failure)?;
            out(io, &["gage: index rebuilt".to_string()]);
        }
        Keys::OneShot(_) => {
            open_vault(matches, io)?;
            out(
                io,
                &["gage: one-shot mode does not cache metadata; nothing to reindex".to_string()],
            );
        }
    }
    Ok(())
}

/// What `ls` shows per entry, in column order: title, short id, created,
/// updated (UTC days) and the last writer, `-` for anything missing.
fn ls_cells(rows: &[ListEntry]) -> Vec<[String; 5]> {
    rows.iter()
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
        .collect()
}

/// One line per entry: title padded to the widest (counted in characters),
/// short id, created and updated as UTC days, and the last writer. Two
/// spaces between columns, no trailing spaces, `-` for anything missing.
/// `rows` come sorted; an empty vault prints nothing.
pub fn render_ls(rows: &[ListEntry]) -> Vec<String> {
    let cells = ls_cells(rows);
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

/// `ls --header`: the same cells as a labelled table. A header row, a row
/// of dashes broken with `+` where the `|` separators fall, then one
/// ` | `-delimited row per entry. Every column but the last is padded to
/// the widest value it holds, header included. The last is never padded
/// (a long device name would leave trailing spaces on every shorter row),
/// so its dashes are sized from its label alone. An empty vault still
/// prints nothing: a header over zero rows is noise, not a table.
pub fn render_ls_table(rows: &[ListEntry]) -> Vec<String> {
    const COLUMNS: [&str; 5] = ["title", "id", "created at", "updated at", "updated by"];
    if rows.is_empty() {
        return Vec::new();
    }
    let cells = ls_cells(rows);
    let mut width = COLUMNS.map(|c| c.chars().count());
    for cell in &cells {
        for (w, v) in width.iter_mut().zip(cell).take(COLUMNS.len() - 1) {
            *w = (*w).max(v.chars().count());
        }
    }
    let join = |row: &[String; 5]| -> String {
        row.iter()
            .enumerate()
            .map(|(i, v)| match i {
                i if i == COLUMNS.len() - 1 => v.clone(),
                _ => format!("{v}{}", " ".repeat(width[i] - v.chars().count())),
            })
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let dashes: Vec<String> = width.iter().map(|&w| "-".repeat(w)).collect();
    let mut lines = vec![join(&COLUMNS.map(String::from)), dashes.join("-+-")];
    lines.extend(cells.iter().map(join));
    lines
}

/// The leading [`SHORT_ID_LEN`] characters of `id`; anything shorter is
/// left alone.
pub fn short_id(id: &str) -> &str {
    match id.char_indices().nth(SHORT_ID_LEN) {
        Some((end, _)) => &id[..end],
        None => id,
    }
}
