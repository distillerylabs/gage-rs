# ADR-0042: The expiry is in the filename

- Status: Accepted
- Origin: D-ENROLL-EXPIRY-IN-NAME ([design doc](../design/gage-cli-init-design.md))

## Decision

A pending request is named `<request-id>-<expires-epoch>.age`, with the
expiry in UTC epoch seconds in the clear. It is a housekeeping hint only
([ADR-0041](0041-enroll-ttl.md)).

## Context

Keeping the name a bare UUID meant displaying an expiry gage could not
compute (the real one is sealed) and walking git history per request to find
when it was introduced. With the epoch in the name, listing needs no
history walk and pruning is a parse and a comparison.

## Consequences

A small, deliberate exception to "filenames encode nothing". The existence
of pending requests already leaks, the commit already reveals roughly when,
and what is added is the TTL, which is nearly always the 24 h default.
