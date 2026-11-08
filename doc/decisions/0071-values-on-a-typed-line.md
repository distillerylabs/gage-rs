# ADR-0071: Values on a typed line stay out of history

- Status: Accepted
- Origin: Keep --field values out of the session command history ([#218](https://github.com/distillerylabs/gage-rs/issues/218))
- Amends: [ADR-0050](0050-line-editor.md), whose "each submitted, non-blank line" this narrows

## Context

A session records the lines a human types. Values read by the prompter never
reach that record, but `insert --field NAME=VALUE` puts a value on the line
itself, so the line carries plaintext into the history file and the line
editor's recall list. The design doc says history must never contain it.

## Decision

A line containing `=` is not recorded, in the file or in recall. The session
otherwise runs it as usual.

- **The line is dropped, not masked.** A recalled line holding a placeholder
  would store the placeholder if someone pressed Enter on it.
- **The check reads the raw line.** The line is recorded before it runs, and
  one that fails to parse is still typed text: a misspelled flag or an
  unbalanced quote can carry a value that neither clap nor the tokenizer
  would ever hand back as a field.
- **`--field` stays available in a session.** It is the only way to set a
  field there without opening an editor.

## Consequences

A line with an `=` for another reason (`show a=b`, `--use=work`) can't be
recalled. That is the cheap side of the trade: a false positive costs one line
of recall, and a false negative writes a secret to disk.

A value given without `=` is not caught. `recipient approve --code VALUE`
remains the accepted risk [ADR-0039](0039-enroll-code-format.md) describes.

The history file stays append-only, so lines written before this change stay
in it until the file is deleted.
