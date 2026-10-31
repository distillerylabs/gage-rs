# R1c: Docs, pre-flight, cut 0.1.0

[R1 plan](index.md) · [#152](https://github.com/distillerylabs/gage-rs/issues/152) · prev: [R1b](r1b-release-pipeline.md)

> **Recommended model: Sonnet.** This milestone documents the pipeline
> R1b shipped and runs a checklist. The judgment calls:
>
> - how much of 160 commits of port history the first CHANGELOG shows
> - the wording of the security status

## Goal

A first-time user can do all of this from the README alone:

- install gage by any of the four channels
- verify what they downloaded
- know what is and isn't supported

A maintainer can cut, yank or hotfix a release from `doc/release.md`
alone. Then `0.1.0` ships, and every channel is checked against it.

## Depends on

R1b: the pipeline exists, and a PR run is green.

## Design references

- [R1 index](index.md), [ADR-0058](../../../../decisions/README.md)
  (written in R1a), [R1b's settled decisions](r1b-release-pipeline.md).
- `README.md`: its "Status" section ("Not released, and not
  independently audited") and its "Documentation" section ("Build it
  with `make build`").
- `SECURITY.md`: "Supported versions" ("no releases yet").
- [doc/cli/getting-started.md](../../../../cli/getting-started.md): it
  starts from `make build`.
- [doc/design/security-model.md](../../../../design/security-model.md).
- `deny.toml`'s `RUSTSEC-2026-0387` ignore and
  [#98](https://github.com/distillerylabs/gage-rs/issues/98).

## Decisions to make first

Proposals, to be settled in review and then moved to "Settled
decisions":

1. **The first CHANGELOG entry.** Proposal: replace release-plz's
   generated 0.1.0 section, in the release PR itself, with a short
   hand-written summary:
   - what gage is
   - the format v2 guarantee
   - supported platforms
   - the security status
   - a link to the decision log

   It does not list 160 port and divergence commits. git-cliff
   generates later sections normally.
2. **Install order in the README.** Homebrew, the shell/PowerShell
   installer, `cargo install gage-cli --locked`, then manual archive plus
   verification. Building from source moves to CONTRIBUTING.
3. **Security status wording.**
   - "Not independently audited" stays, prominently, in both the README
     and the release notes.
   - "Not released" goes.
   - Supported versions becomes "the latest release only; fixes are not
     backported", per ADR-0058.
4. **#98 at release time.** If age still pins p256 0.13, ship with the
   ignore in place. The reason is the one `deny.toml` already gives: the
   affected code isn't reachable from gage. Restate it in the release
   notes' known issues. If age has released by then, update and drop the
   ignore first.
5. **docs.rs.**
   - `[package.metadata.docs.rs]` on `gage-core`: no `test-support`,
     `all-features = false`, default targets.
   - `gage-cli` gets `[package.metadata.docs.rs]` too, so its docs page
     isn't presented as an API. Pointing its crate docs at the README
     is enough.

## Settled decisions

All five proposals above are accepted as written, with these specifics:

1. **First CHANGELOG entry.** The 0.1.0 section is hand-written in the
   release PR, replacing release-plz's generated list. The draft is in
   [Draft: CHANGELOG 0.1.0](#draft-changelog-010) below; the runbook's
   normal-release step tells the maintainer to paste it.
2. **Install order.** Homebrew, shell/PowerShell installer, `cargo install
   gage-cli --locked`, manual archive plus verification. Building from
   source lives in CONTRIBUTING.
3. **Security status.** "Not independently audited" stays in the README's
   Status section and in the release notes. "Not released" goes.
   SECURITY.md: latest release only, no backports.
4. **#98.** age is still 0.12.1 and #98 is open, so 0.1.0 ships with the
   `RUSTSEC-2026-0387` ignore in place, restated in the CHANGELOG's known
   issues.
5. **docs.rs.** `[package.metadata.docs.rs]` on both crates:
   `gage-core` with `all-features = false` (so no `test-support`);
   `gage-cli` with a crate-docs pointer at its README.

## Tasks

### 1. User docs

- [x] README:
      - an Install section (decision 2)
      - a "Verifying a download" subsection: `sha256sum -c` against the
        published checksum, and `gh attestation verify <archive> --repo
        distillerylabs/gage-rs`
      - the Status wording (decision 3)
      - the Documentation section no longer sends users to `make build`
- [x] `doc/cli/getting-started.md`: start from an installed `gage`, and
      link the README's Install section.
- [x] Move building from source (`make build`) into CONTRIBUTING.md.
- [x] SECURITY.md: Supported versions (decision 3), and a pointer to the
      verification steps.
- [x] `make qa` stays green. The QA catalog checks `doc/cli/` behavior,
      and `command_reference_names_every_registered_command` must still
      pass.

### 2. Library docs

- [x] docs.rs metadata (decision 5).
- [x] `gage-core`'s crate docs open with a usage paragraph for library
      consumers. Keep the existing module overview in `lib.rs`, which
      already serves.
- [x] `cargo doc -p gage-core --no-deps` with `RUSTDOCFLAGS="-D
      warnings"` is clean. Add it to `lint.yml` (ubuntu) if it passes
      without suppressions.

### 3. Maintainer runbook

- [x] `doc/release.md`, linked from CONTRIBUTING.md, covering:
      - **Normal release:** the release-plz PR appears, you edit the
        CHANGELOG if needed, merge, approve the `release` environment,
        then watch `release.yml`.
      - **Setup and secrets:** the GitHub App, the `release` environment
        and its reviewers, the tap token, branch protection's required
        checks (R1b decision 7).
      - **After the first publish:** configure crates.io trusted
        publishing for both crates, then revoke `CARGO_REGISTRY_TOKEN`
        (R1b decision 6).
      - **Rotation:** the App key, the tap token.
      - **Yank:** `cargo yank --version X -p <crate>` for both crates,
        mark the GitHub Release as superseded, and revert the tap formula.
      - **Hotfix:** branch from the tag, cherry-pick, open a release-plz
        PR against it, or release from `main` if `main` is releasable.
      - **What a format change means for a release:** a `format_version`
        bump is always a minor (pre-1.0) or major release, never a
        patch.

### 4. Pre-flight

- [x] `make lint`, `make test`, `make qa` and `make audit` are clean
      (run on the R1c branch; re-run on the release-plz PR's head).
- [x] CI is green on Linux, macOS and Windows for that commit, including
      the `release.yml` PR run.
- [x] #98 is handled per decision 4 (age still 0.12.1; ignore stays).
- [x] The CHANGELOG 0.1.0 section is per decision 1.
- [x] `git diff --stat main -- tests/fixtures` is empty across the whole
      R1 stack.

### 5. Cut (the user)

- [ ] The user merges the release-plz PR and approves the `release`
      environment.

### 6. Post-release verification

- [ ] `cargo install gage-cli --locked` on a clean machine, then `gage
      --version`.
- [ ] `brew install distillerylabs/tap/gage` on macOS (arm), then
      `--version`.
- [ ] The shell installer on Linux, and the PowerShell installer on
      Windows.
- [ ] Download one archive per OS, then `sha256sum -c` and `gh
      attestation verify` pass.
- [ ] docs.rs built `gage-core 0.1.0`.
- [ ] Run [getting-started](../../../../cli/getting-started.md)
      end-to-end against the installed binary on one machine.
- [ ] The runbook's after-first-publish step (trusted publishing,
      revoke the token) is done.
- [ ] Mark R1 `[x]` in the divergence index and here.

## Definition of done

- Every box above is ticked, and decisions 1–5 are recorded as settled.
- `0.1.0` is installable and verifiable through every channel in the
  README.
- `doc/release.md` covers the release, yank, hotfix and credential
  rotation.
- No long-lived crates.io token remains.
- `make lint`, `make test`, `make qa` and `make audit` are clean, and CI is
  green on Linux, macOS and Windows.

## Draft: CHANGELOG 0.1.0

Pasted over release-plz's generated 0.1.0 section in the release PR:

```markdown
## [0.1.0] - <date>

First release. gage is a git-backed, age-encrypted secret and notes
manager: a vault is an ordinary git repository of opaque `.age` files, and
there is no server or daemon.

- **On-disk format v2** is specified in `doc/format/v2.md` and frozen. It
  versions independently of the crates; a format change is never a patch
  release.
- **Platforms:** Linux (x86_64, aarch64; glibc 2.35 or newer), macOS
  (Apple silicon and Intel), Windows (x86_64). Install with Homebrew, the
  shell or PowerShell installer, `cargo install gage-cli --locked`, or a
  downloaded archive. Archives carry sha256 checksums and GitHub artifact
  attestations.
- **Crates:** `gage-cli` (the `gage` binary) and `gage-core` (the library,
  imported as `gage`). Pre-1.0: a minor release may change the CLI or the
  library API.
- **Not independently audited.** gage leans on the `age` library for all
  cryptography; the vault, sync and key-handling code around it is new.
  See `doc/design/security-model.md`.
- **Known issue:** `cargo audit` reports RUSTSEC-2026-0387 (crypto-bigint)
  through age 0.12.1's p256/hpke dependencies. gage uses only age's x25519
  and scrypt types, so the affected code is not reachable. Tracked in #98.
- The decisions behind the design are in `doc/decisions/`.
```
