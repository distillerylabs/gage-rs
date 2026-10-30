# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`gage-rs` is `gage` (`git` + `age`), a git-backed, age-encrypted secret & notes manager written in Rust. gage-rs is its own project and its own reference implementation: it began as a port of an earlier Go implementation, which is deprecated ([ADR-0001](doc/decisions/0001-divergence-from-go-gage.md)). Nothing here follows that implementation, and nothing promises to read its vaults.

- Design rationale: [doc/design/gage-cli-design.md](doc/design/gage-cli-design.md) and [gage-cli-init-design.md](doc/design/gage-cli-init-design.md). Most non-obvious decisions (why a field lives where it does, why a check exists) are explained there, not in code comments. Read the relevant section before changing behavior it documents. [security-model.md](doc/design/security-model.md) and [lint-rationale.md](doc/design/lint-rationale.md) sit beside them.
- Decisions that constrain code: [doc/decisions/](doc/decisions/README.md), one ADR per decision.
- The on-disk format: [doc/format/](doc/format/README.md). User-visible behavior: [doc/cli/](doc/cli/README.md).
- Plans: `doc/implementation/00_gage-cli/plans/`. The current roadmap is the [divergence plan](doc/implementation/00_gage-cli/plans/divergence/index.md). The port-era plans, `Q-` register and port ledgers are frozen in [doc/archive/](doc/archive/README.md); read them as history only.

## Commands

```
make build       # cargo build --locked -p gage-cli -> target/debug/gage
make test        # cargo fetch, then CARGO_NET_OFFLINE=true cargo test --locked --workspace (no network, no pre-existing keys)
make lint        # cargo fmt --check + cargo clippy --workspace --all-targets --locked -- -D warnings
make fmt         # cargo fmt --all
make lint-selftest  # proves the library-purity lint still fires (CI runs it with lint)
make qa          # black-box multi-persona QA of the binary (scripts/qa); slow, CI runs it only into main
make audit       # cargo-deny (advisories, bans, licenses); needs network
make cover       # instrumented test run -> lcov.info + summary (cargo-llvm-cov)
make cover-check # fail if total line coverage < COVER_MIN (80); run `make cover` first
make install-hooks  # git pre-commit hook that runs `make lint`
```

Run a single test: `cargo test -p gage-core <test_name> -- --nocapture` (or `-p gage-cli`).

The toolchain is pinned in `rust-toolchain.toml`. Cargo-installed tools (cargo-deny, cargo-llvm-cov) are pinned in the Makefile and installed into `./bin`, not resolved from `PATH`.

CI runs natively on Linux, macOS, and Windows, not cross-compiled. Several invariants (page-locking, vault file locking, core-dump suppression) only hold if real platform syscalls run, so cross-platform breakage can't be caught on one OS alone.

## Development workflow

**Never commit or push on your own initiative.** Leave all changes staged/unstaged in the working tree for the user to review locally. Only run `git commit` or `git push` when the user explicitly asks for that specific action in the current request — a prior commit/push approval does not carry forward to later changes.

**Plan mode always ends in a GitHub issue and a branch, before any implementation.** When a session starts in plan mode, the outcome of that planning is not code — it is:
1. A GitHub issue (`gh issue create`) documenting the feature/fix: the problem, the agreed approach from the plan, and the acceptance criteria (for milestone work, the test list from the plan doc).
2. A branch created for that issue (e.g. `gh issue develop <N> --checkout`, or `git checkout -b <N>-short-slug`) off `main`, which is where all implementation work happens.

Only after both exist may implementation begin. Never start implementing straight out of plan mode without first documenting the work in a GitHub issue, and never implement on `main`. Reference the issue number in the branch name, and leave the branch uncommitted/unpushed per the rule above unless asked. Creating the issue and branch is the one step here that is expected as part of plan-mode output; if the plan is not yet settled or the user hasn't approved it, ask before filing.

**Work runs as rolling phases under one roadmap issue.** The roadmap tracking issue lists the phases of the current [plan](doc/implementation/00_gage-cli/plans/divergence/index.md). A phase's parent issue and sub-issues are filed only when that phase starts, not upfront, so they reflect what earlier phases learned. Add the new parent issue as a sub-issue of the roadmap issue.

**Work that needs more than one milestone uses a parent issue, sub-issues, and a stack of branches.** When a plan is too big for one reviewable change, it is split into milestones and set up as follows. This is the standing process; don't wait to be re-told it. (Single-milestone work keeps the simple issue + branch flow above.)

*Splitting the work.*
- Aim for milestones that are each independently reviewable and mergeable, ending green on `make lint` and `make test`, with a strictly linear dependency order. Typically 2-4; more usually means the scope should be cut instead.
- Split by risk and layer: library first (`crates/gage`, additive and easy to test), then CLI integration (`crates/gage-cli`), then any secondary command plus docs last so docs describe shipped behavior. Isolate security-sensitive or crash-safety work in its own milestone.
- Recommend a model per milestone in the plan: Opus for security-sensitive ordering, secret handling, or crash-safety reasoning; Sonnet for well-specified, additive, pattern-following work and docs.
- Any decision the plan makes that constrains future code is recorded as an ADR in [doc/decisions/](doc/decisions/README.md).

*When the plan is approved (still no implementation):*
1. File one **parent issue** (`gh issue create`): problem, agreed approach, resolved decisions, the milestone table, overall acceptance criteria, and the plan-doc paths.
2. Create the parent's branch off `main` with `gh issue develop <P> --checkout --name <P>-<slug>`. Note this also creates the branch on the remote, which is expected.
3. On that branch, write **all** the plan docs under `doc/implementation/00_gage-cli/plans/<phase-slug>/`: `index.md` (why, resolved decisions, milestone table with model/dependency/status columns, process notes) plus one `m<N>-<slug>.md` per milestone (nav line, "Recommended model" blockquote, Goal, Depends on, Design references, Decisions to make first, tests-first task checklist, Definition of done).
4. File one **sub-issue per milestone**, each attached to the parent through the sub-issue API (`gh api -X POST repos/<owner>/<repo>/issues/<P>/sub_issues -F sub_issue_id=<numeric id from .id>`, not the issue number). Each body links its plan doc (a blob URL on the parent branch), states the recommended model and dependency, and says the plan doc's test list is the definition of done.
5. Stop and wait for the user to review, commit, and open the PR for the parent branch. No sub-issue branches yet.

*Implementing a milestone (only when the user says to start it):*
1. Branches are a **stack**, everything ultimately merging down into `main`: `main` <- parent branch (plan docs) <- M0 <- M1 <- M2. Cut each milestone's branch off the *previous* milestone's branch (the first off the parent branch), linked to its sub-issue: `gh issue develop <sub-N> --checkout --base <previous-branch> --name <sub-N>-<slug>`. If `gh` insists the base exist on the remote, fall back to `git checkout -b <sub-N>-<slug>` from the previous local branch.
2. Check `git status` first and stop on a dirty tree: uncommitted changes follow a checkout, so the previous phase must already be committed by the user.
3. Follow the milestone doc's tests-first task list, keep its checkboxes and the index's status column current, and finish with `make lint` and `make test` clean.
4. Stop at the end of the milestone and wait for review. Never start the next milestone unprompted. Merge order for the stack is the user's call.

The no-commit/no-push rule above applies throughout: the exception is only what the user explicitly asks for in the current request (for example "commit these and create a PR"). When writing a `gh` body with backticks, use a quoted heredoc or `--body-file` and don't escape the backticks, which otherwise appear literally.

**Before considering any change done:**
- `make lint` (rustfmt check + clippy with `-D warnings`) and `make test` must both be clean; `cargo check --workspace --all-targets` is a fast intermediate check while iterating. Don't leave this for CI to catch — CI's only job is cross-platform confirmation (Linux/macOS/Windows), not first-pass discovery.
- **Format and CLI surface:** if the change touches a file gage reads or writes, it follows "Format stability" below; if it changes rendered help, the snapshots in `tests/fixtures/help/` are updated deliberately and the change is visible in review.

**Write tests first, at task granularity.** Write a task's tests before its implementation, then implement to green, then move to the next task — don't write the whole implementation and backfill tests after, and don't try to author a whole milestone's test suite upfront against APIs that don't exist yet. This is the project's actual convention, not a generic suggestion.

**If you're working from a milestone plan doc** (`doc/implementation/00_gage-cli/plans/*/*/*.md`):
- Resolve any open "Decisions to make first" in the doc itself — get them reviewed — before writing any code for that milestone. A decision settled in the doc is durable across sessions; one made implicitly in code has to be re-derived by whoever reads it next.
- The doc's test list is the definition of done, not a nice-to-have checked loosely at the end. A milestone isn't done until every item in that list is green (on all three CI platforms, unless the doc says otherwise). When judging whether a milestone is complete, check the actual list in the doc — the failure mode to watch for is quiet scope narrowing: most of the list implemented and reported as done.
- If context runs low mid-milestone, write a `*-sessionhandoff.md` in the same plans directory for a reader with zero conversation history, rather than pushing through with degraded context.

**Testing conventions used throughout this codebase:**
- Prefer in-process tests: drive the CLI through `gage_cli::run(args, io, prompter)` with injected IO and a fake `Prompter`. Reserve real-terminal tests (a pty crate) for the few paths that genuinely need one (masked passphrase entry, `insert -m`'s multiline capture) — if a test *can* be written in-process, it should be. `assert_cmd` is for the handful of real-binary smoke tests (version stamping, redirected stdin).
- Never rely on real network access or pre-existing keys (`make test` runs offline). Exercise git behavior against ephemeral local bare repos (the `gittest` helper) instead of mocking the git crate; exercise "network unreachable" via an injected fake `RemoteSyncer` instead of a real timeout.
- When a failure mode can't be produced honestly in a test, inject a fake through an existing seam (`Prompter`, `Locker`, `RemoteSyncer`) rather than adding a new bespoke one — the real implementation should run in production and in every realistic test; fakes exist only for the specific failure being proven.
- Process-level properties need process-level tests. `vaultlock` serialization is proven with child processes, not threads: file-lock semantics are per open file description, so a thread-based test passes and proves nothing.

**General practices, project-specific and otherwise:**
- Additive over reworked. The milestone plan is structured so a later milestone extends an earlier abstraction rather than rewriting it — prefer adding a new enum variant/allowlist entry to an existing typed interface over introducing a parallel path.
- Typed, distinguishable errors at library boundaries: `thiserror` enums whose variants callers can match (e.g. `unlock` returns wrong-passphrase vs. corrupt-file vs. no-identity-registered as different variants), so the caller — not the library — decides retry/UX policy. `anyhow` is allowed in `gage-cli` only.
- No speculative abstraction: build what the current milestone/task needs, not scaffolding for a later one that hasn't started (e.g. don't build multi-vault-type plumbing while there's only `git`).
- Respect the library/CLI purity rule (below) as a correctness property, not a style nit — it's what keeps a future GUI/TUI addable without reworking existing commands.

## Format stability

The on-disk format is a versioned contract: [doc/format/v2.md](doc/format/v2.md) specifies every artifact gage reads or writes, and `tests/fixtures/format-v2/` is a golden vault that is never regenerated or edited. The `format_v2_*` tests may not be edited to make a change pass: a change that needs them edited is a format change.

A format change needs, in the same change: a `format_version` bump, a new spec (`doc/format/v<N>.md`), a new `tests/fixtures/format-v<N>/` fixture, and an ADR in [doc/decisions/](doc/decisions/README.md). See [doc/format/README.md](doc/format/README.md). Nothing is promised about vaults written by the earlier Go implementation.

## Decisions

A decision that constrains future code is an ADR in [doc/decisions/](doc/decisions/README.md): the next number, one decision per file, linked from the index. Don't leave it only in a code comment, a PR description or a plan doc. Code comments that need to cite the reason point at the ADR.

## Architecture: the library/CLI split is load-bearing

All real behavior lives in `crates/gage` (the library); `crates/gage-cli` (binary `gage`) is a thin clap frontend. This isn't a style preference — a GUI/TUI is planned as a second in-process frontend over the same library later, and the split is enforced by lint, not just convention:

- `crates/gage` **must never** use `print!`/`println!`/`eprint!`/`eprintln!`/`dbg!`, call `std::io::stdin`/`stdout`/`stderr`, call `std::process::exit`, or spawn a subprocess with `std::process::Command::new`. `crates/gage/clippy.toml` lists these under `disallowed-macros`/`disallowed-methods` and the crate denies both lints, so violations fail `make lint`. Tests opt out locally with `#[allow]`, never crate-wide.
- `crates/gage` may not depend on clap, terminal-input, readline, clipboard, or QR crates; that is checked in CI, so a frontend concern can't leak in through a dependency.
- Anything needing a human decision (ambiguous query match, recipient-change warning, unlock prompt) is a **typed return value** — a candidate list, a warning struct, an `UnlockRequest` — never printed text or a direct stdin read. `gage-cli` decides how to render it (terminal prompt today; a dialog/picker for a future GUI).
- `Identity` is a parameter, never internal state: `Vault::unlock(&dyn Prompter) -> Result<Identity, _>` is the only thing that produces one, and every `Vault` CRUD method takes it explicitly. `Vault` itself is stateless and identity-agnostic in both invocation modes. `Identity` zeroizes and releases its page lock in `Drop`; there is no separate `close()` to forget.
- The in-session metadata index (decrypt-once cache of title/description/dates) lives on `Session`, never on `Vault`.

When adding behavior, put it in `crates/gage` and add a thin `gage-cli` command on top — not the other way around.

### Core library types (`crates/gage`)

- **`Vault`** — one vault's on-disk state (config, recipients, entries). Vault-generic CRUD (`init`, `ls`, `insert`, `recipient add`, ...), independent of whether anything is unlocked.
- **`Session`** — process-scoped: one or more unlocked `Vault`s, their `Identity`s, and each vault's metadata index. Backs both the interactive REPL and non-interactive `--script`/`--stdin` mode.
- **`Entry`** — a single decrypted record (YAML: `title`, `description`, `created`/`updated`/`updated_by`, `value`, `fields`).

The crate is grouped by concern ([ADR-0055](doc/decisions/0055-module-hierarchy.md)); `lib.rs` re-exports the types a frontend names (`Vault`, `Session`, `Entry`, `Identity`, `Prompter`, `ErrorKind`, ...) and everything else is reached through its area:

- `storage` — `atomicfile` (temp+rename writes), `config`, `paths`, `xdgpaths`, `vaultconfig`, `vaultlock` (the per-vault advisory lock), `identityfile`.
- `crypto` — `agekey`, `crypt`, `identity`, `locker`, `memlock` (`mlock`/`VirtualLock` behind one signature), `generate`.
- `sync` — `gitrepo` (git wrapper), `remotesync`, `syncerr`, `syncresolve`, `remoteauth`.
- `trust` — `recipient` (verify/add/remove, plus the `.age-recipients` file), `trustcache`, `orphan`.
- `entry` — the record plus `history`, `search`, `movecopy` and the session's metadata `index`; `enrollment` and `recovery` are their own areas.
- root — `vault`, `session`, `prompter`, `error` (`ErrorKind`, the frontend-neutral kind every error has; `gage-cli`'s `exitcode` maps it to exit codes 0–7), `devicename` (validates/normalizes device names — these become filesystem path components, so validation is a security boundary, not cosmetic), and the test-gated `gittest` (ephemeral bare-repo harness — no real network in tests) and `testsupport`.

### Key invariants worth knowing before touching related code

- **The on-disk format is a versioned contract.** See "Format stability" above.
- **Entries are opaque.** One `.age` file per entry under `entries/`, named by random UUIDv4 — no human-readable path or directory structure ever encodes vault contents. All metadata (title, etc.) is inside the encrypted payload.
- **A vault is the unit of trust.** Recipients (who can decrypt) are vault-wide, never per-directory — see "Why no per-directory sharing" in the design doc if you're tempted to add finer-grained sharing.
- **Decryption method is per-device, not per-vault.** `[method].default` in `.gage/config.toml` is a suggestion for new devices, not a constraint; each device's actual method is local-only config, never committed.
- **Identities, locks and the trust cache are keyed by vault id**, never by the local vault name (`$GAGE_DATA/identities/<vault-id>/`, `$GAGE_STATE/locks/<vault-id>.lock`, `$GAGE_STATE/<vault-id>/known-config.toml`). The vault id is untrusted input and is validated before any path is built from it.
- **Query resolution order is title-first**: exact title → substring title → exact UUID → substring UUID → ambiguous (list candidates). This order is deliberate (see design doc "Addressing entries") — don't reorder it without reading why.
- **Writes are commit + push; divergence is never auto-resolved.** A real conflict (both sides changed the same entry) is always kicked to a human via `gage sync`, since `.age` files are opaque binary blobs git can't three-way-merge.
- **`recipient add` always re-encrypts**, and refuses up front (`CannotGrantFullAccess`) if the acting identity can't read every entry. There is no partial-recipient state.
- **Every write holds the per-vault advisory lock** (`vaultlock`) across the full read-modify-commit sequence. Reads don't take it. Don't add a write path that skips this.
- **No shelling out to `git`** — all git operations go in-process through `git2` ([ADR-0014](doc/decisions/0014-git-crate.md)). Consequence: `~/.ssh/config`, credential helpers, and `insteadOf` rewrites are not honored; the supported remote auth path is HTTPS + token (`gage auth login`).
- **Tests run with no network** and no pre-existing keys. Remote/git behavior is tested against ephemeral local bare repos (`gittest`), not real sockets. Network-unreachable behavior is tested via an injected fake `RemoteSyncer`, not a real timeout.
- **Injectable seams for otherwise-untestable failures:** `Prompter` (every human decision), `Locker` (page-lock failure), `RemoteSyncer` (network unreachable) — traits in the library. Use the real implementation everywhere except the specific test proving that failure mode.
- **The scrypt work factor ships at 2^19.** Tests lower the live value through one test-only hook; guard tests prove the shipped constant, that nothing else writes the live value, and that the shipped number reaches a real age header.

### Command registry

`gage-cli` commands are registered in one place with name, aliases, description, group, and one-shot/session availability, so `gage help` and in-session `help` render from a single source rather than two hand-maintained lists. Adding a command means registering it there, not just adding a clap subcommand. Its rendered output is pinned by the snapshots in `tests/fixtures/help/`.
