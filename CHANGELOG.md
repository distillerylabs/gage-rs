# Changelog

All notable changes to gage are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as
[ADR-0058](doc/decisions/0058-release-and-versioning.md) defines it.

## [Unreleased]

## [0.1.0](https://github.com/distillerylabs/gage-rs/releases/tag/v0.1.0) - 2026-10-31

### Other

- install docs, library docs, release runbook
- publishable crates and one version string
- typed CLI dispatch
- test seams off production types
- group the library by concern, re-export the frontend types
- small Go-isms
- scope the Unix-only golden's import to its test
- error/frontend boundary
- VaultId, enums, Option config
- CLI source sweep
- library source sweep
- security ADR review and final grep
- docs archive, re-home, process
- retire the Go parity machinery
- format v2 spec and golden vault
- user docs final pass
- ledger closure, gosec mapping and whole-surface help test
- inventory guard and audit of the 95 unmatched rows
- Bump the cargo-dependencies group across 1 directory with 5 updates
- recovery enroll, recovery verify, hints, docs
- init gains the recovery key; recovery rotate
- the recovery key library
- second Opus review's missing tests and the prompted-path dedupe
- recipient pending, approve and deny, and the P7 docs
- Opus review of the lock, reset, divergence and prompter rows
- identity enroll and clone's offer
- Opus review of the passphrase-source rows
- add --script/--stdin, GAGE_PASSPHRASE and the P6 docs
- add -c/--clip and -q/--qr to show and generate
- add cat display, ls -H, and vault remove's orphan cleanup
- add gage log and gage history --decrypt
- reconcile tests against Go's move tests and mutation-check the rest
- cross-vault mv/cp, two-vault locks, and the P5 user docs
- trust cache, recipient verify --repair, and the root --yes
- recipient add/remove with all-or-nothing re-encryption
- identity add/list and recipient list/verify
- box SyncError's cause; wait for echo-off in the insert -m pty test
- interactive conflict resolution and the sync user docs
- sync wiring, pull/push/sync, clone, init --remote
- remote primitives, syncerr, remoteauth, gage auth
- M13 test review: make completion tests able to fail
- session Tab completion and command abbreviation
- metadata index, search/grep, reindex
- session mode (Session, REPL, picker, idle timeout, history)
- Gate the fake editor's scratch_mode to Unix
- second Opus review of the scratch-file tests
- query resolution, show/edit/rename/generate, insert -e
- tests from the test audit; no rollback on a failed commit
- one-shot CRUD (insert, cat, rm, ls)
- tests from the test audit
- identity, age wrapper, memory protection
- vault lifecycle and vault-id keying
- close gaps found in the test review
- git-crate spike, primitives, registry, help
- Cargo workspace, CI, and library purity lint
