# ADR-0083: Apps build the bindings from a release tag; nothing prebuilt is published

- Status: Accepted
- Origin: [#191](https://github.com/distillerylabs/gage-rs/issues/191), from gage-ios's [ADR-0006](https://github.com/distillerylabs/gage-ios/blob/main/doc/decisions/0006-getting-the-bindings.md) and [gage-ios#16](https://github.com/distillerylabs/gage-ios/issues/16)

## Context

[ADR-0060](0060-mobile-bindings.md) says an app pins a released version of
the bindings, but leaves open what is released and how. `gage-ffi` is
`publish = false`, and crates.io wouldn't help anyway: an iOS app links
the build output, not a Rust crate. What this repo offers is
`make xcframework`, which writes `GageFFI.xcframework` and the generated
`gage_ffi.swift` under `target/ios/`.

gage-ios already consumes that. Its `scripts/build-bindings.sh` runs
`make xcframework` in a worktree at a pinned gage-rs ref and wraps the
output in a local `GageFFI` Swift package whose manifest lives in its own
repo. gage-ios#16 restricts that pin to release tags.

A prebuilt package would have to be built on a macOS runner, attached to a
GitHub release, and trusted like the release archives
([ADR-0059](0059-release-pipeline.md), [ADR-0068](0068-attest-on-tags-only.md)).
That is a second artifact with its own trust model, built before anyone
needs it.

## Decision

**A release tag is the released version of the bindings.** An app pins a
`vX.Y.Z` tag of this repo and builds `make xcframework` at it. Nothing
prebuilt is published: no XCFramework on the GitHub release, no Swift
package manifest in this repo, and `gage-ffi` stays `publish = false`.
The release pipeline gains no artifact.

**`make xcframework` and its output are the contract.** The target name,
the output paths under `target/ios/`, the framework and module names, and
the link requirements in the [gage-ffi README](../../crates/gage-ffi/README.md)
change only as a noted change in the release that makes them. How the
framework is built (cargo targets, uniffi-bindgen, slices, deployment
target) stays behind the target, as gage-ios's ADR-0006 expects.

**The bindings' API is versioned with the workspace.** It follows
[ADR-0058](0058-release-and-versioning.md) as `gage-core`'s public API
does: within 0.x a minor release may break it.

**The package manifest belongs to the app.** Wrapping the build output as
a Swift package (or, later, an Android library) is the app's job, in the
app's repo.

**A local override is the app's, and never its default.** Someone changing
both repos builds from their own checkout through whatever opt-in the app
provides. The app's CI builds only from the tag.

**Android follows the same rule when it comes.** It builds a
`make` target at a release tag. Whether that target exists yet is A3's
question.

## Consequences

- Every machine that builds an app needs the Rust toolchain this repo
  pins and the mobile targets, and compiles gage-rs from scratch on a
  fresh checkout. That cost lands on the app's CI.
- There is no attestation for the bindings. An app trusts the tag in this
  repo, as it would trust any source dependency.
- A change that breaks the app's build (a renamed output, a new linker
  requirement) is a breaking change and is noted in the changelog.
- Publishing a prebuilt package is a new ADR that supersedes this one.
  The likely reasons are app CI build time or what Android packaging
  needs. Tag-only pinning carries over to it.
