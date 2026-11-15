# ADR-0079: A refusal of access keeps its remedy

- Status: Accepted
- Origin: [#227](https://github.com/distillerylabs/gage-rs/issues/227), found in gage-ios's M1 device run

## Context

A remote that refuses access is classified once, in `syncerr`, and every
fetch and push it refuses carries `Remedy::AuthLogin { host }` (or
`AuthLoginAgain`, or `UseHttps` for a token that would cross cleartext).
Operations that wrap a sync failure then replaced that remedy with their own:

- `EnrollmentError::Push` always said `EnrollAgain { auth_refused, host }`,
  so a read-only token that cloned fine and was refused at enroll's push
  reached the app as a different variant from the same refusal on a clone.
- A write's failed-push warning preferred the write's own clause remedy
  (`Push` for an approval), so a refused approval said to push again, which
  the same token is refused for again.

A frontend recognizes a refusal by its remedy, never by its message
([ADR-0054](0054-frontend-neutral-errors.md),
[ADR-0063](0063-what-crosses-the-bindings.md)), so each wrapper that shadowed
the remedy hid the refusal from every frontend at once.

## Decision

**A remedy that fixes remote access is never shadowed.** `Remedy` names
the access remedies (`fixes_remote_access`: `AuthLogin`, `AuthLoginAgain`,
`UseHttps`), and `Remedy::unless_access(cause, own)` is the one rule a
wrapper applies: the cause's remedy if it fixes access, otherwise the
wrapper's own, falling back to the cause's. Any error or warning that wraps
a sync failure and has advice of its own goes through it.

**`EnrollAgain` carries no data.** Its `auth_refused` and `host` fields
described the case that now says `AuthLogin`. An auth refusal with no host
(a local path, where no token exists) says to enroll again, and its note
says to check the path, rather than to store a token that can't help.

## Consequences

- A frontend handles "the remote refused this device" in one place, for
  every operation, by matching the access remedies.
- The CLI loses the follow-up step in two hints: an enroll refused for
  access no longer adds "then `gage identity enroll` again", and a refused
  approval no longer adds "run `gage push`". The messages still say the
  request was not published or the approval not pushed. A remedy is one
  action, and the one that has to come first is the token.
- `Remedy::EnrollAgain` changes shape in the bindings: apps that matched
  its fields match the bare case.
- A new wrapper that adds its own remedy to a sync failure uses
  `unless_access`, not `Option::or`.
