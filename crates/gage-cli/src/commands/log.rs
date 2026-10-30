//! `log` and `history --decrypt`: one entry's git history at two danger
//! levels. `log` prints when an entry changed and nothing that was ever
//! encrypted; `history --decrypt` decrypts every past revision and prints
//! each as a diff against the one before it. What they walk lives in the
//! library (`Vault::log`, `Vault::history`); this is flags, the deleted-
//! entry fallback and rendering.

use crate::exitcode::ExitCode;
use clap::{Arg, ArgAction, ArgMatches};
use gage::Entry;
use gage::Identity;
use gage::Vault;
use gage::entry::ResolveError;
use gage::entry::history::{LogEntry, Revision};
use jiff::tz::TimeZone;
use uuid::Uuid;

use super::entry::{resolve_failure, session_failure, use_flag, use_name};
use super::{Keys, arg, fail, out};
use crate::Io;
use crate::error::CliError;

pub const LOG_LONG_ABOUT: &str =
    "Show an entry's commit history — timestamps only, nothing decrypted.

Commit timestamps for one entry. The commit messages themselves are entry
UUIDs and nothing more, so this reveals no titles — see `gage history
--decrypt` for the command that does show past values.";

pub const HISTORY_LONG_ABOUT: &str =
    "Decrypt every past revision of one entry (surfaces old secret values).

Walks one entry's revisions and decrypts each, so old values — including
passwords already rotated away — are printed. --decrypt is required and
names exactly that. Use `gage log` for timestamps without any plaintext.";

/// Local time with its offset.
const TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S %z";

/// How many characters of a commit hash a line shows.
const SHORT_HASH_LEN: usize = 7;

pub fn history_flags() -> Vec<Arg> {
    vec![
        Arg::new("decrypt")
            .long("decrypt")
            .action(ArgAction::SetTrue)
            .help("required: decrypt and print every past revision of the entry"),
        use_flag(),
    ]
}

// --- which entries ----------------------------------------------------------

/// `query` as an entry that may no longer exist: the id of a removed entry
/// is the only way to ask for its history, and git still has it. Only
/// after ordinary resolution finds nothing, so title-first resolution is
/// untouched and a substring never matches a removed entry.
fn historical_id(err: ResolveError, query: &str) -> Result<Uuid, ResolveError> {
    let full_id = matches!(query.len(), 32 | 36);
    match (&err, Uuid::try_parse(query)) {
        (ResolveError::NotFound { .. }, Ok(id)) if full_id => Ok(id),
        _ => Err(err),
    }
}

/// Runs `f` with the vault, its identity and the entries to walk: the one
/// `query` names (a removed entry by its full id), or every entry the
/// vault holds. An ambiguous query fails listing its candidates one-shot
/// and asks which was meant in a session.
fn with_targets<T>(
    keys: &mut Keys,
    matches: &ArgMatches,
    io: &mut Io,
    query: Option<&str>,
    f: impl FnOnce(&mut Io, &Vault, &Identity, Vec<Uuid>) -> Result<T, CliError>,
) -> Result<T, CliError> {
    let mut resolved = None;
    if let (Keys::Session(session), Some(query)) = (&mut *keys, query) {
        resolved = Some(match session.resolve(use_name(matches), query) {
            Ok((id, _)) => id,
            Err(gage::session::SessionError::Resolve(e)) => {
                historical_id(e, query).map_err(resolve_failure)?
            }
            Err(e) => return Err(session_failure(e)),
        });
    }
    keys.with_vault(matches, io, |io, vault, identity| {
        let targets = match (query, resolved) {
            (_, Some(id)) => vec![id],
            (Some(query), None) => vec![
                vault
                    .resolve(identity, query)
                    .map(|(id, _)| id)
                    .or_else(|e| historical_id(e, query))
                    .map_err(resolve_failure)?,
            ],
            (None, None) => vault.entry_ids()?,
        };
        f(io, vault, identity, targets)
    })
}

fn no_history(io: &mut Io, query: Option<&str>) {
    let what = match query {
        Some(_) => "that entry",
        None => "this vault",
    };
    let _ = writeln!(
        io.stderr,
        "gage: no history for {what}: nothing was ever committed"
    );
}

// --- log ---------------------------------------------------------------------

pub fn log(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    let query = matches.get_one::<String>("query").map(String::as_str);
    let groups = with_targets(keys, matches, io, query, |_, vault, _, targets| {
        targets
            .into_iter()
            .map(|id| Ok((id, vault.log(id)?)))
            .collect::<Result<Vec<_>, CliError>>()
    })?;
    let tz = TimeZone::system();
    let lines = timeline(groups, query.is_none(), &tz);
    if lines.is_empty() {
        no_history(io, query);
    }
    out(io, &lines);
    Ok(())
}

/// One line per commit, newest first. With several entries (`tag`), the
/// commits of all of them form one timeline, sorted on the timestamps and
/// not the rendered strings, so a daylight-saving change can't misorder
/// it. The sort is stable: commits in the same second keep each entry's
/// own newest-first order.
fn timeline(groups: Vec<(Uuid, Vec<LogEntry>)>, tag: bool, tz: &TimeZone) -> Vec<String> {
    let mut rows: Vec<(Uuid, LogEntry)> = groups
        .into_iter()
        .flat_map(|(id, log)| log.into_iter().map(move |l| (id, l)))
        .collect();
    rows.sort_by_key(|(_, l)| std::cmp::Reverse(l.when));
    rows.iter()
        .map(|(id, l)| {
            let mut line = format!(
                "{}  {}",
                format_time(l.when.as_jiff(), tz),
                short_hash(&l.hash)
            );
            if tag {
                line.push_str(&format!("  {}", super::short_id(&id.to_string())));
            }
            if l.deleted {
                line.push_str(" (deleted)");
            }
            line
        })
        .collect()
}

fn format_time(when: jiff::Timestamp, tz: &TimeZone) -> String {
    when.to_zoned(tz.clone()).strftime(TIME_FORMAT).to_string()
}

fn short_hash(hash: &str) -> &str {
    &hash[..hash.len().min(SHORT_HASH_LEN)]
}

// --- history -----------------------------------------------------------------

pub fn history(matches: &ArgMatches, io: &mut Io, keys: &mut Keys) -> Result<(), CliError> {
    // Before any unlock: the flag is the whole confirmation this command
    // asks for (there is no prompt), so it is checked first.
    if !matches.get_flag("decrypt") {
        return Err(fail(
            ExitCode::Usage,
            "history prints old secret values and needs --decrypt to say so: `gage history --decrypt <query>`; for timestamps only, use `gage log`",
        ));
    }
    let query = arg(matches, "query");
    let revisions = with_targets(
        keys,
        matches,
        io,
        Some(query),
        |_, vault, identity, targets| Ok(vault.history(targets[0], identity)?),
    )?;
    if revisions.is_empty() {
        no_history(io, Some(query));
        return Ok(());
    }
    out(io, &render_history(&revisions, &TimeZone::system()));
    Ok(())
}

fn render_history(revisions: &[Revision], tz: &TimeZone) -> Vec<String> {
    let mut lines = Vec::new();
    let mut previous: Option<&Entry> = None;
    for r in revisions {
        let deleted = if r.deleted { " (deleted)" } else { "" };
        lines.push(format!(
            "== {}  {}{deleted}",
            format_time(r.when.as_jiff(), tz),
            short_hash(&r.hash)
        ));
        if let Some(entry) = &r.entry {
            lines.extend(diff_entry_lines(previous, entry));
        }
        previous = r.entry.as_ref();
    }
    lines
}

/// `next` against the revision before it (`None` for the first, so every
/// line is an addition): ` ` for unchanged, `-` and `+` for what was
/// replaced. Compared are the parts of an entry a person writes: title,
/// description (only if either side has one), value, `updated_by`, and
/// `fields.<key>` over both sides' keys, sorted. `created` and `updated`
/// move with every revision and would only be noise.
fn diff_entry_lines(prev: Option<&Entry>, next: &Entry) -> Vec<String> {
    let mut rows: Vec<(String, Option<&str>, Option<&str>)> = Vec::new();
    let old = |f: fn(&Entry) -> &str| prev.map(f);
    rows.push(("title".into(), old(|e| &e.title), Some(&next.title)));
    if prev.is_some_and(|p| !p.description.is_empty()) || !next.description.is_empty() {
        rows.push((
            "description".into(),
            old(|e| &e.description),
            Some(&next.description),
        ));
    }
    rows.push(("value".into(), old(|e| &e.value), Some(&next.value)));
    rows.push((
        "updated_by".into(),
        old(|e| &e.updated_by),
        Some(&next.updated_by),
    ));
    let mut keys: Vec<&String> = prev
        .into_iter()
        .flat_map(|p| p.fields.keys())
        .chain(next.fields.keys())
        .collect();
    keys.sort();
    keys.dedup();
    for key in keys {
        rows.push((
            format!("fields.{key}"),
            prev.and_then(|p| p.fields.get(key)).map(String::as_str),
            next.fields.get(key).map(String::as_str),
        ));
    }

    let mut lines = Vec::new();
    for (name, old, new) in rows {
        match (old, new) {
            (Some(o), Some(n)) if o == n => push_value(&mut lines, ' ', &name, n),
            (o, n) => {
                if let Some(o) = o {
                    push_value(&mut lines, '-', &name, o);
                }
                if let Some(n) = n {
                    push_value(&mut lines, '+', &name, n);
                }
            }
        }
    }
    lines
}

/// `marker name: value`, with a multi-line value's later lines carrying
/// the same marker so a diff of a note still reads line by line.
fn push_value(lines: &mut Vec<String>, marker: char, name: &str, value: &str) {
    let mut parts = value.split('\n');
    lines.push(format!("{marker} {name}: {}", parts.next().unwrap_or("")));
    for part in parts {
        lines.push(format!("{marker}     {part}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gage::entry::Timestamp;

    fn entry(title: &str, value: &str) -> Entry {
        Entry {
            title: title.into(),
            value: value.into(),
            updated_by: "dev".into(),
            ..Entry::default()
        }
    }

    fn at(secs: i64, hash: &str, deleted: bool) -> LogEntry {
        LogEntry {
            hash: hash.into(),
            when: Timestamp::from_unix(secs).unwrap(),
            deleted,
        }
    }

    #[test]
    fn the_first_revision_is_all_additions() {
        let lines = diff_entry_lines(None, &entry("T", "v"));
        assert_eq!(lines, ["+ title: T", "+ value: v", "+ updated_by: dev"]);
    }

    #[test]
    fn a_description_appears_only_if_either_side_has_one() {
        let mut with = entry("T", "v");
        with.description = "d".into();
        let none = entry("T", "v");
        assert!(
            !diff_entry_lines(Some(&none), &none)
                .iter()
                .any(|l| l.contains("description"))
        );
        assert!(diff_entry_lines(Some(&none), &with).contains(&"+ description: d".to_string()));
        assert!(diff_entry_lines(Some(&with), &none).contains(&"- description: d".to_string()));
    }

    #[test]
    fn fields_are_compared_over_both_sides_keys_sorted() {
        let mut old = entry("T", "v");
        old.fields.insert("b".into(), "1".into());
        old.fields.insert("a".into(), "same".into());
        let mut new = entry("T", "v");
        new.fields.insert("a".into(), "same".into());
        new.fields.insert("c".into(), "3".into());
        let lines = diff_entry_lines(Some(&old), &new);
        let fields: Vec<&str> = lines
            .iter()
            .map(String::as_str)
            .filter(|l| l.contains("fields."))
            .collect();
        assert_eq!(
            fields,
            ["  fields.a: same", "- fields.b: 1", "+ fields.c: 3"]
        );
    }

    #[test]
    fn a_multiline_value_keeps_its_marker_on_every_line() {
        let lines = diff_entry_lines(Some(&entry("T", "a\nb")), &entry("T", "a\nc"));
        assert!(lines.contains(&"- value: a".to_string()));
        assert!(lines.contains(&"-     b".to_string()));
        assert!(lines.contains(&"+     c".to_string()));
    }

    #[test]
    fn a_vault_wide_timeline_interleaves_entries_by_time_not_by_entry() {
        let (a, b) = (Uuid::from_u128(0xaa << 120), Uuid::from_u128(0xbb << 120));
        let groups = vec![
            (
                a,
                vec![at(300, "aaaaaaaa", false), at(100, "a1111111", false)],
            ),
            (
                b,
                vec![at(400, "bbbbbbbb", true), at(200, "b2222222", false)],
            ),
        ];
        let lines = timeline(groups, true, &TimeZone::UTC);
        let hashes: Vec<&str> = lines
            .iter()
            .map(|l| l.split("  ").nth(1).unwrap())
            .collect();
        assert_eq!(hashes, ["bbbbbbb", "aaaaaaa", "b222222", "a111111"]);
        assert_eq!(
            lines[0],
            "1970-01-01 00:06:40 +0000  bbbbbbb  bb000000 (deleted)"
        );
        assert_eq!(lines[1], "1970-01-01 00:05:00 +0000  aaaaaaa  aa000000");
    }

    #[test]
    fn times_render_in_the_given_zone_with_their_offset() {
        let when = jiff::Timestamp::from_second(0).unwrap();
        let tz = TimeZone::fixed(jiff::tz::offset(-5));
        assert_eq!(format_time(when, &tz), "1969-12-31 19:00:00 -0500");
    }

    #[test]
    fn only_a_full_id_stands_in_for_a_removed_entry() {
        let id = Uuid::new_v4();
        let not_found = || ResolveError::NotFound {
            query: String::new(),
        };
        for q in [id.to_string(), id.simple().to_string()] {
            assert_eq!(historical_id(not_found(), &q).unwrap(), id);
        }
        assert!(historical_id(not_found(), &id.to_string()[..8]).is_err());
        assert!(historical_id(not_found(), &format!("urn:uuid:{id}")).is_err());
        // Any other resolver error is returned as it is.
        let list = gage::entry::CandidateList {
            query: "q".into(),
            candidates: vec![],
        };
        assert!(matches!(
            historical_id(ResolveError::Ambiguous(list), &id.to_string()),
            Err(ResolveError::Ambiguous(_))
        ));
    }
}
