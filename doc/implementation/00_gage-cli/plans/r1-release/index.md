# R1 First release (0.1.0): implementation plan

[← divergence plan](../divergence/index.md) · Parent issue
[#149](https://github.com/distillerylabs/gage-rs/issues/149) · Roadmap
[#114](https://github.com/distillerylabs/gage-rs/issues/114)

The divergence phases (D1–D4) made gage-rs its own reference
implementation. R1 ships it:

- `0.1.0` on crates.io as `gage-core` (library) and `gage-cli` (binary
  `gage`)
- a GitHub Release with checksummed, attested archives for five native
  targets
- shell and PowerShell installers
- a Homebrew tap

R1 changes no on-disk bytes, no exit code, no error text and no help
output. The golden vault and the help snapshots prove it, unedited.

---

## Where things stand at the start of R1

Measured on `main` at `c58a909`, 2026-10-29.

| What | State | Where |
|---|---|---|
| workspace version | `0.0.0` | `Cargo.toml` `[workspace.package]` |
| publishable crates | none: `publish = false` at the workspace level | `Cargo.toml` |
| crates.io `gage` | **taken** by an unrelated project (warewithall2/gage, created 2026-10-18, v0.3.0) | crates.io |
| crates.io `gage-core`, `gage-cli`; Homebrew `gage` | free | crates.io, formulae.brew.sh |
| version stamping | Makefile only: `GAGE_VERSION` from `git describe --tags --exact-match`, else `dev`; `GAGE_COMMIT` from `git rev-parse` | `Makefile`, `crates/gage-cli/build.rs`, `src/version.rs` |
| what a non-Makefile build prints | `gage dev (commit unknown)` | `version::resolve_build_info` |
| release workflow, CHANGELOG, install docs | none | — |
| supported versions | "no releases yet; latest `main`" | `SECURITY.md` |
| open advisory ignore | `RUSTSEC-2026-0387` (crypto-bigint via age's p256/hpke, unused) | `deny.toml`, [#98](https://github.com/distillerylabs/gage-rs/issues/98) |
| tool versions today | cargo-dist 0.32.0, release-plz 0.4.2 | crates.io |

**Three findings that shape the milestones.**

- **The library can't be published as `gage`.** Renaming the *package*
  to `gage-core` while keeping `[lib] name = "gage"` leaves every
  `use gage::` unchanged. What changes is every place that names the
  package:
  - `cargo -p gage` in `scripts/lint-selftest.sh`, CLAUDE.md and
    CONTRIBUTING.md
  - `offenders(&metadata, "gage")` in
    `crates/gage/tests/no_frontend_deps.rs`
  - the `gage = { path = ... }` dependencies in `gage-cli` and
    `reset-local-state`
- **The build that ships isn't the Makefile build.** cargo-dist and
  `cargo install` both run cargo directly. So the version fallback has to
  produce the right string on its own, or every release prints `dev`.
- **`gage-cli` can't dry-run publish until `gage-core` is on the index.**
  Until then, CI can only `cargo package --list` it. The first real
  publish is ordered `gage-core` then `gage-cli`, and release-plz does
  that.

## Resolved decisions

From the planning Q&A (recorded on #149):

- **Channels:**
  - GitHub Releases binaries
  - crates.io
  - a Homebrew tap (`distillerylabs/homebrew-tap`)
  - shell and PowerShell installers
- **Crate names:**
  - `gage-core` (`[lib] name = "gage"`)
  - `gage-cli` (binary `gage`)
  - `reset-local-state` stays unpublished
- **Version `0.1.0`, pre-1.0.**
  - A 0.x minor release may break the CLI contract or the library API.
  - Format v2 is frozen and versioned on its own (`format_version`),
    independent of the crate version.
- **Tooling:** cargo-dist builds the binaries, installers and Homebrew
  formula on a `v*` tag. release-plz opens the version-bump and CHANGELOG
  PR and publishes to crates.io when it merges.
- **Native builds only,** as in CI: no `cross`, no cross-compiling.
- **Every GitHub Action stays SHA-pinned,** as the existing workflows
  are.
- **Provenance:** sha256 checksums plus GitHub artifact attestations on
  every archive.
- **One ADR:** ADR-0058, the release and versioning policy (R1a).
- **Out of scope for 0.1.0:** macOS notarization, Windows code signing,
  distro packages (deb/rpm/AUR/scoop/winget), reproducible-build
  verification.

## Milestones

| Milestone | Status | Model | Depends on |
|---|---|---|---|
| [R1a Publishable crates + versioning](r1a-publishable-crates.md) ([#150](https://github.com/distillerylabs/gage-rs/issues/150)) | `[x]` | Sonnet | parent branch |
| [R1b Release pipeline](r1b-release-pipeline.md) ([#151](https://github.com/distillerylabs/gage-rs/issues/151)) | `[ ]` | **Opus** | R1a |
| [R1c Docs, pre-flight, cut](r1c-docs-and-cut.md) ([#152](https://github.com/distillerylabs/gage-rs/issues/152)) | `[ ]` | Sonnet | R1b |

**Why Sonnet for R1a.**

- A package rename that the compiler and `cargo metadata` check.
- Manifest fields.
- A version fallback small enough to cover completely with unit tests.

**Why Opus for R1b.**

- It decides who and what can publish:
  - a crates.io token or trusted publishing
  - a token that can push tags and trigger workflows
  - a token that can write to the Homebrew tap
- It produces the provenance users verify against.
- A generated workflow that quietly drops SHA pinning, or runs with
  broader permissions than it needs, reaches every user's machine.

**Why Sonnet for R1c.**

- It documents a pipeline R1b already shipped.
- It runs a checklist.

## Acceptance

- Every milestone doc's test list is green.
- `git diff --stat main -- tests/fixtures` is empty. The help snapshots
  and `format_v2_*` tests are green and unedited.
- `gage --version` prints the same release string from a Makefile build,
  a cargo-dist build and `cargo install gage-cli`.
- `0.1.0` is published:
  - `gage-core` and `gage-cli` on crates.io
  - a GitHub Release with five attested archives and the installers
  - `brew install distillerylabs/tap/gage` working
- ADR-0058 is linked from `doc/decisions/README.md`.
- `make lint`, `make test`, `make qa` and `make audit` are clean.
- CI is green on Linux, macOS and Windows.

## Process notes

- **Branches stack:** `main` <- `149-first-release` (these docs) <- R1a
  <- R1b <- R1c. Each milestone branch is cut from the previous one when
  the user says to start it.
- **Decisions first.** Each milestone doc lists proposals under
  "Decisions to make first". Settle them in the doc, and get them
  reviewed, before writing code. Then move them to "Settled decisions".
- **Irreversible steps are the user's.** These are permanent or
  outward-facing, so a milestone prepares them and the user performs
  them:
  - publishing to crates.io (a version can be yanked but never
    replaced)
  - pushing a release tag
  - creating repo secrets and the tap repo
- **If context runs low mid-milestone,** write a `*-sessionhandoff.md`
  here.
