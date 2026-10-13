# M3 — Entry format

[← M2](m2-identity-crypto.md) · [P2 plan](index.md) · next: [M4 — One-shot CRUD](m4-crud.md)

> **Recommended model: Sonnet.** Serialization and file placement on top
> of M2's proven crypto, with the YAML decisions already settled below.
> The one part that deserves care is the emitter's fidelity: a corrupted
> secret written without complaint is the outcome this layer must never
> produce.

## Goal

The entry layer, library only, no CLI verb yet: an `Entry` type, its YAML
form (gage-owned emitter, `saphyr` parser), `entries/<uuid>.age` written
atomically at `0600` and encrypted to every key in `.age-recipients`
through M2's wrapper, read back with an `Identity`, and listed without
decrypting.

## Depends on

- **M2**: the age wrapper, `Identity`, typed decrypt errors.
- **P1**: `recipients` reader, `atomicfile`, `uuid`.

## Design references

- Design doc "Entry format" (fixed metadata fields, the `value`/`fields`
  split), "On-disk layout" (opaque UUID filenames under `entries/`).
- Go open questions: the accepted risk on entry-level versioning (why
  unknown fields are preserved).
- [Q-YAML-CRATE](../open-questions.md#q-yaml-crate),
  [Q-COMPAT](../open-questions.md#q-compat) (Go's field names and shape,
  not Go's bytes).

## Go sources

- Go plan `gage-cli-design/m3-entry-format.md`.
- Parity ledger rows: §3 M3.
- Go sources at the pinned SHA: `internal/gage/entry.go` (all but
  `Insert`, `Remove`, `Resolve`, which are M4's), `internal/gage/field.go`.
- Test inventory: `internal/gage/entry_test.go` (33) and `field_test.go`
  (4): 37 rows.

## Decisions to make first

Recommended answers; confirm them when this plan is reviewed.

- **Crates** ([Q-YAML-CRATE](../open-questions.md#q-yaml-crate)): `saphyr`
  0.2 with `default-features = false` (the `encoding` feature pulls
  `encoding_rs`, which gage doesn't need), and `jiff` 0.2 with only `std`
  (no time-zone database). `deny.toml` gains Zlib (`foldhash`, through
  `hashlink`).
- **Parsing.** Load with `early_parse(false)`, so every scalar keeps its
  original text and style. gage's own string fields (`title`,
  `description`, `updated_by`, `value`, `fields` values) take that text as
  is: `title: 0x1F` is the string `0x1F`, as yaml.v3 decodes into a Go
  string. A plain `null`, `~` or empty scalar in one of those fields is the
  empty string (yaml.v3 leaves the zero value). A mapping or sequence where
  a string belongs, a `fields` value that isn't a scalar, a duplicate key,
  more than one document, or a top level that isn't a mapping is a parse
  error (Conflict). A leading BOM is stripped, which `saphyr` doesn't do.
  An empty document is an empty `Entry`, as yaml.v3.
- **Timestamps**: `Option<Timestamp>` wrapping `jiff::Timestamp`. Written
  as RFC 3339 UTC at second precision, unquoted
  (`created: 2026-01-14T10:32:00Z`); read from any RFC 3339 offset and
  normalized to UTC seconds. A missing `created`/`updated` is `None` (Go's
  zero time), which M4's `ls` renders as `-`.
- **Unknown keys** are kept as owned `saphyr` nodes with their original
  representation and style, sorted by key after the known ones on write
  (Go's rule), and re-emitted from that representation: a plain scalar
  stays plain, a quoted one is double-quoted, mappings and sequences
  recurse. So `x: 0x1F` survives as written, which Go (decoding to `int`)
  doesn't manage. Aliases come back expanded, as in Go.
- **The emitter** is Go's `marshalEntryNodes` ported: fixed field order
  (`title`, `description` if set, `created`, `updated`, `updated_by`,
  `value`, `fields` if non-empty, then unknown keys); a multi-line string
  that can survive a literal block (no leading newline, no tab or control
  character, no line ending in whitespace) is written as one with the
  right chomping indicator; other multi-line or control-character strings
  are double-quoted with explicit escapes; single-line strings are plain
  only when the parser reads them back unchanged, else double-quoted.
- **Verify, then fall back** (Go's `MarshalEntry`): re-parse the bytes and
  compare every gage-owned string; on any mismatch re-emit with every
  string double-quoted; if that also fails, refuse to write (Internal).
  Extension over Go: the comparison also covers unknown keys (structural
  equality of the parsed nodes), since the parser makes that cheap.
- **Entry ids** are `uuid::Uuid` v4; `entry_ids()` lists `entries/*.age`
  whose stem parses as a UUID, skipping subdirectories and strays, sorted,
  and returns nothing (not an error) when `entries/` doesn't exist.
- **API shape** (library): `Entry`, `Entry::field(name)`,
  `entry::{marshal, unmarshal}`, and on `Vault`: `write_entry(id, &Entry)`,
  `read_entry(id, &Identity)`, `entry_ids()`. Typed errors: `EntryNotFound`
  (NotFound), `NotARecipient` and `CorruptCiphertext` passed through from
  M2, `Parse` (Conflict), `Unfaithful` (Internal).

## Tasks

Tests first, at task granularity.

### 1. The `Entry` type and timestamps (6 rows)

- [x] `TestTimestampRoundTripsWithoutDriftOrPrecisionLoss`,
      `TestTimestampUnmarshalYAMLRejectsBadRFC3339`,
      `TestTimestampUnmarshalYAMLRejectsNonStringNode` (a mapping where a
      timestamp belongs).
- [x] `TestTimestampMarshalYAMLEmitsUnderlyingTime`, as "a timestamp is
      emitted as an unquoted RFC 3339 scalar". Go's version tests a
      `MarshalYAML` method, which has no Rust counterpart.
- [x] `TestEveryEntryFieldIsMarshaled`: an exhaustive destructuring of
      `Entry` (a new field stops it compiling) plus a check that each field
      reaches the output, in place of Go's reflection.
- [x] `TestNewEntryIDsAreValidUUIDv4AndUnique` (large N).

### 2. The emitter and parser (13 rows)

- [x] Shape: `TestEntryMarshalMatchesDesignDocShape` (the design doc's
      example, field for field), `TestEntryMarshalIsDeterministic`,
      `TestEntryEmptyDescriptionAndFieldsRoundTripConsistently` (absent,
      never `null` or `""`), `TestEntryMultiLineValueStaysReadable`
      (a literal block, not an escaped string).
- [x] Fidelity: `TestEntryValueRoundTripsYAMLHostileContent` (leading `-`,
      embedded newlines, `:`, trailing whitespace, non-ASCII, a leading
      newline, tabs, CRLF, control characters),
      `TestEntryEveryStringFieldRoundTripsHostileContent`,
      `TestEntryQuotedFallbackRoundTripsEverything`,
      `TestVerifyFaithfulReportsEachMismatchedField`.
- [x] Unknown keys: `TestEntryPreservesUnknownFields`,
      `TestExtraPreservesLargeAndNestedValues`,
      `TestUnknownValueNodeHandlesEveryClosedSetType` (null, bool, int,
      float, string, sequence, mapping), `TestUnknownValueNodeMapAndSeqRecurse`.
- [x] Parsing decisions above, as Rust-side tests: raw text for
      `0x1F`-style scalars in gage's fields, `null`/`~` as empty, a
      duplicate key refused, a second document refused, a BOM accepted.
- [x] Excused: `TestUnknownValueNodeDefaultFallsBackToLibraryEncoding`.
      Go's unknown values are `any`, so an exotic type can reach the
      emitter; Rust's are a closed node type and the match is exhaustive,
      so there is no default branch to test.

### 3. Fields — `field` (4 rows)

- [x] `TestEntryFieldExtractsNamedField`,
      `TestEntryFieldEmptyValueIsAFieldNotAMiss`,
      `TestEntryFieldOnEntryWithNoFieldsSaysSo`,
      `TestEntryFieldUnknownNameErrorsClearly` (typed errors; the messages
      are what P3's `show --field` and P6's `--field` print).

### 4. Entry files (12 rows)

- [x] `TestWriteEntryThenReadEntryRoundTrips`,
      `TestEncryptDecryptEntryRoundTripIsByteIdentical`,
      `TestEntryDecryptableByEveryRecipient`.
- [x] `TestWriteEntryFilePermissions` (`0600`; `entries/` `0700`; Unix
      only), `TestWriteEntryTightensAnExistingLooseEntriesDir`,
      `TestWriteEntryIsAtomic` (through `atomicfile`; an interrupted write
      leaves the old ciphertext or nothing).
- [x] `TestEntryFileRevealsNothingInPlaintext` (no title, value or field
      bytes in the file).
- [x] `TestWriteEntryRejectsARecipientItCannotEncryptTo` (a plugin
      recipient in `.age-recipients`), `TestWriteEntryWithoutARecipientsFileFails`.
- [x] `TestReadEntryUnknownIDFails` (NotFound),
      `TestReadEntryWithNonRecipientIdentityFailsWithTypedError`,
      `TestReadEntryOnCorruptCiphertextFails`.

### 5. Listing (2 rows)

- [x] `TestEntryIDsListsFilesInEntriesDir`,
      `TestEntryIDsSkipsSubdirectoriesAndStrayFiles`; plus "no `entries/`
      directory is an empty list".

### 6. Ledger

- [x] Parity ledger §3 M3 row updated.

## Implementation notes

Where the code differs from the recommended answers above (nothing here
changes behaviour Go has):

- **Unknown values are `entry::Extra`, a closed enum** (`Plain`, `Text`,
  `Seq`, `Map`), not owned `saphyr` nodes. It keeps what the plan wanted
  (a plain scalar stays plain, `0x1F` stays `0x1F`, a quoted one is
  written double-quoted, collections recurse, nested maps keep document
  order) and derives `Eq`, so `Entry` does too and "faithful" is one
  `==`. Tags are dropped and aliases expanded, as in Go. `saphyr`'s spans
  are not carried; P6 · M12's `cat` alignment re-parses the text it
  renders.
- **`saphyr-parser` is a direct dependency** (same 0.2 line): `saphyr`
  doesn't re-export `Parser`, and `early_parse(false)` needs a hand-built
  `YamlLoader`.
- **Keys are compared after loading**, because with `early_parse(false)`
  `a` and `"a"` are different keys to the loader: duplicates are refused
  on the key's text.
- **`verify_faithful` names mismatched fields, never their contents**
  (Go's message echoes `%q` of the secret).
- **`Entry`'s `Debug` redacts** the value, field values and unknown
  fields; `marshal` returns `Zeroizing<Vec<u8>>` and `read_entry` wipes
  the decrypted YAML.
- **`entry_ids` accepts only the canonical lower-case hyphenated
  spelling**, the one `entry_path` builds; Go's `uuid.Parse` also takes
  other spellings that it could then never read back.
- **A timestamp that is present but empty or null is a parse error**
  (Go's behaviour); an absent one is `None` and is not written.

## Definition of done

Every box above checked; all 37 rows ported or excused in this doc;
`make lint` and `make test` clean locally and on Linux, macOS and Windows
CI. Entries can be written and read back at the library level. No CLI
verb touches them yet.
