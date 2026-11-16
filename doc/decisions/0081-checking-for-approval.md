# ADR-0081: A device checks for its approval by its key on the recipient list

- Status: Accepted
- Origin: [#231](https://github.com/distillerylabs/gage-rs/issues/231), from gage-ios's Enrollment screen

## Context

A device that has asked to join waits for someone else to approve it. The
library had no way to ask whether that had happened. gage-ios checked by
syncing, unlocking and listing, and read `LockedOrAuth` from the list as "not
approved yet". So every check cost the person their passphrase and a second
of scrypt, and the answer was inferred from an error that has other causes.
The check's unlock also saved this device's first trust record before it was
a recipient, which [#228](https://github.com/distillerylabs/gage-rs/issues/228)
has since stopped.

What approval changes, from this device's side, is public: `recipient
approve` adds the device's key to `.gage/config.toml`. The key is in this
device's registration, recorded when its request was published. Neither
needs a passphrase to read.

## Decision

**`Vault::check_approved(prompter)` pulls, then answers whether the key this
device's registration records as its own is on the committed recipient
list.** It matches by key, never by device name, since names are labels
(ADR-0043). It unlocks nothing and asks nothing. The pull is
`pull_checked`'s, so `prompter` hears the recipient-change warning a pull
gives, and while this device is still waiting no trust record is saved
(#228). The bindings export it as `Vault.check_approved`.

**No recorded key is an error, not "waiting".** A device that never
published a request from here has nothing to wait for. It is
`ApprovalCheckError::NotEnrolled`, kind `Usage` with
`Remedy::EnrollOrRecover`, and it is found before the pull. A missing or
unreadable registration is an error too, not a "no".

**A failed pull is an error, not "waiting".** An unreachable remote is
`Unreachable`, so a frontend can say it couldn't check rather than say the
device is still waiting.

## Consequences

- The answer is the list's claim, not proof of access. Anyone who can push
  to the remote can add a key to `config.toml` without re-sealing the vault
  key or re-encrypting entries. The device would then read as approved and
  its first read would fail. That costs a confusing screen, not a secret:
  the first unlock and read is what shows the device can decrypt, and a
  frontend should treat a failure there as it would any other.
- A pull that finds a divergence still succeeds, and the check answers from
  the local branch. A waiting device has nothing of its own to diverge with
  once its request is pushed, so this is not expected in practice.
- An expired or denied request reads the same as one still waiting: its
  sealed file names no device or key, so the waiting device can't tell them
  apart without the approver. Asking again is `enroll`.
