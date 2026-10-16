//! A single decrypted record: the fixed metadata fields plus the payload
//! (see "Entry format" in the design doc). This module is the type and its
//! YAML form; [`store`] is where entries meet the vault's files.
//!
//! The YAML is written by gage's own emitter ([`marshal`]) and read with
//! `saphyr`'s parser ([`unmarshal`]), because a password manager cannot
//! delegate "this string comes back byte for byte" to a library's style
//! heuristics: see `yaml` for the rules and the check behind them.

mod crud;
mod store;
mod yaml;

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use uuid::Uuid;

use crate::crypt;
use crate::exitcode::{Code, HasCode};
use crate::identity::Identity;

pub(crate) use crud::sort_list;
pub use crud::{
    Candidate, CandidateList, ListEntry, LockError, NewEntry, ResolveError, WRITE_LOCK_TIMEOUT,
    WriteError, resolve_query,
};
pub use yaml::{marshal, unmarshal};

/// The vault-relative directory every entry file lives under ("On-disk
/// layout").
pub const ENTRIES_DIR_NAME: &str = "entries";

/// The suffix on every entry file: `<uuid>.age`, and nothing else lives in
/// `entries/` by construction.
pub const ENTRY_FILE_EXT: &str = ".age";

/// A fresh, opaque UUIDv4 entry id: filenames carry no meaning of their own.
pub fn new_id() -> Uuid {
    Uuid::new_v4()
}

/// `created`/`updated`'s wire form: an RFC 3339 instant at second precision,
/// written in UTC and unquoted (`2026-01-14T10:32:00Z`, as the design doc
/// shows). Fractions of a second would not survive a human editing the file
/// in `$EDITOR`, so none are ever held: every constructor truncates.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(jiff::Timestamp);

impl Timestamp {
    /// `t` normalized to the wire's canonical form. Every timestamp gage
    /// itself produces goes through this (or [`FromStr`]), so created and
    /// updated are stamped consistently whatever clock produced `t`.
    pub fn new(t: jiff::Timestamp) -> Timestamp {
        let whole = jiff::Timestamp::from_second(t.as_second()).unwrap_or(t);
        Timestamp(whole)
    }

    /// Seconds since the Unix epoch, or `None` outside the representable
    /// range.
    pub fn from_unix(seconds: i64) -> Option<Timestamp> {
        jiff::Timestamp::from_second(seconds).ok().map(Timestamp)
    }

    /// The clock, as an entry stamp.
    pub fn now() -> Timestamp {
        Timestamp::new(jiff::Timestamp::now())
    }

    pub fn as_unix(self) -> i64 {
        self.0.as_second()
    }

    pub fn as_jiff(self) -> jiff::Timestamp {
        self.0
    }
}

impl From<jiff::Timestamp> for Timestamp {
    fn from(t: jiff::Timestamp) -> Timestamp {
        Timestamp::new(t)
    }
}

/// RFC 3339, UTC, `Z`: exactly what the emitter writes.
impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Timestamp({})", self.0)
    }
}

/// Not an RFC 3339 timestamp.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("parsing timestamp {input:?}: {reason}")]
pub struct TimestampParseError {
    input: String,
    reason: String,
}

/// Reads any RFC 3339 offset and normalizes to UTC seconds, the way
/// [`Timestamp::new`] does, so a value that round-trips compares equal.
impl FromStr for Timestamp {
    type Err = TimestampParseError;

    fn from_str(s: &str) -> Result<Timestamp, TimestampParseError> {
        s.parse::<jiff::Timestamp>()
            .map(Timestamp::new)
            .map_err(|e| TimestampParseError {
                input: s.to_string(),
                reason: e.to_string(),
            })
    }
}

/// A value under a key this build doesn't recognize. A future gage may add
/// a field; an older binary reading that entry keeps it rather than
/// silently dropping it on the next `edit` (the accepted risk on
/// entry-level versioning). It is the closed set of shapes a YAML value
/// takes, so the emitter's match is exhaustive.
///
/// Scalars keep the text they were written with: `0x1F` stays `0x1F`, where
/// Go (decoding to an integer) writes back `31`. Tags are dropped and
/// aliases come back expanded, as in Go.
#[derive(Clone, PartialEq, Eq)]
pub enum Extra {
    /// An unquoted scalar, whatever it would resolve to (null, bool, int,
    /// float or string): written back unquoted, so it resolves the same
    /// way. An empty one is YAML's null.
    Plain(String),
    /// A quoted or block scalar: always a string, written back quoted.
    Text(String),
    Seq(Vec<Extra>),
    /// In document order, keys unique.
    Map(Vec<(String, Extra)>),
}

impl fmt::Debug for Extra {
    /// Shape only: unknown fields may well hold secrets.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Extra::Plain(_) | Extra::Text(_) => f.write_str("<scalar>"),
            Extra::Seq(items) => write!(f, "<sequence of {}>", items.len()),
            Extra::Map(pairs) => write!(f, "<mapping of {}>", pairs.len()),
        }
    }
}

/// One vault secret, decrypted. See "Entry format" in the design doc.
///
/// [`marshal`] writes these field by field in the design doc's order;
/// `every_entry_field_is_marshaled` destructures the struct, so a field
/// added here that the emitter and parser don't know fails to compile.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Entry {
    /// What `ls`/`search` match against and display: required, and the
    /// closest thing to a "name" an entry has, though it only exists
    /// post-decrypt.
    pub title: String,
    /// Optional free text, also searchable. Omitted from the wire entirely
    /// when empty, never `description: null` or an explicit empty string.
    pub description: String,
    /// Set once, at insert time, and never touched again. `None` when the
    /// document had none (Go's zero time), and then not written.
    pub created: Option<Timestamp>,
    /// Rewritten on every edit, along with `updated_by`.
    pub updated: Option<Timestamp>,
    /// The local device's identity name, recording which already-known
    /// identity made the change.
    pub updated_by: String,
    /// The primary payload: a password, or the body of an unstructured note.
    pub value: String,
    /// Structured key/value metadata (`--field NAME` extracts from here).
    /// Omitted from the wire when empty. Written sorted by name.
    pub fields: BTreeMap<String, String>,
    /// Keys this build doesn't recognize, written after the known ones in
    /// sorted order (their original position isn't recoverable, so sorting
    /// is what makes the output deterministic).
    pub extra: BTreeMap<String, Extra>,
}

/// Title, description and timestamps are what a log line wants; the value,
/// field values and unknown fields are secrets and never appear.
impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry")
            .field("title", &self.title)
            .field("description", &self.description)
            .field("created", &self.created)
            .field("updated", &self.updated)
            .field("updated_by", &self.updated_by)
            .field("value", &"<redacted>")
            .field("fields", &self.fields.keys().collect::<Vec<_>>())
            .field("extra", &self.extra.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Entry {
    /// Applies the stamps gage owns to `self`, an edited copy of
    /// `original`: `created` is the original's (set once, at insert, and
    /// never touched again), `updated` is now, and `updated_by` is the
    /// editing device. Whatever the edited text said for the three is
    /// overruled: they record what happened, not what someone typed.
    pub fn restamp_after_edit(&mut self, original: &Entry, identity: &Identity) {
        self.created = original.created;
        self.updated = Some(Timestamp::now());
        self.updated_by = identity.device().to_string();
    }

    /// One entry from `fields`: what `show --field NAME` prints instead of
    /// `value`.
    ///
    /// It lives here rather than in a frontend because "which part of this
    /// entry did you mean" is a question about an entry, not a terminal: a
    /// GUI's field picker wants the same lookup and the same not-found
    /// answer. An unknown name is NotFound rather than Usage (the name
    /// isn't malformed, it just isn't in this entry), and the error lists
    /// the names that are there, never their values. An empty value is a
    /// field, not a miss.
    pub fn field(&self, name: &str) -> Result<&str, FieldError> {
        if let Some(v) = self.fields.get(name) {
            return Ok(v);
        }
        if self.fields.is_empty() {
            return Err(FieldError::NoFields {
                title: self.title.clone(),
                name: name.to_string(),
            });
        }
        Err(FieldError::Unknown {
            title: self.title.clone(),
            name: name.to_string(),
            available: self.fields.keys().cloned().collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FieldError {
    #[error("gage: entry {title:?} has no fields; --field {name} cannot be read from it")]
    NoFields { title: String, name: String },
    /// `available` is sorted, so the message doesn't reshuffle between runs.
    #[error("gage: entry {title:?} has no field {name:?}; it has: {}", .available.join(", "))]
    Unknown {
        title: String,
        name: String,
        available: Vec<String>,
    },
}

impl HasCode for FieldError {
    fn code(&self) -> Code {
        Code::NotFound
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// No file at `entries/<id>.age`.
    #[error("gage: no entry with that id in this vault: {id}")]
    EntryNotFound { id: Uuid },
    /// M2's typed errors, passed through unwrapped rather than swallowed:
    /// a caller tells "wrong key" ([`crypt::Error::NotARecipient`]) from
    /// "damaged file" ([`crypt::Error::CorruptCiphertext`]) from "no such
    /// entry" by variant.
    #[error(transparent)]
    Crypt(#[from] crypt::Error),
    /// The decrypted bytes are not an entry.
    #[error("gage: parsing entry: {0}")]
    Parse(String),
    /// The entry would not survive its own YAML round trip, even fully
    /// quoted, so nothing was written: a corrupted secret written without
    /// complaint is the one outcome this layer must never produce.
    #[error("gage: refusing to write an entry that does not survive its own YAML round trip: {0}")]
    Unfaithful(String),
    #[error("gage: reading .age-recipients: {0}")]
    Recipients(#[from] crate::recipients::Error),
    #[error("gage: {action} {path}: {source}")]
    Io {
        action: &'static str,
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("gage: writing entry {id}: {source}")]
    Write {
        id: Uuid,
        source: crate::atomicfile::Error,
    },
}

impl HasCode for Error {
    fn code(&self) -> Code {
        match self {
            Error::EntryNotFound { .. } => Code::NotFound,
            Error::Crypt(e) => e.code(),
            Error::Parse(_) => Code::Conflict,
            Error::Unfaithful(_)
            | Error::Recipients(_)
            | Error::Io { .. }
            | Error::Write { .. } => Code::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_with_fields(pairs: &[(&str, &str)]) -> Entry {
        Entry {
            title: "ProtonMail".into(),
            fields: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Entry::default()
        }
    }

    // Go: TestTimestampRoundTripsWithoutDriftOrPrecisionLoss
    #[test]
    fn timestamp_round_trips_without_drift_or_precision_loss() {
        // Sub-second precision and a non-UTC offset in, whole UTC seconds
        // out, and stable from then on.
        let t: Timestamp = "2026-01-14T11:32:00.987654321+01:00".parse().unwrap();
        assert_eq!(t.to_string(), "2026-01-14T10:32:00Z");
        let again: Timestamp = t.to_string().parse().unwrap();
        assert_eq!(again, t);
        assert_eq!(again.as_unix(), t.as_unix());
        // The constructor truncates a clock reading the same way.
        let precise = jiff::Timestamp::new(1_768_386_720, 999_999_999).unwrap();
        assert_eq!(Timestamp::new(precise), t);
    }

    // Go: TestTimestampUnmarshalYAMLRejectsBadRFC3339
    #[test]
    fn timestamp_rejects_bad_rfc3339() {
        for bad in [
            "",
            "yesterday",
            "2026-01-14",
            "2026-13-14T10:32:00Z",
            "1768386720",
        ] {
            let err = bad.parse::<Timestamp>().unwrap_err();
            assert!(
                err.to_string().contains(&format!("{bad:?}")),
                "{bad:?}: {err}"
            );
        }
    }

    // Go: TestTimestampMarshalYAMLEmitsUnderlyingTime, as "a timestamp is
    // emitted as an unquoted RFC 3339 scalar". Go's version tests a
    // MarshalYAML method, which has no Rust counterpart; the emitter is the
    // only writer.
    #[test]
    fn a_timestamp_is_emitted_as_an_unquoted_rfc3339_scalar() {
        let e = Entry {
            created: Timestamp::from_unix(1_768_386_720),
            ..Entry::default()
        };
        let text = String::from_utf8(marshal(&e).unwrap().to_vec()).unwrap();
        assert!(text.contains("\ncreated: 2026-01-14T10:32:00Z\n"), "{text}");
    }

    // Go: TestEveryEntryFieldIsMarshaled. The destructuring is the guard (a
    // new field stops this compiling until it is listed here); the body
    // checks each one reaches the output, in place of Go's reflection.
    #[test]
    fn every_entry_field_is_marshaled() {
        let e = Entry {
            title: "t-title".into(),
            description: "d-desc".into(),
            created: Timestamp::from_unix(1_000_000_000),
            updated: Timestamp::from_unix(1_100_000_000),
            updated_by: "u-device".into(),
            value: "v-value".into(),
            fields: [("f-name".to_string(), "f-val".to_string())].into(),
            extra: [("x-key".to_string(), Extra::Plain("x-val".into()))].into(),
        };
        let Entry {
            title,
            description,
            created,
            updated,
            updated_by,
            value,
            fields,
            extra,
        } = &e;
        let text = String::from_utf8(marshal(&e).unwrap().to_vec()).unwrap();
        for needle in [
            title.as_str(),
            description,
            &created.unwrap().to_string(),
            &updated.unwrap().to_string(),
            updated_by,
            value,
            "f-name",
            fields["f-name"].as_str(),
            "x-key",
            "x-val",
        ] {
            assert!(text.contains(needle), "{needle:?} missing from:\n{text}");
        }
        assert_eq!(extra.len(), 1);
        assert_eq!(unmarshal(text.as_bytes()).unwrap(), e);
    }

    // Go: TestNewEntryIDsAreValidUUIDv4AndUnique
    #[test]
    fn new_entry_ids_are_valid_uuid_v4_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..10_000 {
            let id = new_id();
            assert_eq!(id.get_version_num(), 4);
            assert_eq!(id.get_variant(), uuid::Variant::RFC4122);
            assert!(seen.insert(id), "duplicate id {id}");
        }
    }

    // Not a Go row (Go stamps in `gage edit` itself): the three stamps come
    // from the original, the clock and the identity, never the edit.
    #[test]
    fn restamp_after_edit_overrules_the_edited_stamps() {
        let original = Entry {
            created: Timestamp::from_unix(1_000_000_000),
            updated: Timestamp::from_unix(1_000_000_000),
            updated_by: "laptop-1".into(),
            ..Entry::default()
        };
        let mut edited = Entry {
            title: "edited".into(),
            created: Timestamp::from_unix(915_148_800),
            updated: Timestamp::from_unix(915_148_800),
            updated_by: "some-other-device".into(),
            ..Entry::default()
        };
        let before = Timestamp::now();
        edited.restamp_after_edit(
            &original,
            &Identity::for_tests(&age::x25519::Identity::generate()),
        );
        assert_eq!(edited.created, original.created);
        assert!(edited.updated.unwrap() >= before);
        assert_eq!(edited.updated_by, "test-device");
        assert_eq!(edited.title, "edited", "the content is the edit's");
    }

    // Go: TestEntryFieldExtractsNamedField
    #[test]
    fn field_extracts_the_named_field() {
        let e = entry_with_fields(&[("username", "me@proton.me"), ("totp", "JBSW")]);
        assert_eq!(e.field("username").unwrap(), "me@proton.me");
        assert_eq!(e.field("totp").unwrap(), "JBSW");
    }

    // Go: TestEntryFieldEmptyValueIsAFieldNotAMiss
    #[test]
    fn field_with_an_empty_value_is_a_field_not_a_miss() {
        let e = entry_with_fields(&[("pin", "")]);
        assert_eq!(e.field("pin").unwrap(), "");
    }

    // Go: TestEntryFieldOnEntryWithNoFieldsSaysSo
    #[test]
    fn field_on_an_entry_with_no_fields_says_so() {
        let err = entry_with_fields(&[]).field("username").unwrap_err();
        assert!(matches!(err, FieldError::NoFields { .. }), "{err}");
        assert_eq!(err.code(), Code::NotFound);
        assert_eq!(
            err.to_string(),
            "gage: entry \"ProtonMail\" has no fields; --field username cannot be read from it"
        );
    }

    // Go: TestEntryFieldUnknownNameErrorsClearly. Names listed sorted,
    // values never.
    #[test]
    fn field_with_an_unknown_name_lists_the_names_that_exist() {
        let e = entry_with_fields(&[("zeta", "SECRET-Z"), ("alpha", "SECRET-A")]);
        let err = e.field("username").unwrap_err();
        assert_eq!(err.code(), Code::NotFound);
        let msg = err.to_string();
        assert_eq!(
            msg,
            "gage: entry \"ProtonMail\" has no field \"username\"; it has: alpha, zeta"
        );
        assert!(!msg.contains("SECRET"), "{msg}");
    }

    #[test]
    fn debug_output_never_shows_secrets() {
        let e = Entry {
            value: "hunter2".into(),
            fields: [("pin".to_string(), "4321".to_string())].into(),
            extra: [("note".to_string(), Extra::Plain("sekrit".into()))].into(),
            ..Entry::default()
        };
        let shown = format!("{e:?}");
        for secret in ["hunter2", "4321", "sekrit"] {
            assert!(!shown.contains(secret), "{shown}");
        }
        assert!(shown.contains("pin") && shown.contains("note"), "{shown}");
    }
}
