# ADR-0016: libgit2 ignores the user's git config

- Status: Accepted
- Origin: Q-GIT-ISOLATION ([entry](../archive/rust-port/open-questions.md#q-git-isolation)); Q-GIT-CRATE

## Context

libgit2 honors `url.<base>.insteadOf` from the user's global config; a
hostile rewrite redirected a fetch to a decoy repository on all three OSes.
gage does not honor `~/.ssh/config`, credential helpers or `insteadOf`.

## Decision

One `#[allow(unsafe_code)]` `std::sync::Once` in `gitrepo` points libgit2's
System, XDG, Global and ProgramData search paths at an empty string (so each
level has no directory to search) before every repository open, including
those in `gittest`. The same `Once` sets the server timeouts
([ADR-0018](0018-remote-timeout.md)). Repositories are opened with
`Repository::open`, never "from env", so `GIT_CONFIG_*` is never read.

## Consequences

A child-process test with a hostile global config (`insteadOf` to a decoy
bare repo, `core.autocrlf=true`) first asserts libgit2 can see it, then
asserts neither has any effect. Removing the isolation turns it red.
