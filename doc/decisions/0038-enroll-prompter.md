# ADR-0038: The enrollment code is an argument

- Status: Accepted
- Origin: D-ENROLL-PROMPTER ([design doc](../design/gage-cli-init-design.md))

## Decision

The approver's code is an ordinary input parameter, not an interactive
decision. `OpenEnrollment(requests, codes)` is a pure function of what it is
given. `gage-cli` collects codes from `--code` flags or, if none, prompts
with the existing masked `Prompter::value`. A wrong code returns a typed
error, and the CLI decides whether to ask again.

## Context

A new `UnlockRequest` kind was rejected: an enrollment code is not an
unlock (it proves the requester knew an out-of-band secret, not that the
device holds a key), and an implementation that renders "Enter passphrase"
for any kind would show the *wrong prompt* silently, which is worse than a
compile-time break.
