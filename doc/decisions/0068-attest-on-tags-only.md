# ADR-0068: Only the tag-only `announce` job can attest release archives

- Status: Accepted
- Origin: Attest release archives only on tagged releases ([#212](https://github.com/distillerylabs/gage-rs/issues/212))

## Context

`release.yml` also runs on pull requests, so every PR builds the archives as
workflow artifacts (`pr-run-mode = "upload"`). dist attests in
`build-local-artifacts` by default, which gave that job `id-token: write`
and `attestations: write` on every run, including builds nobody had tagged
or approved. An attestation should mean a release.

A condition on the attest step would not have been enough. `id-token: write`
is a job-level grant that can't depend on the event, and any code in the job
can ask for the token, including a dependency's build script.

## Decision

**Attest in `announce`.** `github-attestations-phase = "announce"` moves the
step and both permissions there. dist places the step beside the one that
creates the GitHub Release, so with `github-release = "announce"` this is the
phase that keeps the two together. `announce` compiles nothing and runs only
after `host`, which runs only on a tag, so a pull request run skips it. It
also runs after the smoke test, so only an archive that passed gets attested.

**PR builds stay.** `pr-run-mode = "upload"` still builds all four archives,
now with `contents: read` alone. Turning PR builds off would have removed the
permission too, at the cost of finding packaging failures only at release.

**A lint guard keeps it this way.** `scripts/check-release-attest.sh`, run by
`make lint`, fails if any job but `announce` can attest or if `announce` can't.
`make dist-check` only proves `release.yml` matches its config and patch, so a
dist bump or a regeneration could restore the grant unnoticed.

**The README pins the tag.** Its `gh attestation verify` command passes
`--source-ref refs/tags/vX.Y.Z`. Attestations already issued can't be taken
back, so the pin is what tells a release from anything else this workflow
ever built, and it works for releases made before this change.

## Consequences

- **A release's attestations are now issued last.** The Homebrew formula is
  pushed before `announce`, so an attestation failure leaves a formula naming
  a release that doesn't exist yet, the same state a failure creating the
  release already leaves. Re-running `announce` finishes it.
- **Every file on the release page is attested,** not only the archives,
  because dist's default filter is `artifacts/*`.
- **`announce` holds `contents: write` and `id-token: write` together.** It
  runs no project or dependency code, only actions pinned by SHA and the
  `gh` CLI.
- **The artifact from a PR run has no attestation,** so `gh attestation
  verify` rejects it with or without `--source-ref`.
