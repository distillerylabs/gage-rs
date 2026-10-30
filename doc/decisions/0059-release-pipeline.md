# ADR-0059: The release pipeline's trust model

- Status: Accepted
- Origin: first release R1b ([#151](https://github.com/distillerylabs/gage-rs/issues/151))

## Context

cargo-dist builds the release binaries and release-plz publishes the
crates and pushes the tag (ADR-0058). Whatever runs in those workflows can
ship code to every user's machine. Three ways to lose that safety without
any build failing: an action pinned by a movable tag, a job holding more
permissions than it needs, or a credential that an unreviewed pull
request's workflow can read.

cargo-dist 0.32 can't express the last two on its own: its generated
`release.yml` grants `contents: write` to every job and can't attach an
environment to its Homebrew job.

## Decision

**Every action is SHA-pinned with a version comment.**
`scripts/check-action-pins.sh` runs in `make lint` and rejects any other
`uses:` in `.github/workflows/`; its self-test runs in `make lint-selftest`.
In the generated file, dist emits the pins itself through
`github-action-commits`.

**`release.yml` is dist's output plus one reviewed patch.**
`dist-workspace.toml` sets `allow-dirty = ["ci"]`. Every hand edit lives in
`scripts/dist/release.yml.patch`, and `make dist-check` (run by `lint.yml`)
requires the committed file to equal fresh dist output with the patch
applied. Change the workflow by editing it and running `make dist-patch`.
Change the config, or bump dist, with `make dist-regen`. Never edit one
without the other.

**Least privilege, per job.** Workflows default to `contents: read`.
`release.yml` grants `contents: write` only to `announce`, which creates
the GitHub Release, and `id-token`/`attestations` write only to the build
jobs, which attest. `release-plz.yml` grants `id-token: write` only to
its `release` job, for crates.io trusted publishing.

**No publish credential is a repo secret.** Each lives in an environment
that only `main` or `v*` tags can use:
- `release`: a required reviewer; publishes the crates and pushes the tag.
- `release-pr`: opens the release PR.
- `homebrew-tap`: tags only, pushes the formula.

Each job mints a short-lived token from one GitHub App, scoped to one
repo and the permissions that job needs. crates.io uses trusted
publishing.

**One human approval per release.** It is given on the `release`
environment. A tag ruleset lets only the App create `v*` tags, so a tag
exists only once that approval was given, and the jobs that run on the
tag (the tap push, the GitHub Release) need no second approval.

**A failed smoke test publishes nothing.** Every archive's `gage
--version` runs on its own native runner (`release-smoke.yml`) before
`host`, and the tap push and the GitHub Release both depend on `host`.

## Consequences

- Bumping cargo-dist means regenerating and re-reviewing the patch. If it
  no longer applies, `make dist-regen` says so and leaves dist's raw
  output in place to re-edit.
- The release jobs build release-plz and cargo-dist from pinned crates.io
  sources rather than downloading binaries, at the cost of a few minutes
  per release. The one exception is dist's own install step inside
  `release.yml`, which fetches a version-pinned (not hash-pinned)
  installer from cargo-dist's GitHub release, the same version
  `CARGO_DIST_VERSION` pins.
- The gate in front of the release job is branch protection on `main`, a
  repo setting the code can't check. The release runbook records the
  required checks.
