# Releasing gage

For maintainers. The policy is [ADR-0058](decisions/0058-release-and-versioning.md)
(versions, what semver covers) and [ADR-0059](decisions/0059-release-pipeline.md)
(who and what can publish). The settled choices behind the workflows are in
[the R1b plan](implementation/00_gage-cli/plans/r1-release/r1b-release-pipeline.md).

The moving parts:

| Piece | Does | Where |
|---|---|---|
| release-plz | opens the release PR (version bump, CHANGELOG); on merge, publishes `gage-core` then `gage-cli` to crates.io and pushes the tag `v<version>` | `.github/workflows/release-plz.yml`, `release-plz.toml` |
| cargo-dist | on the tag, builds five archives, installers and the Homebrew formula; attests, smoke-tests, then creates the GitHub Release | `.github/workflows/release.yml`, `release-smoke.yml`, `dist-workspace.toml` |

`release.yml` is generated; never edit it by hand. See "Changing the release
workflow" below.

## A normal release

1. **The release PR.** Every push to `main` refreshes a PR from a
   `release-plz-*` branch that bumps both crates to the same version and
   adds a section to `CHANGELOG.md`. It needs no action until you want to
   release.
2. **Read it.**
   - The version is right for what changed. release-plz runs
     cargo-semver-checks on `gage-core` against the last published version,
     so an accidental API break can't ship as a patch; `gage-cli` is not
     checked (ADR-0058). Pre-1.0, a breaking change to
     the CLI or the library is a minor bump; a `format_version` change is
     always a minor (or, from 1.0, a major) and never a patch.
   - Edit `CHANGELOG.md` on the PR branch if the generated section isn't
     what users should read. For `0.1.0` the text to paste is in
     [the R1c plan](implementation/00_gage-cli/plans/r1-release/r1c-docs-and-cut.md#draft-changelog-010).
   - `git diff --stat main -- tests/fixtures` is empty unless this release
     is a format change.
3. **Checks.** The PR's CI is green, including the `release.yml` run, which
   builds and smoke-tests all five archives without publishing anything.
   Run `make lint`, `make test`, `make qa` and `make audit` locally on the
   PR's head if you want a second look.
4. **Merge** the PR. `release-plz.yml` sees a merged `release-plz-*` PR
   whose `v<version>` tag doesn't exist and starts its `release` job.
5. **Approve** the `release` environment when GitHub asks. This is the one
   human gate: nothing is published before it, and the tag (hence the
   GitHub Release) exists only after it.
6. **Watch.** `release-plz.yml` publishes the crates and pushes the tag;
   the tag starts `release.yml`: build, attest, smoke test, push the
   formula to `distillerylabs/homebrew-tap`, create the GitHub Release.
7. **Verify** each channel against the new version: `cargo install
   gage-cli --locked`, `brew install distillerylabs/tap/gage`, the shell
   and PowerShell installers, and one downloaded archive per OS checked with
   `sha256sum -c` and `gh attestation verify` (the README's
   [Verifying a download](../README.md#verifying-a-download)). Check that
   docs.rs built `gage-core`.

If `release.yml` fails after the crates are published, the version is
already on crates.io and cannot be re-published. Fix forward with a patch
release; yank if the published version is harmful (below).

## Setup and secrets

Nothing publishable is a repository secret, so no `pull_request` run can
read a credential.

- **GitHub App**, owned by `distillerylabs`, installed on `gage-rs` and
  `homebrew-tap` only. Permissions: Contents read and write, Pull requests
  read and write, Metadata read. Its client id and private key are stored as
  `RELEASE_APP_CLIENT_ID` and `RELEASE_APP_PRIVATE_KEY` in each of the three
  environments below. Each job mints a short-lived token for one repository
  with only the permissions that job needs.
- **Environments:**
  - `release`: you as required reviewer; deployment branches `main` only.
  - `release-pr`: no reviewer; `main` only.
  - `homebrew-tap`: no reviewer; deployment tags `v*` only.
- **Tag ruleset** on `refs/tags/v*`: creation, update and deletion
  restricted, with the App as the only bypass actor. This is what lets the
  `homebrew-tap` environment go without a reviewer: a `v*` tag can only come
  from the approved `release` job.
- **Branch ruleset on `main`:** pull requests only, and these required
  checks (all from GitHub Actions): `test (ubuntu-latest)`, `test
  (macos-latest)`, `test (windows-latest)`, `lint (ubuntu-latest)`, `lint
  (macos-latest)`, `lint (windows-latest)`, `msrv`, `audit`, `qa
  (ubuntu-latest)`, `qa (macos-latest)`, plus `release.yml`'s `plan`, the
  five `build-local-artifacts (<target>)` and the five
  `custom-release-smoke / smoke (<target>)`. Without the release checks, a
  release PR that breaks one target could merge and the crates would be
  published before the tag build fails. Add new required checks here when a
  workflow gains one.
- **crates.io:** trusted publishing for both crates (repository
  `distillerylabs/gage-rs`, workflow `release-plz.yml`, environment
  `release`).

### After the first publish

Only the first publish needs a token, because trusted publishing can be
configured only for a crate that exists. After `0.1.0` is on crates.io:

1. On crates.io, for both `gage-core` and `gage-cli`, add the trusted
   publisher above.
2. Delete the `CARGO_REGISTRY_TOKEN` secret from the `release` environment.
3. Revoke the token on crates.io.

With no `CARGO_REGISTRY_TOKEN` set, release-plz publishes by trusted
publishing. No long-lived crates.io credential should remain.

### Rotation

- **The App's private key:** generate a new key in the App's settings,
  update `RELEASE_APP_PRIVATE_KEY` in all three environments, run a harmless
  workflow that mints a token (a push to `main` runs `release-plz.yml`'s
  `release-pr` job), then delete the old key. Do it immediately if the key
  may have leaked, and also review the App's recent activity and the tag
  list.
- **The tap's access:** there is no separate tap token; it is the App's
  token scoped to `homebrew-tap`. To cut it off, remove `homebrew-tap` from
  the App's installation.
- **Reviewer or ruleset changes** are repository settings; the lists above
  are the intended state.

## Yank

A yank stops new resolutions of a version; it does not delete it. For a
release that must not be used:

1. `cargo yank --version X.Y.Z gage-cli` and `cargo yank --version X.Y.Z
   gage-core`. Yank both: the versions move in lockstep.
2. Edit the GitHub Release: mark it a pre-release or say in its notes that it
   is superseded, and link the fix. Don't delete it or its tag; the tag
   ruleset forbids deleting tags for a reason.
3. Revert the tap: in `distillerylabs/homebrew-tap`, restore `Formula/gage.rb`
   to the last good version (a normal PR or commit there).
4. Ship the fix as a patch release (below).

If the cause is a leaked credential rather than a bad build, rotate it first.

## Hotfix

Only the latest release is supported, so a hotfix normally releases from
`main`: merge the fix, then release-plz opens a patch PR; follow "A normal
release". If `main` holds changes that aren't releasable yet:

1. Branch from the tag: `git checkout -b hotfix-X.Y v<version>`.
2. Cherry-pick the fix.
3. Open a release-plz PR against that branch (a `release-plz-*` branch
   whose base is the hotfix branch), and merge it after the same checks.
   Note the `release-plz.yml` trigger is pushes to `main`, so a release from
   a non-`main` base needs that workflow and the environment's deployment
   rules extended first; if that is more than the fix is worth, merge the
   fix to `main` behind the unfinished work instead.

## What a format change means for a release

`format_version` versions the on-disk format independently of the crates
([ADR-0058](decisions/0058-release-and-versioning.md)). A change that needs
the `format_v2_*` tests edited is a format change: it needs a new spec, a new
fixture and an ADR in the same change ([doc/format/](format/README.md)).
Release it as a minor version (pre-1.0) or a major version, never a patch,
and say so at the top of its CHANGELOG section.

## Changing the release workflow

`release.yml` is cargo-dist's output plus one reviewed patch,
`scripts/dist/release.yml.patch`. `make dist-check` (run by `lint.yml`)
fails if the committed file isn't exactly that.

- Change the workflow: edit `release.yml`, then `make dist-patch`.
- Change `dist-workspace.toml`, or bump cargo-dist: `make dist-regen`, then
  re-review the patch. A bump also changes `CARGO_DIST_VERSION` in the
  Makefile, which must equal `cargo-dist-version`.
- Every `uses:` is SHA-pinned with a version comment; `make lint` rejects
  any that isn't.
