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

Proposals, to be settled in review and then moved to "Settled
decisions":

1. **Targets and runners.** Five targets, each on a native runner:

   | Target | Runner |
   |---|---|
   | `x86_64-unknown-linux-gnu` | `ubuntu-22.04` |
   | `aarch64-unknown-linux-gnu` | `ubuntu-22.04-arm` |
   | `aarch64-apple-darwin` | `macos-latest` |
   | `x86_64-apple-darwin` | `macos-15-intel` |
   | `x86_64-pc-windows-msvc` | `windows-latest` |

   - **The glibc baseline is 22.04 (glibc 2.35),** not `ubuntu-latest`
     (24.04, glibc 2.39), so the binary runs on Debian 12 and RHEL 9-era
     systems.
   - **No musl for 0.1.0.** It would be a second libc for the memlock,
     core-dump-suppression and file-lock syscalls, none of which CI runs
     against musl. Revisit once someone asks.
   - **Intel macOS** is included while GitHub still offers a runner.
     Drop it, with a CHANGELOG note, when the runner goes.
2. **SHA pinning of generated workflows.** Proposal:
   - Use cargo-dist's action-pinning config (`github-action-commits`, if
     the pinned version has it) so `dist generate` itself emits SHAs.
   - Fallback: hand-pin, set `allow-dirty = ["ci"]`, and rely on
     decision 3's check to catch regressions.

   Either way, add `scripts/check-action-pins.sh` to `make lint`. It fails
   on any `uses:` in `.github/workflows/` that isn't a 40-hex SHA with a
   version comment.
3. **Drift check.**
   - Add `make dist-check`, which runs `dist generate --check` (or its
     0.32 equivalent) with cargo-dist pinned and installed into `./bin`.
   - Run it in `lint.yml` on ubuntu, so `release.yml` can't drift from
     `dist-workspace.toml`.
4. **Smoke-gating the release.**
   - A failed per-target `gage --version` check must stop two things:
     the GitHub Release being published and the tap being pushed.
   - Proposal: `github-release = "announce"`, so the release is
     published only in the final announce job.
   - The smoke runs in the build jobs (via `github-build-setup`, or a
     post-build step if 0.32 supports one) before artifacts are
     uploaded.
   - Homebrew publishing also depends on it.
   - If 0.32 can't express that ordering, the fallback is
     `publish-jobs = ["./release-smoke"]` plus a `needs:` edit made under
     `allow-dirty`. Record which one was used.
5. **One tag, one changelog.**
   - release-plz tags only `gage-cli`, with `git_tag_name = "v{{ version
     }}"`. `gage-core` has `git_tag_enable = false`.
   - `git_release_enable = false` for both, because cargo-dist owns the
     GitHub Release.
   - There is one root `CHANGELOG.md`, written from `gage-cli`'s entry,
     with commits from both crates included.
   - The versions are lockstep, per ADR-0058.
6. **Credentials.**
   - **crates.io:** use trusted publishing (OIDC, no stored token)
     from the `release` environment. Trusted publishing can only be
     configured for a crate that already exists, so the *first*
     publish uses a short-lived, crate-scoped `CARGO_REGISTRY_TOKEN`
     that the maintainer revokes right after 0.1.0. R1c's runbook
     records the switch.
   - **Tag push and release PRs:** use a GitHub App owned by
     `distillerylabs`, installed on this repo only, with contents and
     pull-requests write, through `actions/create-github-app-token`.
     - A tag pushed with `GITHUB_TOKEN` doesn't trigger `release.yml`.
     - A PR opened with `GITHUB_TOKEN` doesn't run CI.

     The App fixes both, and it is preferred over a PAT because it isn't
     tied to a person and its tokens are short-lived.
   - **Homebrew tap:** `HOMEBREW_TAP_TOKEN`, a fine-grained token with
     contents write on `distillerylabs/homebrew-tap` only. Or use the
     same App, installed on the tap repo too.
   - **All publish credentials live in a `release` environment**
     restricted to `main` and `v*` tags, with the maintainer as required
     reviewer. A publish can't run without a human approving it, and a
     PR workflow can't read them.
7. **Gating on CI.**
   - The release-plz release job runs on push to `main`, so it is gated
     by branch protection on the merged PR. Branch protection must
     require `test (*)`, `lint (*)`, `audit` and the QA checks.
   - Verify this in the repo settings, and record the required-check
     list in the runbook (R1c), rather than re-running the suites in the
     release job.
8. **PR runs.**
   - `pr-run-mode = "upload"`: every PR builds all five archives as
     workflow artifacts. No publish happens, and no credential is
     reachable.
   - If that is too slow for every PR, restrict it with a path filter to
     PRs touching `Cargo.*`, `dist-workspace.toml` or
     `.github/workflows/release.yml`.

## Settled decisions

None yet.

## Tasks

### 1. Guards first

- [ ] Tests first: write `scripts/check-action-pins.sh` (decision 2) and a
      self-test, as `lint-selftest` does:
      - a fixture workflow with a tag-pinned `uses:` must fail
      - the current workflows must pass

      Wire both into `make lint` and `make lint-selftest`.
- [ ] Pin `cargo-dist` and `release-plz` in the Makefile, installed into
      `./bin`, as with cargo-deny.

### 2. cargo-dist

- [ ] `dist init` writes `dist-workspace.toml` and `release.yml` with:
      - decision 1's targets and runners
      - installers `shell`, `powershell`, `homebrew` (tap
        `distillerylabs/homebrew-tap`, formula `gage`)
      - `github-attestations = true`
      - sha256 checksums
      - `pr-run-mode` per decision 8
      - `github-release` per decision 4

      Set `dist = false` on `gage-core` and `reset-local-state`, so only
      `gage-cli` is distributed.
- [ ] Archives include `LICENSE`, `README.md` and `CHANGELOG.md`.
- [ ] Apply decision 2 (pinning) and decision 4 (smoke gating). Give every
      job explicit, minimal `permissions`: `id-token: write` and
      `attestations: write` only where attesting, and `contents: write`
      only where the release is created.
- [ ] Add `make dist-check` (decision 3) and run it in `lint.yml`.
- [ ] A PR run uploads five archives, checksums and installers, and every
      smoke step is green. Link the run in the PR.
- [ ] `dist build` locally for the host target produces an archive whose
      `gage --version` prints `gage 0.1.0 (commit <sha>)`.

### 3. release-plz

- [ ] `release-plz.toml` per decision 5, plus `release-plz.yml`:
      - a `release-pr` job on push to `main`
      - a `release` job, in the `release` environment, on push to
        `main`

      Both use the App token (decision 6), with minimal `permissions`,
      and every action SHA-pinned.
- [ ] Locally, `release-plz update` (no push) produces the expected
      `0.1.0` changes and a `CHANGELOG.md`. Revert them afterwards: the
      real release PR comes from CI.
- [ ] `release-plz release --dry-run` publishes `gage-core` before
      `gage-cli`, and creates the single `v0.1.0` tag.

### 4. Setup the maintainer performs

These are outward-facing, so the user performs them. The milestone
writes the exact steps into the PR description, and R1c moves them into
the runbook.

- [ ] Create `distillerylabs/homebrew-tap`.
- [ ] Create and install the GitHub App. Store its id and key as
      `release` environment secrets.
- [ ] Create the `release` environment with a required reviewer and the
      branch/tag rules.
- [ ] Create the crate-scoped first-publish `CARGO_REGISTRY_TOKEN` in
      that environment.
- [ ] Confirm branch protection's required checks (decision 7).

### 5. Green

- [ ] `make lint` (including the pin check and `dist-check`) and `make
      test` are clean.
- [ ] `git diff --stat main -- tests/fixtures` is empty.

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
