# ADR-0076: An unlock's prompt failure keeps the prompt's kind

- Status: Accepted
- Origin: mobile bindings spike finding ([#190](https://github.com/distillerylabs/gage-rs/issues/190))

## Context

`UnlockError::Prompter` wraps the reason a prompter gave no answer, and it
reported `LockedOrAuth` whatever that reason was. `PromptError` already
draws the line: a person declining is `LockedOrAuth`, while a broken
terminal or a request this build can't render is `Internal`. The wrapper
hid that.

A broken terminal exited as an authentication failure. In the bindings, an
app whose dialog couldn't be shown got the same error as a person declining
to answer.

## Decision

`UnlockError::Prompter` has the kind of the `PromptError` it wraps, as other
wrapping errors do ([ADR-0054](0054-frontend-neutral-errors.md)).

`UnlockError::WrongPassphrase` stays `LockedOrAuth` even when the prompt that
ended the retries broke. A wrong passphrase was entered, so authentication
did fail, and the error says so.

## Consequences

- A passphrase prompt that breaks before any answer was tried exits `6`
  (`internal`) instead of `5` (`locked-or-auth`). Exit codes are covered by
  semver ([ADR-0058](0058-release-and-versioning.md)), so this ships in a
  minor release.
- End of input and Ctrl-C are `Cancelled`, and a run with no passphrase
  source is `Refused`. Both stay `5`, so a script that pipes nothing to
  `gage` sees no change.
- `Failed` from an app's prompter reaches it as `GageError.Internal`, unless a
  wrong passphrase came first.
- This covers unlock only. Commands that ask for a value (a recovery key, a
  token, an enrollment code) choose their own exit code for a failed prompt.
