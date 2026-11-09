# Changelog

All notable changes to gage are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as
[ADR-0058](doc/decisions/0058-release-and-versioning.md) defines it.

## [Unreleased]

## [0.2.0](https://github.com/distillerylabs/gage-rs/compare/v0.1.1...v0.2.0) - 2026-11-09

### Other

- Undo init and clone when registering them fails ([#184](https://github.com/distillerylabs/gage-rs/pull/184))
- Rewrite the global config under its own lock ([#183](https://github.com/distillerylabs/gage-rs/pull/183))
- Report an unlock's prompt failure with the prompt's own kind ([#190](https://github.com/distillerylabs/gage-rs/pull/190))
- Harden script errors, the editor's environment, identity modes and remotes ([#220](https://github.com/distillerylabs/gage-rs/pull/220))
- Keep lines that may carry a value out of session history ([#218](https://github.com/distillerylabs/gage-rs/pull/218))
- Remove the edit scratch file when gage is ended by a signal ([#216](https://github.com/distillerylabs/gage-rs/pull/216))
- Escape control characters in vault text the CLI renders ([#214](https://github.com/distillerylabs/gage-rs/pull/214))
- Refuse a vault key file that doesn't continue the one last seen ([#207](https://github.com/distillerylabs/gage-rs/pull/207))
- Surface failed entry authentication at the CLI and FFI; document its limits ([#201](https://github.com/distillerylabs/gage-rs/pull/201))
- Rotate the MAC key on membership changes; re-stamp across a key change ([#200](https://github.com/distillerylabs/gage-rs/pull/200))
- Authenticate entries with a per-vault MAC key; format v3 ([#199](https://github.com/distillerylabs/gage-rs/pull/199))
- library create/join operations
- Gate the Unix-only test helpers for Windows lint ([#184](https://github.com/distillerylabs/gage-rs/pull/184))
- Restrict entry files to 0600 after every checkout ([#197](https://github.com/distillerylabs/gage-rs/pull/197))
- Box the vault-key error variants to keep errors small on Windows
- Scope the Device import to the Unix-only test
- Harden the sync merge writer and checkout against non-regular-file tree entries
- Bump the cargo-dependencies group with 3 updates
- bindings for join + read
- Merge branch '179-a1a-library-create-join' into 180-a1b-ios-build-ci
- fix Windows lint (unused import, large InitError)
- iOS build, no SSH on mobile, simulator smoke CI
- UniFFI bindings spike for iOS (open, unlock, list)

### Changed

- A passphrase prompt that breaks before any answer was tried (a terminal that
  can't be read, or piped input that isn't UTF-8) now exits `6` (`internal`)
  instead of `5` (`locked-or-auth`). A declined prompt and a wrong passphrase
  still exit `5`.

## [0.1.1](https://github.com/distillerylabs/gage-rs/compare/v0.1.0...v0.1.1) - 2026-10-31

### Other

- Reword gage-cli description; require a per-issue branch for all changes

### Changed

- Intel macOS (`x86_64-apple-darwin`) binaries are no longer published, and
  the Homebrew formula supports Apple silicon and Linux only. Use
  `cargo install gage-cli --locked` on Intel Macs.

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
