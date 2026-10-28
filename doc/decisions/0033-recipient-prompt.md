# ADR-0033: Recipient-change confirmation and `--yes`

- Status: Accepted
- Origin: Q-RECIPIENT-PROMPT ([entry](../archive/rust-port/open-questions.md#q-recipient-prompt)); Q-CONFIRM-DEFAULT-YES ([entry](../archive/rust-port/open-questions.md#q-confirm-default-yes))

## Decision

`Prompter::confirm_recipient_change(&RecipientChangeWarning) -> Result<bool, PromptError>`
has a default returning `PromptError::NotInteractive`, like
`resolve_conflict` ([ADR-0021](0021-sync-conflict-resolution.md)). Questions
go through the `prompter` parameter every write already takes; `Identity`
carries no prompter.

- `--yes` is a hidden, global `gage-cli` root flag that wraps the prompter
  and answers **only** this method. It never answers `confirm`, so "remove
  this device's own key?" stays a real question in a scripted run.
- `--yes` is refused in an interactive session, on a session line and on the
  way in: it would silently approve every change for the session's life.
  Under `--script`/`--stdin` it is allowed on the command line and wraps the
  prompter for the whole run; a `--yes` typed on a script line adds nothing.
- The terminal prompter **refuses when nobody is at it**: it returns
  `NotInteractive` before drawing or reading anything, instead of spending
  the next piped line as the answer (`printf 'pass\nvalue' | gage insert x
  --value-stdin` would otherwise answer with the value). The outcome is the
  bare-Enter outcome: no write, exit 1.
- Whether the trust cache is regenerated is decided by the library from
  `verify_recipients`, so who answered cannot change it.
- `Prompter::confirm_default_yes` serves clone's "enroll now?" offer, the one
  default-yes question, which `confirm` (default no) cannot express. It has
  no default body, so every wrapper must implement and delegate it. The
  default belongs to the question's rendering, not to the absence of an
  answerer: a script is never asked, and never says yes.
- Re-selecting a held vault in a session does not warn.
