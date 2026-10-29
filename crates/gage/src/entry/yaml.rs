//! An entry's YAML form: gage's own emitter and `saphyr`'s parser
//! (ADR-0048).
//!
//! The emitter is gage's because no library's scalar-style heuristics can
//! be trusted with a password manager's contents: a typical YAML emitter writes a
//! block scalar that reads back one newline short for `"\nsecret"`, and one
//! it cannot re-read for a value with a tab, so the entry would encrypt
//! cleanly and never open again. So [`marshal`] owns the rule (see
//! [`Writer::string`]) and then checks it: it parses its own output and
//! compares everything, falls back to quoting every string if anything
//! differs, and refuses to write if even that differs.
//!
//! The parser is configured with `early_parse(false)`, so every scalar
//! keeps the text and style it was written with: gage's string fields take
//! that text as is (`title: 0x1F` is the string `0x1F`, as a typed decode
//! into a string would), and unknown fields keep theirs.

use std::collections::BTreeMap;

use saphyr::{ScalarStyle, YamlLoader, YamlOwned};
use saphyr_parser::Parser;
use zeroize::Zeroizing;

use super::{Entry, Error, Extra, Timestamp};

/// How far [`Writer`] goes to keep strings intact.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Quoting {
    /// Pleasant to edit by hand (`gage edit` shows this file to a human in
    /// `$EDITOR`): block scalars for multi-line values that can survive
    /// one, quotes only where they are needed.
    Readable,
    /// Every string double-quoted: the widest fidelity, the least readable.
    /// Only the fallback path uses it.
    Always,
}

/// Renders `e` as the YAML shown in "Entry format": `title`, `description`
/// if set, `created`, `updated` (each only if present), `updated_by`,
/// `value`, `fields` if any, then unknown keys, sorted. Fields are sorted
/// too, so one entry is always one byte sequence.
///
/// Refuses ([`Error::Unfaithful`]) rather than return text that does not
/// parse back to exactly `e`.
pub fn marshal(e: &Entry) -> Result<Zeroizing<Vec<u8>>, Error> {
    let readable = emit(e, Quoting::Readable);
    if verify_faithful(readable.as_bytes(), e).is_ok() {
        return Ok(Zeroizing::new(readable.as_bytes().to_vec()));
    }
    // The readable styling did not survive its own round trip. Quote every
    // string, the one style that round-trips everything tested.
    let quoted = emit(e, Quoting::Always);
    verify_faithful(quoted.as_bytes(), e).map_err(Error::Unfaithful)?;
    Ok(Zeroizing::new(quoted.as_bytes().to_vec()))
}

fn emit(e: &Entry, q: Quoting) -> Zeroizing<String> {
    let mut w = Writer {
        out: Zeroizing::new(String::new()),
        q,
    };
    w.string_pair(0, "title", &e.title);
    if !e.description.is_empty() {
        w.string_pair(0, "description", &e.description);
    }
    if let Some(t) = e.created {
        w.timestamp_pair("created", t);
    }
    if let Some(t) = e.updated {
        w.timestamp_pair("updated", t);
    }
    w.string_pair(0, "updated_by", &e.updated_by);
    w.string_pair(0, "value", &e.value);
    if !e.fields.is_empty() {
        w.key(0, "fields");
        w.out.push('\n');
        for (name, value) in &e.fields {
            w.string_pair(2, name, value);
        }
    }
    for (key, value) in &e.extra {
        w.extra_pair(0, key, value);
    }
    w.out
}

struct Writer {
    out: Zeroizing<String>,
    q: Quoting,
}

/// A string rendered for the position after `key:` or `-`.
enum Rendered {
    /// One line of text, to be followed by a newline.
    Inline(String),
    /// A literal block: it carries its own line breaks, final one included.
    Block(String),
}

impl Writer {
    fn indent(&mut self, n: usize) {
        for _ in 0..n {
            self.out.push(' ');
        }
    }

    fn key(&mut self, indent: usize, key: &str) {
        self.indent(indent);
        let text = match self.q {
            Quoting::Always => double_quoted(key),
            Quoting::Readable if plain_ok(key) => key.to_string(),
            Quoting::Readable => double_quoted(key),
        };
        self.out.push_str(&text);
        self.out.push(':');
    }

    fn rendered(&mut self, r: Rendered) {
        match r {
            Rendered::Inline(t) => {
                self.out.push(' ');
                self.out.push_str(&t);
                self.out.push('\n');
            }
            Rendered::Block(t) => {
                self.out.push(' ');
                self.out.push_str(&t);
            }
        }
    }

    fn string_pair(&mut self, indent: usize, key: &str, value: &str) {
        self.key(indent, key);
        let r = self.string(value, indent + 2);
        self.rendered(r);
    }

    /// `created: 2026-01-14T10:32:00Z`: an RFC 3339 scalar, unquoted, which
    /// is what the design doc shows. It is plain-safe by construction.
    fn timestamp_pair(&mut self, key: &str, t: Timestamp) {
        self.key(0, key);
        self.out.push(' ');
        self.out.push_str(&t.to_string());
        self.out.push('\n');
    }

    /// One string scalar, in the style that both round-trips and reads
    /// well.
    ///
    /// A multi-line value gets a literal block only when it can survive
    /// one: no leading newline (the block would read back one short), no
    /// tab or control character (not representable in a block), no line
    /// ending in whitespace (some readers strip it) and no leading space on
    /// the first line (it would be taken for the block's indentation).
    /// Everything else multi-line is double-quoted, which escapes those
    /// characters explicitly. A single line is plain only when the parser
    /// reads it back unchanged *and* nothing would resolve it to a
    /// non-string, else double-quoted.
    fn string(&self, s: &str, content_indent: usize) -> Rendered {
        match self.q {
            Quoting::Always => Rendered::Inline(double_quoted(s)),
            Quoting::Readable if block_safe(s) => Rendered::Block(literal_block(s, content_indent)),
            Quoting::Readable if plain_ok(s) => Rendered::Inline(s.to_string()),
            Quoting::Readable => Rendered::Inline(double_quoted(s)),
        }
    }

    /// An unknown field's string: a quoted or block scalar in the source,
    /// so it must stay one (plain would resolve it differently).
    fn text(&self, s: &str, content_indent: usize) -> Rendered {
        if self.q == Quoting::Readable && block_safe(s) {
            Rendered::Block(literal_block(s, content_indent))
        } else {
            Rendered::Inline(double_quoted(s))
        }
    }

    fn extra_pair(&mut self, indent: usize, key: &str, value: &Extra) {
        self.key(indent, key);
        self.extra_value(indent, value);
    }

    /// What follows `key:` or `-` at `indent`.
    fn extra_value(&mut self, indent: usize, value: &Extra) {
        match value {
            // Empty is YAML's null: nothing after the indicator.
            Extra::Plain(t) if t.is_empty() => self.out.push('\n'),
            Extra::Plain(t) => {
                self.out.push(' ');
                self.out.push_str(t);
                self.out.push('\n');
            }
            Extra::Text(s) => {
                let r = self.text(s, indent + 2);
                self.rendered(r);
            }
            Extra::Seq(items) if items.is_empty() => self.out.push_str(" []\n"),
            Extra::Map(pairs) if pairs.is_empty() => self.out.push_str(" {}\n"),
            Extra::Seq(items) => {
                self.out.push('\n');
                for item in items {
                    self.indent(indent + 2);
                    self.out.push('-');
                    self.extra_value(indent + 2, item);
                }
            }
            Extra::Map(pairs) => {
                self.out.push('\n');
                for (k, v) in pairs {
                    self.extra_pair(indent + 2, k, v);
                }
            }
        }
    }
}

/// A character that is never written raw: control characters (tab and
/// newline included), DEL, the C1 block, the Unicode line separators and
/// the byte-order mark.
fn exotic(c: char) -> bool {
    c < ' '
        || ('\u{7f}'..='\u{9f}').contains(&c)
        || matches!(c, '\u{2028}' | '\u{2029}' | '\u{feff}')
}

fn block_safe(s: &str) -> bool {
    if !s.contains('\n') || s.starts_with('\n') || s.starts_with(' ') {
        return false;
    }
    s.split('\n')
        .all(|line| !line.ends_with([' ', '\t']) && !line.chars().any(|c| exotic(c) && c != '\n'))
}

/// `s` (which passed [`block_safe`]) as a literal block scalar whose lines
/// sit `indent` columns in. The chomping indicator keeps exactly the
/// trailing line breaks `s` has: strip for none, clip for one, keep for
/// more.
fn literal_block(s: &str, indent: usize) -> String {
    let body = s.trim_end_matches('\n');
    let trailing = s.len() - body.len();
    let mut out = String::from(match trailing {
        0 => "|-",
        1 => "|",
        _ => "|+",
    });
    for line in body.split('\n') {
        out.push('\n');
        if !line.is_empty() {
            out.push_str(&" ".repeat(indent));
        }
        out.push_str(line);
    }
    out.push('\n');
    // keep: every break past the first is a blank line of its own.
    out.push_str(&"\n".repeat(trailing.saturating_sub(1)));
    out
}

fn double_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            '\u{2028}' => out.push_str("\\L"),
            '\u{2029}' => out.push_str("\\P"),
            '\u{feff}' => out.push_str("\\uFEFF"),
            c if exotic(c) => out.push_str(&format!("\\x{:02X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Whether `s` can be written unquoted and read back as the same string by
/// any YAML reader, not only this one: conservative on purpose, since a
/// quote costs nothing and a misread is a corrupted secret.
fn plain_ok(s: &str) -> bool {
    let Some(first) = s.chars().next() else {
        return false;
    };
    !(s.starts_with(' ')
        || s.ends_with(' ')
        || "-?:,[]{}#&*!|>'\"%@`".contains(first)
        || s.starts_with("...")
        || s.chars().any(exotic)
        || s.contains(": ")
        || s.contains(" #")
        || s.ends_with(':')
        || resolves_to_non_string(s))
}

/// Null, booleans (including YAML 1.1's spellings), and anything made only
/// of the characters a number or a date is made of.
fn resolves_to_non_string(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "null" | "~" | "true" | "false" | "yes" | "no" | "on" | "off" | "y" | "n" | ".inf" | ".nan"
    ) {
        return true;
    }
    s.chars().any(|c| c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_hexdigit() || "+-._:xXoOtTzZ ".contains(c))
}

/// Re-reads freshly marshaled bytes and reports which fields did not come
/// back unchanged. Names the fields, never their contents: the message
/// ends up in an error a user may paste into an issue.
pub(super) fn verify_faithful(data: &[u8], want: &Entry) -> Result<(), String> {
    let got = unmarshal(data).map_err(|e| format!("the entry did not parse back: {e}"))?;
    let mut changed: Vec<String> = Vec::new();
    let mut check = |name: &str, same: bool| {
        if !same {
            changed.push(name.to_string());
        }
    };
    check("title", got.title == want.title);
    check("description", got.description == want.description);
    check("created", got.created == want.created);
    check("updated", got.updated == want.updated);
    check("updated_by", got.updated_by == want.updated_by);
    check("value", got.value == want.value);
    for name in got.fields.keys().chain(want.fields.keys()) {
        check(
            &format!("field {name:?}"),
            got.fields.get(name) == want.fields.get(name),
        );
    }
    for key in got.extra.keys().chain(want.extra.keys()) {
        check(
            &format!("unknown field {key:?}"),
            got.extra.get(key) == want.extra.get(key),
        );
    }
    changed.sort();
    changed.dedup();
    if changed.is_empty() {
        Ok(())
    } else {
        Err(format!("changed on the round trip: {}", changed.join(", ")))
    }
}

/// Parses the YAML [`marshal`] produces, or a human-edited variant of it
/// (`gage edit`'s `$EDITOR` flow). Keys this build doesn't recognize land
/// in [`Entry::extra`] rather than causing an error or being dropped.
///
/// Refused as [`Error::Parse`]: a mapping or sequence where a string
/// belongs, a `fields` value that isn't a scalar, a duplicate key, more
/// than one document, and a top level that isn't a mapping. An empty
/// document is an empty entry. A leading byte-order mark is skipped.
pub fn unmarshal(data: &[u8]) -> Result<Entry, Error> {
    let text = std::str::from_utf8(data).map_err(|e| parse_error(format!("not UTF-8: {e}")))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let mut loader = YamlLoader::<YamlOwned>::default();
    loader.early_parse(false);
    Parser::new_from_str(text)
        .load(&mut loader, true)
        .map_err(|e| parse_error(e.to_string()))?;
    if let Some(e) = loader.error() {
        return Err(parse_error(e.to_string()));
    }
    let mut docs = loader.into_documents();
    if docs.len() > 1 {
        return Err(parse_error("more than one YAML document".into()));
    }
    match docs.pop() {
        None | Some(YamlOwned::BadValue) => Ok(Entry::default()),
        // `~` or `null` alone: yaml.v3 leaves the zero value.
        Some(YamlOwned::Representation(t, ScalarStyle::Plain, _)) if is_null(&t) => {
            Ok(Entry::default())
        }
        Some(YamlOwned::Mapping(m)) => entry_from(m.into_iter()),
        Some(_) => Err(parse_error("the top level is not a mapping".into())),
    }
}

fn parse_error(msg: String) -> Error {
    Error::Parse(msg)
}

fn is_null(text: &str) -> bool {
    matches!(text, "" | "~" | "null" | "Null" | "NULL")
}

fn entry_from(pairs: impl Iterator<Item = (YamlOwned, YamlOwned)>) -> Result<Entry, Error> {
    let mut e = Entry::default();
    let mut seen = std::collections::HashSet::new();
    for (key, value) in pairs {
        let key = key_text(key)?;
        if !seen.insert(key.clone()) {
            return Err(parse_error(format!("duplicate key {key:?}")));
        }
        match key.as_str() {
            "title" => e.title = string_of(value, "title")?,
            "description" => e.description = string_of(value, "description")?,
            "created" => e.created = Some(timestamp_of(value, "created")?),
            "updated" => e.updated = Some(timestamp_of(value, "updated")?),
            "updated_by" => e.updated_by = string_of(value, "updated_by")?,
            "value" => e.value = string_of(value, "value")?,
            "fields" => e.fields = fields_of(value)?,
            _ => {
                e.extra.insert(key, extra_of(value)?);
            }
        }
    }
    Ok(e)
}

fn key_text(key: YamlOwned) -> Result<String, Error> {
    match key {
        YamlOwned::Representation(text, _, _) => Ok(text),
        _ => Err(parse_error("a mapping key is not a scalar".into())),
    }
}

/// A gage-owned string field: the scalar's text, whatever it might
/// resolve to. A plain null or empty scalar is the empty string.
fn string_of(v: YamlOwned, what: &str) -> Result<String, Error> {
    match v {
        YamlOwned::Representation(t, ScalarStyle::Plain, _) if is_null(&t) => Ok(String::new()),
        YamlOwned::Representation(t, _, _) => Ok(t),
        _ => Err(parse_error(format!("{what} must be a string"))),
    }
}

fn timestamp_of(v: YamlOwned, what: &str) -> Result<Timestamp, Error> {
    match v {
        YamlOwned::Representation(t, _, _) => {
            t.parse().map_err(|e| parse_error(format!("{what}: {e}")))
        }
        _ => Err(parse_error(format!("{what} must be an RFC 3339 timestamp"))),
    }
}

fn fields_of(v: YamlOwned) -> Result<BTreeMap<String, String>, Error> {
    match v {
        YamlOwned::Representation(t, ScalarStyle::Plain, _) if is_null(&t) => Ok(BTreeMap::new()),
        YamlOwned::Mapping(m) => {
            let mut out = BTreeMap::new();
            for (k, v) in m {
                let name = key_text(k)?;
                let value = string_of(v, &format!("field {name:?}"))?;
                if out.insert(name.clone(), value).is_some() {
                    return Err(parse_error(format!("duplicate field {name:?}")));
                }
            }
            Ok(out)
        }
        _ => Err(parse_error("fields must be a mapping".into())),
    }
}

fn extra_of(v: YamlOwned) -> Result<Extra, Error> {
    match v {
        YamlOwned::Representation(t, ScalarStyle::Plain, _) => Ok(Extra::Plain(t)),
        YamlOwned::Representation(t, _, _) => Ok(Extra::Text(t)),
        YamlOwned::Sequence(items) => Ok(Extra::Seq(
            items.into_iter().map(extra_of).collect::<Result<_, _>>()?,
        )),
        YamlOwned::Mapping(m) => {
            let mut pairs: Vec<(String, Extra)> = Vec::new();
            for (k, v) in m {
                let key = key_text(k)?;
                if pairs.iter().any(|(seen, _)| *seen == key) {
                    return Err(parse_error(format!("duplicate key {key:?}")));
                }
                pairs.push((key, extra_of(v)?));
            }
            Ok(Extra::Map(pairs))
        }
        // A tag on a collection: the tag is dropped.
        YamlOwned::Tagged(_, inner) => extra_of(*inner),
        _ => Err(parse_error("unsupported YAML value".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_DOC: &str = "\
title: ProtonMail
description: Personal email, 2FA via authenticator app
created: 2026-01-14T10:32:00Z
updated: 2026-08-20T09:03:00Z
updated_by: yubikey-5c-nfc-1
value: correcthorsebatterystaple
fields:
  username: me@proton.me
  totp_seed: JBSWY3DPEHPK3PXP
";

    fn design_doc_entry() -> Entry {
        Entry {
            title: "ProtonMail".into(),
            description: "Personal email, 2FA via authenticator app".into(),
            created: "2026-01-14T10:32:00Z".parse().ok(),
            updated: "2026-08-20T09:03:00Z".parse().ok(),
            updated_by: "yubikey-5c-nfc-1".into(),
            value: "correcthorsebatterystaple".into(),
            fields: [
                ("username", "me@proton.me"),
                ("totp_seed", "JBSWY3DPEHPK3PXP"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
            extra: BTreeMap::new(),
        }
    }

    fn marshaled(e: &Entry) -> String {
        String::from_utf8(marshal(e).unwrap().to_vec()).unwrap()
    }

    fn only_value(value: &str) -> Entry {
        Entry {
            value: value.to_string(),
            ..Entry::default()
        }
    }

    /// Strings a YAML emitter has to get right: every one is a
    /// way a naive emitter has corrupted a stored secret.
    fn hostile() -> Vec<String> {
        [
            "- starts with a dash",
            "-",
            "? question",
            ": colon first",
            "key: value",
            "trailing colon:",
            "has # hash",
            "#comment",
            "line one\nline two",
            "line one\nline two\n",
            "line one\nline two\n\n\n",
            "\nleading newline",
            "\n",
            "\n\n",
            "a\n\nb",
            " leading space",
            "trailing space ",
            "multi\n  indented\nline",
            "  first line indented\nsecond",
            "trailing ws on a line  \nnext",
            "tab\there",
            "\ttab first",
            "tab at end of line\t\nnext",
            "crlf\r\nline",
            "lone\rcr",
            "nul\0byte",
            "bell\x07 and escape \x1b[0m",
            "del\x7f",
            "c1\u{85}control",
            "line\u{2028}separator",
            "para\u{2029}separator",
            "bom\u{feff}inside",
            "\u{feff}leading bom",
            "non-ascii: héllo wörld ✓ 日本語 🔐",
            "quote \" inside",
            "'single'",
            "\"double\"",
            "back\\slash",
            "null",
            "~",
            "true",
            "False",
            "yes",
            "123",
            "0x1F",
            "1e3",
            "2026-01-14",
            "2026-01-14T10:32:00Z",
            "",
            " ",
            "&anchor",
            "*alias",
            "!tag",
            "|",
            ">",
            "%directive",
            "@at",
            "`tick",
            "[flow]",
            "{flow: map}",
            "---",
            "...",
            "--- doc",
            "a: b\nc: d\n",
            "a literal \\n that is not a newline",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    // The design doc's own
    // example parses to the entry, and the entry writes back in the same
    // shape (fields sorted).
    #[test]
    fn entry_marshal_matches_design_doc_shape() {
        assert_eq!(unmarshal(KEY_DOC.as_bytes()).unwrap(), design_doc_entry());
        assert_eq!(
            marshaled(&design_doc_entry()),
            "\
title: ProtonMail
description: Personal email, 2FA via authenticator app
created: 2026-01-14T10:32:00Z
updated: 2026-08-20T09:03:00Z
updated_by: yubikey-5c-nfc-1
value: correcthorsebatterystaple
fields:
  totp_seed: JBSWY3DPEHPK3PXP
  username: me@proton.me
"
        );
    }

    #[test]
    fn entry_marshal_is_deterministic() {
        let mut e = design_doc_entry();
        e.extra.insert("zz".into(), Extra::Plain("1".into()));
        e.extra.insert("aa".into(), Extra::Plain("2".into()));
        let first = marshaled(&e);
        for _ in 0..20 {
            assert_eq!(marshaled(&e), first);
        }
        assert!(first.find("\naa: 2\n").unwrap() < first.find("\nzz: 1\n").unwrap());
    }

    // Absent
    // on the wire, never null or "".
    #[test]
    fn empty_description_and_fields_are_absent_and_round_trip() {
        let e = Entry {
            title: "t".into(),
            ..Entry::default()
        };
        let text = marshaled(&e);
        assert!(!text.contains("description"), "{text}");
        assert!(!text.contains("fields"), "{text}");
        assert!(!text.contains("null"), "{text}");
        assert_eq!(unmarshal(text.as_bytes()).unwrap(), e);
    }

    #[test]
    fn multi_line_value_stays_readable() {
        let text = marshaled(&only_value("first line\nsecond line\nthird"));
        assert!(
            text.contains("value: |-\n  first line\n  second line\n  third\n"),
            "{text}"
        );
        assert!(!text.contains("\\n"), "{text}");
        // The three chomping modes each keep exactly their trailing breaks.
        for (value, header) in [("a\nb", "|-"), ("a\nb\n", "|"), ("a\nb\n\n", "|+")] {
            let text = marshaled(&only_value(value));
            assert!(
                text.contains(&format!("value: {header}\n")),
                "{value:?}: {text}"
            );
            assert_eq!(unmarshal(text.as_bytes()).unwrap().value, value);
        }
    }

    #[test]
    fn value_round_trips_yaml_hostile_content() {
        for s in hostile() {
            let text = marshaled(&only_value(&s));
            assert_eq!(
                unmarshal(text.as_bytes()).unwrap().value,
                s,
                "{s:?} via\n{text}"
            );
        }
    }

    // The same
    // strings as every string the entry holds, keys of `fields` included.
    #[test]
    fn every_string_field_round_trips_hostile_content() {
        for s in hostile() {
            let e = Entry {
                title: s.clone(),
                description: s.clone(),
                updated_by: s.clone(),
                value: s.clone(),
                fields: [(s.clone(), s.clone())].into(),
                extra: [(
                    s.clone(),
                    Extra::Map(vec![(s.clone(), Extra::Seq(vec![Extra::Text(s.clone())]))]),
                )]
                .into(),
                ..Entry::default()
            };
            let text = marshaled(&e);
            // An empty description is absent, which reads back empty.
            assert_eq!(unmarshal(text.as_bytes()).unwrap(), e, "{s:?} via\n{text}");
        }
    }

    /// Beyond the curated list: thousands of strings assembled from the
    /// characters that trip emitters, from a fixed seed. Every one must
    /// marshal (not refuse) and read back exactly, in the readable style
    /// or, failing that, the quoted fallback.
    #[test]
    fn value_round_trips_generated_hostile_strings() {
        const ALPHABET: &[&str] = &[
            "a", "Z", "0", "9", " ", "  ", "\t", "\n", "\r", "\r\n", "-", "- ", ":", ": ", "#",
            " #", "'", "\"", "\\", "|", ">", "&", "*", "!", "%", "@", "`", "?", "[", "]", "{", "}",
            ",", ".", "...", "---", "~", "null", "true", "\0", "\x1b", "\x7f", "\u{85}",
            "\u{2028}", "\u{feff}", "é", "日", "🔐",
        ];
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move |n: usize| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as usize) % n
        };
        for _ in 0..3_000 {
            let len = next(12);
            let s: String = (0..len).map(|_| ALPHABET[next(ALPHABET.len())]).collect();
            let e = Entry {
                title: s.clone(),
                description: s.clone(),
                updated_by: s.clone(),
                value: s.clone(),
                fields: [(s.clone(), s.clone())].into(),
                ..Entry::default()
            };
            let bytes = marshal(&e).unwrap_or_else(|err| panic!("{s:?}: {err}"));
            assert_eq!(unmarshal(&bytes).unwrap(), e, "{s:?}");
        }
    }

    #[test]
    fn quoted_fallback_round_trips_everything() {
        for s in hostile() {
            let e = Entry {
                title: s.clone(),
                value: s.clone(),
                fields: [(s.clone(), s.clone())].into(),
                extra: [(s.clone(), Extra::Text(s.clone()))].into(),
                ..Entry::default()
            };
            let text = emit(&e, Quoting::Always);
            let text: &str = &text;
            assert!(text.starts_with("\"title\": \""), "{text}");
            assert_eq!(unmarshal(text.as_bytes()).unwrap(), e, "{s:?} via\n{text}");
        }
    }

    // By name, never by
    // content.
    #[test]
    fn verify_faithful_reports_each_mismatched_field() {
        let mut other = design_doc_entry();
        other.title = "Other".into();
        other.value = "SECRET-CHANGED".into();
        other.fields.insert("username".into(), "x".into());
        other.fields.remove("totp_seed");
        other.created = None;
        other.extra.insert("novel".into(), Extra::Plain("1".into()));
        let err = verify_faithful(KEY_DOC.as_bytes(), &other).unwrap_err();
        for name in [
            "title",
            "value",
            "created",
            "\"username\"",
            "\"totp_seed\"",
            "\"novel\"",
        ] {
            assert!(err.contains(name), "{name} missing from: {err}");
        }
        assert!(!err.contains("SECRET"), "{err}");
        assert!(verify_faithful(KEY_DOC.as_bytes(), &design_doc_entry()).is_ok());
        let err = verify_faithful(b"title: [", &design_doc_entry()).unwrap_err();
        assert!(err.contains("did not parse back"), "{err}");
    }

    #[test]
    fn marshal_refuses_what_cannot_survive_a_round_trip() {
        // A plain extra holding a newline can't be written back unquoted;
        // the fallback doesn't help, so nothing is produced.
        let e = Entry {
            extra: [("k".to_string(), Extra::Plain("a\nb: c".into()))].into(),
            ..Entry::default()
        };
        let err = marshal(&e).unwrap_err();
        assert!(matches!(err, Error::Unfaithful(_)), "{err}");
        assert_eq!(
            crate::exitcode::HasCode::code(&err),
            crate::exitcode::Code::Internal
        );
    }

    #[test]
    fn preserves_unknown_fields() {
        let text = "\
title: t
value: v
future_flag: true
future_count: 0x1F
future_note: \"quoted\"
future_list:
  - 1
  - two
";
        let e = unmarshal(text.as_bytes()).unwrap();
        assert_eq!(e.extra.len(), 4);
        let out = marshaled(&e);
        assert_eq!(unmarshal(out.as_bytes()).unwrap(), e);
        // Plain stays plain, quoted stays quoted, and 0x1F is not 31.
        assert!(out.contains("\nfuture_flag: true\n"), "{out}");
        assert!(out.contains("\nfuture_count: 0x1F\n"), "{out}");
        assert!(out.contains("\nfuture_note: \"quoted\"\n"), "{out}");
    }

    #[test]
    fn extra_preserves_large_and_nested_values() {
        let mut nested = Extra::Text("deep\nvalue".into());
        for depth in 0..12 {
            nested = if depth % 2 == 0 {
                Extra::Map(vec![
                    ("k".into(), nested),
                    ("n".into(), Extra::Plain("1".into())),
                ])
            } else {
                Extra::Seq(vec![nested, Extra::Plain(String::new())])
            };
        }
        let big = Extra::Seq(
            (0..5_000)
                .map(|i| Extra::Text(format!("item {i}\nx")))
                .collect(),
        );
        let e = Entry {
            extra: [("nested".to_string(), nested), ("big".to_string(), big)].into(),
            ..Entry::default()
        };
        let text = marshaled(&e);
        assert_eq!(unmarshal(text.as_bytes()).unwrap(), e);
    }

    // Rust's unknown
    // values are a closed enum, so this walks every variant through the
    // text a generic YAML decode would produce.
    #[test]
    fn unknown_values_of_every_kind_round_trip() {
        let text = "\
title: t
a_null: null
a_bare_null:
a_bool: true
an_int: 42
a_float: 1.5
a_string: \"str\"
a_seq: [1, two, \"3\"]
a_map: {k: v, \"q\": w}
an_empty_seq: []
an_empty_map: {}
";
        let e = unmarshal(text.as_bytes()).unwrap();
        assert_eq!(e.extra["a_null"], Extra::Plain("null".into()));
        assert_eq!(e.extra["a_bare_null"], Extra::Plain(String::new()));
        assert_eq!(e.extra["a_bool"], Extra::Plain("true".into()));
        assert_eq!(e.extra["an_int"], Extra::Plain("42".into()));
        assert_eq!(e.extra["a_float"], Extra::Plain("1.5".into()));
        assert_eq!(e.extra["a_string"], Extra::Text("str".into()));
        assert_eq!(
            e.extra["a_seq"],
            Extra::Seq(vec![
                Extra::Plain("1".into()),
                Extra::Plain("two".into()),
                Extra::Text("3".into())
            ])
        );
        assert_eq!(
            e.extra["a_map"],
            Extra::Map(vec![
                ("k".into(), Extra::Plain("v".into())),
                ("q".into(), Extra::Plain("w".into()))
            ])
        );
        assert_eq!(e.extra["an_empty_seq"], Extra::Seq(vec![]));
        assert_eq!(e.extra["an_empty_map"], Extra::Map(vec![]));
        assert_eq!(unmarshal(marshaled(&e).as_bytes()).unwrap(), e);
    }

    // Nested collections keep
    // document order and unquoted scalars inside them.
    #[test]
    fn unknown_map_and_seq_recurse() {
        let text = "title: t\nouter:\n  zeta: [a, {b: 1}]\n  alpha:\n    - - x\n      - y\n";
        let e = unmarshal(text.as_bytes()).unwrap();
        let Extra::Map(pairs) = &e.extra["outer"] else {
            panic!("not a map")
        };
        assert_eq!(pairs[0].0, "zeta");
        assert_eq!(pairs[1].0, "alpha");
        assert_eq!(unmarshal(marshaled(&e).as_bytes()).unwrap(), e);
    }

    // The parsing decisions in the plan, as Rust-side tests.
    #[test]
    fn gage_string_fields_take_the_raw_text_of_scalars() {
        let e =
            unmarshal(b"title: 0x1F\nvalue: 1e3\nupdated_by: true\ndescription: 007\n").unwrap();
        assert_eq!(e.title, "0x1F");
        assert_eq!(e.value, "1e3");
        assert_eq!(e.updated_by, "true");
        assert_eq!(e.description, "007");
        let e = unmarshal(b"fields:\n  pin: 0123\n  on: yes\n").unwrap();
        assert_eq!(e.fields["pin"], "0123");
        assert_eq!(e.fields["on"], "yes");
        // Quoted null is the string, not the empty string.
        let e = unmarshal(b"title: \"null\"\nvalue: 'null'\n").unwrap();
        assert_eq!((e.title.as_str(), e.value.as_str()), ("null", "null"));
    }

    #[test]
    fn null_and_empty_scalars_are_the_empty_string() {
        let e = unmarshal(
            b"title: null\nvalue: ~\nupdated_by:\ndescription: Null\nfields:\n  k:\n  j: ~\n",
        )
        .unwrap();
        assert_eq!(e.title, "");
        assert_eq!(e.value, "");
        assert_eq!(e.updated_by, "");
        assert_eq!(e.description, "");
        assert_eq!(e.fields["k"], "");
        assert_eq!(e.fields["j"], "");
        let e = unmarshal(b"fields:\ntitle: t\n").unwrap();
        assert!(e.fields.is_empty());
        let e = unmarshal(b"fields: {}\n").unwrap();
        assert!(e.fields.is_empty());
    }

    #[test]
    fn empty_documents_are_an_empty_entry() {
        for doc in ["", "\n", "---\n", "# only a comment\n", "~\n", "null\n"] {
            assert_eq!(
                unmarshal(doc.as_bytes()).unwrap(),
                Entry::default(),
                "{doc:?}"
            );
        }
    }

    #[test]
    fn a_duplicate_key_is_refused() {
        for doc in [
            "title: a\ntitle: b\n",
            "title: a\n\"title\": b\n",
            "fields:\n  k: 1\n  k: 2\n",
            "fields:\n  k: 1\n  \"k\": 2\n",
            "x:\n  k: 1\n  \"k\": 2\n",
            "x: 1\n\"x\": 2\n",
        ] {
            let err = unmarshal(doc.as_bytes()).unwrap_err();
            assert!(matches!(err, Error::Parse(_)), "{doc:?}: {err}");
            assert_eq!(
                crate::exitcode::HasCode::code(&err),
                crate::exitcode::Code::Conflict
            );
        }
    }

    #[test]
    fn a_second_document_is_refused() {
        let err = unmarshal(b"title: a\n---\ntitle: b\n").unwrap_err();
        assert!(matches!(err, Error::Parse(_)), "{err}");
    }

    #[test]
    fn a_byte_order_mark_is_accepted() {
        let mut doc = "\u{feff}".as_bytes().to_vec();
        doc.extend_from_slice(b"title: with bom\nvalue: v\n");
        let e = unmarshal(&doc).unwrap();
        assert_eq!((e.title.as_str(), e.value.as_str()), ("with bom", "v"));
    }

    // The `created:` rows put a mapping and a sequence where a timestamp
    // belongs.
    #[test]
    fn collections_where_strings_belong_are_refused() {
        for doc in [
            "title: [a, b]\n",
            "title:\n  nested: map\n",
            "value: [x]\n",
            "description: {a: b}\n",
            "updated_by:\n  - x\n",
            "fields: [a, b]\n",
            "fields: text\n",
            "fields:\n  k: [1]\n",
            "fields:\n  k: {a: b}\n",
            "created: {a: b}\n",
            "created: [2026-01-14T10:32:00Z]\n",
            "created: yesterday\n",
            "created:\n",
            "- a\n- b\n",
            "just a string\n",
            "[unclosed\n",
            "? [complex]\n: key\n",
        ] {
            let err = unmarshal(doc.as_bytes()).unwrap_err();
            assert!(matches!(err, Error::Parse(_)), "{doc:?}: {err}");
        }
        assert!(matches!(
            unmarshal(&[0xff, 0xfe, 0x00]),
            Err(Error::Parse(_))
        ));
    }

    #[test]
    fn timestamps_from_any_offset_normalize_to_utc_seconds() {
        let e =
            unmarshal(b"created: 2026-01-14T11:32:00.5+01:00\nupdated: \"2026-01-14T10:32:00Z\"\n")
                .unwrap();
        assert_eq!(e.created, e.updated);
        assert!(marshaled(&e).contains("created: 2026-01-14T10:32:00Z\n"));
    }

    #[test]
    fn missing_timestamps_are_none_and_stay_absent() {
        let e = unmarshal(b"title: t\n").unwrap();
        assert_eq!((e.created, e.updated), (None, None));
        let text = marshaled(&e);
        assert!(
            !text.contains("created") && !text.contains("updated:"),
            "{text}"
        );
    }

    #[test]
    fn aliases_in_unknown_fields_come_back_expanded() {
        let e = unmarshal(b"title: t\nbase: &b {k: v}\ncopy: *b\n").unwrap();
        assert_eq!(e.extra["base"], e.extra["copy"]);
        assert_eq!(unmarshal(marshaled(&e).as_bytes()).unwrap(), e);
    }

    #[test]
    fn plain_decisions_quote_anything_that_could_resolve_otherwise() {
        for s in [
            "null",
            "True",
            "yes",
            "123",
            "0x1F",
            "2026-01-14",
            "1e3",
            "-x",
            " x",
            "x ",
            "a: b",
            "a #b",
            "",
            "k:",
        ] {
            assert!(!plain_ok(s), "{s:?}");
        }
        for s in [
            "ProtonMail",
            "me@proton.me",
            "yubikey-5c-nfc-1",
            "JBSWY3DPEHPK3PXP",
            "héllo",
            "a,b",
            "1Password",
        ] {
            assert!(plain_ok(s), "{s:?}");
        }
    }
}
