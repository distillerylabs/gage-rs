# ADR-0072: A script error locates its line and doesn't repeat it

- Status: Accepted
- Origin: Small hardening pass ([#220](https://github.com/distillerylabs/gage-rs/issues/220))

## Context

`--script` and `--stdin` runs usually happen in CI, where stderr is kept in a
log. Their errors quoted the failing line, and a line can hold a value:
`recipient approve --code VALUE`, or `insert T --field pin=VALUE`. A line that
fails to parse is worse placed, since a parser complaint quotes tokens from
it, and an unquoted space turns part of a value into a token of its own.

## Decision

A script error reads `source:line: reason`, and never contains the line.

- **A line that doesn't parse gets a fixed message under `--script` and
  `--stdin`.** The tokenizer's and clap's complaints are replaced. The failing
  command's help stays, because it holds nothing from the line.
- **`--field` errors don't quote the flag, in any mode.** A flag with no `=`
  may be a value in the wrong place.
- **A typed session is unchanged.** The person who typed the line is reading
  the error.

## Consequences

A script author reads the line from the script by its number. Under `--stdin`
that means keeping the input.

A command's own error can still name what it was asked for, such as a query
in `no entry matches "X"`. Queries are titles, not values.
