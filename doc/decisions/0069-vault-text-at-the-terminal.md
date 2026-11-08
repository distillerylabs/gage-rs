# ADR-0069: Vault text reaches the terminal only as text

- Status: Accepted
- Origin: Escape control characters in vault text the CLI renders ([#214](https://github.com/distillerylabs/gage-rs/issues/214))

## Context

Everything in a vault is untrusted, and a terminal obeys the control
sequences it is sent. Enrollment already validated a device name before
showing it, because the name sits directly above a `[y/N]`. Other text gage
reads from a vault had no such check: `.age-recipients` lines, the
`config.toml` diff, titles, `updated_by`, conflicted paths, and library
errors quoting any of these. Validating each source can't cover the diff or
a title, which may legitimately hold almost anything.

## Decision

**`gage-cli` escapes where output leaves, not where text is read.**
`display.rs` holds the one escaping function. `out`, the prompter's `say`
and `prefixed` all go through it, so a new call site is covered without
anyone remembering to. New output goes through one of those three. It lives
in the CLI because a terminal is a CLI concern: a GUI's text widget doesn't
obey escape sequences.

**What is escaped.** C0 controls except tab and newline, DEL, C1 controls,
and the bidi overrides U+202A–202E and U+2066–2069, spelled as `{:?}` spells
them. Tab only lays text out, so it stays and `history` doesn't change for
values that hold one.

**A field can't forge a line.** Each element of `out` is one line, so a
newline in it is escaped too, and `ls | wc -l` still counts entries. Prompts
and errors are legitimately multi-line and keep their newlines, so a field
placed in one, such as a title in the ambiguous-match list, is escaped as a
line first. Escaping is idempotent, so a field that passes two sinks is
unchanged by the second.

**Secret output is the exception.** `show` and the other paths through
`secretout` are byte-exact, because they exist to be piped. `cat` writes
YAML, whose double-quoted form already escapes control characters.

**`recipients[].pubkey` is validated when `config.toml` loads**, beside the
device name, with `recipient add`'s validator. A line in `.age-recipients`
is deliberately not refused at load. The spec says a difference between the
two files is reported, and `recipient verify --repair` is how it is cleared,
so refusing the line would take away that exit. With every config key valid,
a line that isn't a key can never be on both sides, so it is always a
reported difference and never encrypted to.

This is not a format change. The spec already defined that field as an age
recipient, and every vault gage writes passes.
