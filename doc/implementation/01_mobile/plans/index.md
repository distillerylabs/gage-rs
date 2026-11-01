# Mobile: plan

Roadmap [#177](https://github.com/distillerylabs/gage-rs/issues/177) ·
follows the [divergence plan](../../00_gage-cli/plans/divergence/index.md)
(D1–D4, R1; closed)

gage-rs is the reference implementation of gage. This plan takes it to
native mobile apps, starting with iOS, as
[ADR-0060](../../../decisions/0060-mobile-bindings.md) set out:

- the bindings live here, as `crates/gage-ffi` (UniFFI, proc-macro mode)
- the apps live in their own repos (`gage-ios`, later `gage-android`)

This roadmap covers the library, the bindings and their build. App work
is tracked in the app's repo.

Status legend: `[ ]` not started, `[~]` in progress, `[x]` done.

---

## Why

The [spike](../../00_gage-cli/plans/mobile-spike/findings.md) showed
that the core runs on iOS unchanged. From Swift, `gage-ffi` opens,
unlocks and lists a vault inside an iOS 27 simulator. Three things still
stop an app from being started:

- **Creating and joining a vault are CLI orchestration.** `init` and
  `clone` create the identity, create or clone the vault, roll back on
  failure, and register it, all in `gage-cli`.
- **The iOS build is a hand-run recipe.** There is no XCFramework and no
  CI. OpenSSL is linked for libssh2, which can't work on iOS. The
  vendored C code must be built with the deployment target set.
- **The bindings expose only open, unlock and list.**

## Resolved decisions

From the planning Q&A (recorded on #177):

- **iOS first; Android later.**
- **The first phase targets join + read.** An app can create or join a
  vault, sync, and show or search entries. Writes come in a later phase.
- **iOS 18 is the minimum** (`IPHONEOS_DEPLOYMENT_TARGET=18.0`).
- **App work is tracked in the app's own repo.**

## What stays

Everything the divergence plan kept, and in particular:

- the library/CLI purity lint, and `uniffi*` on the library's dependency
  denylist
- user-visible CLI behavior: help, flags, exit codes, error text
- format v2, frozen
- test-first milestones; 3-OS CI

## Phases

Rolling: each phase's parent issue and sub-issues are filed when the
phase starts, so later phases benefit from what earlier ones learn.

| Phase | Milestones | Status | Model | Depends on |
|---|---|---|---|---|
| A1 Mobile foundations ([plan](a1-foundations/index.md), [#178](https://github.com/distillerylabs/gage-rs/issues/178)) | A1a library create/join operations · A1b iOS build + simulator CI · A1c bindings for join + read | `[~]` | **Opus** A1a/A1c; Sonnet A1b | — |
| A2 Writes | insert/edit/remove, conflict resolution and recipient-change prompts through the bindings | `[ ]` | TBD | A1 |
| A3 Android | NDK cross-compile, Kotlin AAR build, emulator CI | `[ ]` | TBD | A1 |

**Order.**

- **A1** unblocks an app that joins and reads.
- **A2** comes after the app has used A1's prompter and error shapes,
  so writes extend them rather than reshape them.
- **A3** waits until the bindings surface has settled on iOS. A2 and A3
  don't depend on each other.
