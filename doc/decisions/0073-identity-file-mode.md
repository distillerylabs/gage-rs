# ADR-0073: An identity file others can read is refused

- Status: Accepted
- Origin: Small hardening pass ([#220](https://github.com/distillerylabs/gage-rs/issues/220))

## Context

gage writes an identity file at `0600`, but nothing checked it when reading.
Another local user who can read the file can guess its passphrase offline
for as long as they like. A stored token that others can read was already
refused ([ADR-0020](0020-remote-auth.md)).

## Decision

Opening an identity file refuses one with any group or other bits set, with a
typed error naming the path and the mode.

- **Refused, not warned about.** A warning scrolls past on every unlock while
  the exposure continues, and the fix is one `chmod`.
- **The mode comes from the open file**, so it describes the bytes read.
- **Windows is not checked.** It doesn't model these bits, as for token files.
- **Only opening is gated.** Taking a backup of the file to restore it isn't,
  since that gives the bytes to no one.

## Consequences

A file restored from a backup or copied with loose modes won't unlock until
its mode is tightened. The error says what it should be.
