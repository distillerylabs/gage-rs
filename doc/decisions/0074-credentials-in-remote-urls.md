# ADR-0074: A remote URL may not carry credentials

- Status: Accepted
- Origin: Small hardening pass ([#220](https://github.com/distillerylabs/gage-rs/issues/220))

## Context

A remote spelled `https://user:secret@host/...` is stored in plaintext in the
global config and `.git/config`, and printed by `vault info`. gage has a
supported place for a credential: a per-host token file
([ADR-0020](0020-remote-auth.md)).

## Decision

`init --remote`, `clone` and `git set-remote` refuse a URL carrying a
password, or any username on `http(s)`. The check is in the library, in
`gitrepo::clone`, `gitrepo::set_remote` and the vault's create validation.

- **Any `http(s)` username counts.** Hosts accept a token in that position, so
  it can't be told from a name. An SSH username is a login name and stays.
- **A new remote must also parse.** `git set-remote` used to store anything.
- **What's already stored keeps working and is not rewritten.** It is hidden
  from `vault info` and from errors that quote a remote.

## Consequences

Someone who used such a URL moves to `gage auth login`, and a clean
`git set-remote` removes the old value from both files.
