# R1b: Release pipeline (cargo-dist + release-plz)

[R1 plan](index.md) · [#151](https://github.com/distillerylabs/gage-rs/issues/151) · prev: [R1a](r1a-publishable-crates.md) · next: [R1c](r1c-docs-and-cut.md)

> **Recommended model: Opus.** This milestone decides who and what can
> publish gage:
>
> - the crates.io credential
> - the token that pushes the release tag
> - the token that writes the Homebrew tap
>
> It also produces the provenance users verify a download against.
> Generated workflows are easy to accept wholesale, but each of these
> regresses a property the repo already holds, and none of them fails a
> build:
>
> - a tag-pinned action
> - `permissions: write-all`
> - a credential an unreviewed PR can reach

## Goal

Merging a release-plz PR does all of this, with no other action:

- publishes `gage-core` then `gage-cli` to crates.io
- pushes one `v<version>` tag
- the tag triggers a cargo-dist workflow that builds five native
  archives, smoke-tests each one, checksums and attests them, publishes
  the GitHub Release with shell and PowerShell installers, and updates
  the Homebrew tap

Every action is SHA-pinned. Every job has least-privilege `permissions`,
and the publish credentials sit behind a GitHub environment that
requires the maintainer's approval.

## Depends on

R1a: packages are named and publishable, and `--version` is right
without the Makefile.

## Design references

- [R1 index](index.md), [ADR-0058](../../../../decisions/README.md)
  (written in R1a).
- The existing workflows (`.github/workflows/{test,lint,audit,qa}.yml`):
  - SHA-pinned actions with a `# vX.Y.Z` comment
  - top-level `permissions: contents: read`
  - native runners only
- The Makefile's pinned-tool pattern (`CARGO_DENY_VERSION`, installed into
  `./bin`, never resolved from `PATH`).
- cargo-dist 0.32.0 and release-plz 0.4.2, the current versions on
  2026-10-29. Check each config key below against the docs of the pinned
  version before using it: both tools change config between minors.

## Where things are

- No release workflow, no `dist-workspace.toml`, no `release-plz.toml`,
  no `CHANGELOG.md`.
- Workflows trigger on `push: main`, `pull_request` and
  `workflow_dispatch`. The audit workflow also runs on a weekly cron.
  None triggers on tags.
- No repo secrets beyond `CODECOV_TOKEN`. No `distillerylabs/homebrew-tap`
  repo.
- After R1a, `reset-local-state` is the only other binary in the
  workspace, and it must not be distributed.

## Decisions to make first

All eight were settled in review on 2026-10-30, after checking each
proposal against the cargo-dist 0.32.0 and release-plz 0.4.2 sources.
The original proposals are in this file's git history; the outcomes are
below.

## Settled decisions

What the sources showed, which changed several proposals:

- `dist generate` always writes a top-level `permissions: contents:
  write`. `github-custom-job-permissions` only reaches custom jobs, so
  `plan`, `build-global-artifacts`, `host` and `announce` can't be
  narrowed by config.
- Its jobs can't carry `environment:`, so the built-in Homebrew job can't
  read an environment secret.
- `github-build-setup` runs *before* the build; 0.32 has no post-build
  hook.
- `github-action-commits` covers only dist's six built-in actions. The
  value is pasted verbatim after `@`.
- `dist plan`, `build` and `host` refuse to run when `release.yml`
  differs from what dist would generate, unless `allow-dirty` lists
  `ci`.
- release-plz uses crates.io trusted publishing by itself whenever no
  registry token is set.
- `release-plz/action` installs release-plz with cargo-binstall and pulls
  in two more actions, inside the job that holds the publish credentials.
- `main` has no branch protection and no rulesets today.

1. **Targets and runners.** The five targets, on cargo-dist's own
   defaults, which are pinned images rather than `-latest`:

   | Target | Runner |
   |---|---|
   | `x86_64-unknown-linux-gnu` | `ubuntu-22.04` |
   | `aarch64-unknown-linux-gnu` | `ubuntu-22.04-arm` |
   | `aarch64-apple-darwin` | `macos-14` |
   | `x86_64-apple-darwin` | `macos-15-intel` |
   | `x86_64-pc-windows-msvc` | `windows-2022` |

   No `github-custom-runners`. The glibc baseline is 2.35. No musl.
   Intel macOS goes, with a CHANGELOG note, when its runner goes.
2. **Pinning.** `release.yml` is generated, then a committed patch is
   applied (see 3).
   - `github-action-commits` maps each dist action to `"<sha> #
     vX.Y.Z"`, so dist itself emits a pinned `uses:` with its version
     comment. The SHAs are the ones the other workflows already use.
   - `scripts/check-action-pins.sh` runs in `make lint`. It fails on any
     `uses:` in `.github/workflows/` that is not `owner/repo@<40 hex> #
     v...`, or a local `./` path. Its self-test runs in `make
     lint-selftest`.
   - release-plz runs from the Makefile-pinned binary, built from crates.io
     source with `--locked`, not from `release-plz/action`.
3. **Generated workflow plus a reviewed patch.**
   - `allow-dirty = ["ci"]`.
   - `scripts/dist/release.yml.patch` holds every hand edit, as a
     unified diff over dist's output.
   - `make dist-check` copies the working tree to a scratch directory,
     drops `allow-dirty`, runs the pinned `dist generate --mode ci`,
     applies the patch and requires the result to equal the committed
     `release.yml`.
   - `lint.yml` runs it on ubuntu.
   - A dist bump regenerates and refreshes the patch, in review.
4. **Smoke gating.**
   - `global-artifacts-jobs = ["./release-smoke"]`, a reusable workflow
     with its own five-runner matrix.
   - Each leg unpacks its target's archive and requires `gage --version`
     to print exactly `gage <version> (commit <sha7>)`.
   - `host` needs it, and both the Homebrew publish and `announce` need
     `host`. So a failed smoke blocks the release and the tap with no
     `needs:` edit.
   - It runs on PR runs too.
   - `github-release = "announce"`.
5. **One tag, one changelog,** as proposed.
   - `gage-cli` is tagged `v{{ version }}`; `gage-core` gets no tag.
   - `git_release_enable = false` for both crates.
   - One root `CHANGELOG.md`, written from `gage-cli`'s entry, with
     `gage-core`'s commits included.
   - `release_always = false`: a release happens only when a release-plz
     PR merges.
   - `semver_check = false`: there is no published baseline to compare
     against yet. Revisit after 0.1.0.
6. **Credentials and approvals: one human approval per release.**
   - **A GitHub App** owned by `distillerylabs`, installed on `gage-rs`
     and `homebrew-tap` only. Each job mints a token scoped to one repo
     and only the permissions it needs.
   - **Environment `release`:** a required reviewer (the maintainer),
     `main` only. It holds the App's client id and key, plus the
     first-publish `CARGO_REGISTRY_TOKEN`, deleted after 0.1.0 when
     trusted publishing takes over.
   - **Environment `release-pr`:** no reviewer, `main` only. It holds the
     App's client id and key for the release-PR job.
   - **Environment `homebrew-tap`:** no reviewer, `v*` tags only. It
     holds the App's client id and key, for the tap job.
   - **A tag ruleset** lets only the App create, move or delete `v*`
     tags. A tag can therefore only come from the approved `release`
     job, and that is what the tap job's ungated environment relies on.
   - **A pre-flight job, without an environment,** runs the `release`
     job only when the pushed commit came from a merged `release-plz-*`
     PR (release-plz's own test) and the tag `v<version>` doesn't exist
     yet. An ordinary push to `main` asks for no approval.
   - **No credential is a repo secret,** so no `pull_request` run can
     read one.
   - **The patch** sets the top-level `permissions: contents: read`, and
     each job's minimum: `contents: write` on `announce` only. Attesting
     happens in the build jobs, which dist already gives `id-token` and
     `attestations` write.
7. **Gating on CI.** `main` is unprotected today, so the gate doesn't
   exist yet.
   - The maintainer adds a ruleset on `main`: PRs only, and these checks
     must pass: `test (ubuntu-latest)`, `test (macos-latest)`, `test
     (windows-latest)`, `lint (ubuntu-latest)`, `lint (macos-latest)`,
     `lint (windows-latest)`, `msrv`, `audit`, `qa (ubuntu-latest)`,
     `qa (macos-latest)`.
   - It is listed in task 4; R1c's runbook records it.
8. **PR runs.** `pr-run-mode = "upload"` on every PR, with no path
   filter. test.yml doesn't build aarch64 Linux or Intel macOS, and
   this does. PR runs reach no credential.

## Tasks

### 1. Guards first

- [x] Tests first: write `scripts/check-action-pins.sh` (decision 2) and a
      self-test, as `lint-selftest` does:
      - a fixture workflow with a tag-pinned `uses:` must fail
      - the current workflows must pass

      Wire both into `make lint` and `make lint-selftest`.
      - `scripts/check-action-pins-selftest.sh` has 16 cases plus the
        repo's own workflows. Its fixtures are inline, because
        `tests/fixtures` stays untouched.
      - `make lint` runs the check as `action-pins`.
- [x] Pin `cargo-dist` and `release-plz` in the Makefile, installed into
      `./bin`, as with cargo-deny: `CARGO_DIST_VERSION`,
      `RELEASE_PLZ_VERSION`, and `make dist-bin` / `release-plz-bin`,
      which print each tool's path for CI.

### 2. cargo-dist

- [x] `dist init` writes `dist-workspace.toml` and `release.yml` with:
      - decision 1's targets and runners
      - installers `shell`, `powershell`, `homebrew` (tap
        `distillerylabs/homebrew-tap`, formula `gage`)
      - `github-attestations = true`
      - sha256 checksums
      - `pr-run-mode` per decision 8
      - `github-release` per decision 4

      Set `dist = false` on `gage-core` and `reset-local-state`, so only
      `gage-cli` is distributed.
      - The config is written by hand rather than by `dist init`, which is
        interactive.
      - `[profile.dist]` (release with thin LTO, dist's default) is added
        to the workspace `Cargo.toml`.
      - `install-updater = false`.
- [x] Archives include `LICENSE`, `README.md` and `CHANGELOG.md`. All three
      are dist auto-includes: `gage-cli`'s own README and LICENSE, and
      the workspace changelog. `dist plan` lists exactly these three.
- [x] Apply decision 2 (pinning) and decision 4 (smoke gating). Give every
      job explicit, minimal `permissions`: `id-token: write` and
      `attestations: write` only where attesting, and `contents: write`
      only where the release is created.
      - The patch (`scripts/dist/release.yml.patch`) does four things:
        - top-level `contents: read`
        - `contents: write` on `announce` only
        - the tag trigger narrowed to `v*`
        - the tap job moved onto `environment: homebrew-tap` with an App
          token and the App's commit identity
      - It also drops `secrets: inherit` from the smoke call.
      - The build jobs' attest permissions are dist's own.
- [x] Add `make dist-check` (decision 3) and run it in `lint.yml`. It is
      shown to fail both on an edit to `release.yml` the patch doesn't
      record and on a `dist-workspace.toml` change that wasn't
      regenerated. `make dist-patch` and `make dist-regen` are its
      maintenance pair.
- [ ] A PR run uploads five archives, checksums and installers, and every
      smoke step is green. Link the run in the PR. *Needs the branch
      pushed.*
- [x] `dist build` locally for the host target produces an archive whose
      `gage --version` prints `gage 0.1.0 (commit <sha>)`.
      - `dist build --artifacts=local --target aarch64-apple-darwin`
        printed `gage 0.1.0 (commit 758870d)`.
      - The smoke step's own script, run against that archive, passed.
        It failed on a wrong commit and on a tampered archive (checksum
        mismatch).

### 3. release-plz

- [x] `release-plz.toml` per decision 5, plus `release-plz.yml`:
      - a `release-pr` job on push to `main`
      - a `release` job, in the `release` environment, on push to
        `main`

      Both use the App token (decision 6), with minimal `permissions`,
      and every action SHA-pinned.
      - Plus the `pending` pre-flight job.
      - `version_group = "gage"` holds the two crates in lockstep.
      - `release-pr` waits for `release`, so it never proposes a
        version that is mid-publish.
- [x] Locally, `release-plz update` (no push) produces the expected
      `0.1.0` changes and a `CHANGELOG.md`. Revert them afterwards: the
      real release PR comes from CI.
      - Run in a scratch clone, so there was nothing to revert.
      - It proposed `gage-core` 0.1.0 and `gage-cli` 0.1.0, and wrote one
        `## [0.1.0]` entry in the root `CHANGELOG.md` with commits from
        both crates.
      - The history isn't conventional commits, so every entry falls
        under "Other". Edit the release PR's changelog by hand for
        0.1.0.
- [x] `release-plz release --dry-run` publishes `gage-core` before
      `gage-cli`, and creates the single `v0.1.0` tag.
      - Scratch clone, with `release_always = true` so no PR lookup is
        needed.
      - It skipped `gage-core`'s upload first, and created no tag for it.
      - `gage-cli`'s dry-run publish then fails: `gage-core` isn't on
        the index yet, as the R1 index predicted.
      - With `gage-cli`'s publish turned off in the scratch copy, the
        only tag is `creation of tag 'v0.1.0'`.

### 4. Setup the maintainer performs

These are outward-facing, so the user performs them. The milestone
writes the exact steps into the PR description, and R1c moves them into
the runbook.

- [x] Create `distillerylabs/homebrew-tap` (public, with a `Formula/`
      directory and a README).
- [x] Create and install the GitHub App. Store its id and key as
      environment secrets.
      - Owner `distillerylabs`, no webhook.
      - Repository permissions: Contents read and write, Pull requests
        read and write, Metadata read.
      - Install it on `gage-rs` and `homebrew-tap` only.
      - Store `RELEASE_APP_CLIENT_ID` and `RELEASE_APP_PRIVATE_KEY` in all
        three environments below.
- [x] Create the environments:
      - `release`: you as required reviewer, deployment branches `main`
        only.
      - `release-pr`: no reviewer, `main` only.
      - `homebrew-tap`: no reviewer, deployment tags `v*` only.
- [x] Create the crate-scoped first-publish `CARGO_REGISTRY_TOKEN`
      (publish-new and publish-update, `gage-core` and `gage-cli`, short
      expiry) as a `release` environment secret.
      - After 0.1.0: configure trusted publishing on both crates (repo
        `distillerylabs/gage-rs`, workflow `release-plz.yml`,
        environment `release`), then delete the token on crates.io and in
        the environment.
- [x] Add a tag ruleset for `refs/tags/v*`: restrict creation, update and
      deletion, with the App as the only bypass actor.
- [x] Add a ruleset on `main` (decision 7). It was unprotected before this milestone.
      - Require a pull request.
      - Require these checks: `test (ubuntu-latest)`, `test
        (macos-latest)`, `test (windows-latest)`, `lint (ubuntu-latest)`,
        `lint (macos-latest)`, `lint (windows-latest)`, `msrv`, `audit`,
        `qa (ubuntu-latest)`, `qa (macos-latest)`.
      - Verified with `gh api` on 2026-10-30: environments, secrets, the
        tag ruleset (App-only bypass) and the `main` ruleset are as
        above. Every check is pinned to the GitHub Actions app.
- [ ] After the R1b PR's first `release.yml` run, add its checks to the
      `main` ruleset: `plan`, the five `build-local-artifacts (<target>)`
      and the five `custom-release-smoke / smoke (<target>)`. Without
      them, a release PR that breaks one target can merge. The crates are
      then published before the tag build fails.

### 5. Green

- [x] `make lint` (including the pin check) and `make test` are clean.
      `make dist-check` is a separate target, run by `lint.yml` on
      ubuntu (decision 3): it builds cargo-dist on first use, which would
      slow every local lint and the pre-commit hook.
- [x] `git diff --stat main -- tests/fixtures` is empty.

## Definition of done

- Every box above is ticked, and decisions 1–8 are recorded as settled,
  including which mechanism decisions 2 and 4 ended up using.
- No `uses:` in `.github/workflows/` is unpinned, and `make lint` fails
  if one is.
- No publish credential is reachable from a `pull_request` run.
- A PR run of `release.yml` produces five smoke-tested archives with
  sha256 checksums and attestations.
- The release-plz dry run shows one tag and the correct publish order.
- CI is green on Linux, macOS and Windows.
