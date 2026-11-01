EXE := $(if $(filter Windows_NT,$(OS)),.exe,)

# Version stamping (ADR-0058). The version is the crate's own, so a tagged
# checkout needs no stamp at all and an untagged one is marked `dev`
# (printing `<version>-dev`). crates/gage-cli/build.rs reads both variables
# (their names are defined once, in crates/gage-cli/src/build_env.rs, and a
# test checks these lines export exactly those) and rebuilds when they change.
# A tag that isn't v<version> is treated as untagged here and refused by
# `tag-check`, which build and stamp-check depend on.
PKG_VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
GAGE_TAG    := $(shell git describe --tags --exact-match 2>/dev/null)
export GAGE_VERSION := $(if $(filter v$(PKG_VERSION),$(GAGE_TAG)),,dev)
export GAGE_COMMIT  := $(shell git rev-parse --short=7 HEAD 2>/dev/null || echo unknown)

.PHONY: build
build: tag-check
	cargo build --locked -p gage-cli

# A checkout that carries a tag must carry exactly v<workspace version>, so a
# mis-tag can't ship a binary that prints the wrong version.
.PHONY: tag-check
tag-check:
	@if [ -n "$(GAGE_TAG)" ] && [ "$(GAGE_TAG)" != "v$(PKG_VERSION)" ]; then \
		echo "tag-check: checkout is tagged $(GAGE_TAG), but the workspace version is $(PKG_VERSION) (want v$(PKG_VERSION))" >&2; exit 1; fi

# Proves tag-check accepts the matching tag, an untagged checkout, and
# refuses a mismatch.
.PHONY: tag-check-selftest
tag-check-selftest:
	@$(MAKE) --no-print-directory tag-check GAGE_TAG=v$(PKG_VERSION)
	@$(MAKE) --no-print-directory tag-check GAGE_TAG=
	@if $(MAKE) --no-print-directory tag-check GAGE_TAG=v0.0.0-wrong 2>/dev/null; then \
		echo "tag-check-selftest: a mismatched tag was accepted" >&2; exit 1; fi

# Proves version stamping reaches the binary: build with a known version
# override and commit, then require --version to print both. CI runs it. The
# variables are overridden on the cargo command line because this Makefile's
# own exports would win otherwise. It also proves the unstamped shape: with no
# GAGE_VERSION the binary prints the crate version.
STAMP_VERSION := v9.9.9-testtag
STAMP_COMMIT  := cafef00
.PHONY: stamp-check
stamp-check: tag-check tag-check-selftest
	GAGE_VERSION=$(STAMP_VERSION) GAGE_COMMIT=$(STAMP_COMMIT) cargo build --locked -p gage-cli
	@out="$$(./target/debug/gage$(EXE) --version)"; echo "$$out"; \
	case "$$out" in *"$(STAMP_VERSION)"*"$(STAMP_COMMIT)"*) ;; \
	*) echo "stamp-check: --version does not carry the stamped version and commit" >&2; exit 1 ;; esac
	env -u GAGE_VERSION GAGE_COMMIT=$(STAMP_COMMIT) cargo build --locked -p gage-cli
	@out="$$(./target/debug/gage$(EXE) --version)"; echo "$$out"; \
	test "$$out" = "gage $(PKG_VERSION) (commit $(STAMP_COMMIT))" || \
	{ echo "stamp-check: an unstamped build must print the crate version" >&2; exit 1; }

# The tests never touch the network: dependencies are fetched first, then
# cargo is forbidden from going online. Run it with the network off once by
# hand if you want to see that claim hold.
.PHONY: test
test:
	cargo fetch --locked
	CARGO_NET_OFFLINE=true cargo test --locked --workspace

# Cargo-installed tools are pinned and installed into ./bin rather than
# resolved from PATH, so `make lint` and `make cover` run the same version
# everywhere and a developer's system-wide copy can't silently produce
# different results than CI. Each tool installs into its own versioned
# directory, so changing a pin yields a new path and make reinstalls
# without any version parsing.
CARGO_DENY_VERSION     := 0.20.2
CARGO_LLVM_COV_VERSION := 0.9.1
# The release tools (doc/implementation/00_gage-cli/plans/r1-release/).
# CARGO_DIST_VERSION must equal cargo-dist-version in dist-workspace.toml:
# dist refuses to run otherwise, and release.yml installs that version.
CARGO_DIST_VERSION     := 0.32.0
RELEASE_PLZ_VERSION    := 0.4.2
# release-plz runs it (release-plz.toml semver_check) when it finds it on PATH.
CARGO_SEMVER_CHECKS_VERSION := 0.51.0
CARGO_DENY_BIN     := $(CURDIR)/bin/cargo-deny-$(CARGO_DENY_VERSION)/bin/cargo-deny$(EXE)
CARGO_LLVM_COV_BIN := $(CURDIR)/bin/cargo-llvm-cov-$(CARGO_LLVM_COV_VERSION)/bin/cargo-llvm-cov$(EXE)
DIST_BIN           := $(CURDIR)/bin/cargo-dist-$(CARGO_DIST_VERSION)/bin/dist$(EXE)
RELEASE_PLZ_BIN    := $(CURDIR)/bin/release-plz-$(RELEASE_PLZ_VERSION)/bin/release-plz$(EXE)
CARGO_SEMVER_CHECKS_BIN := $(CURDIR)/bin/cargo-semver-checks-$(CARGO_SEMVER_CHECKS_VERSION)/bin/cargo-semver-checks$(EXE)

$(CARGO_DENY_BIN):
	@echo "installing cargo-deny $(CARGO_DENY_VERSION) into bin/..."
	@cargo install --locked --root "$(CURDIR)/bin/cargo-deny-$(CARGO_DENY_VERSION)" cargo-deny --version $(CARGO_DENY_VERSION)

$(CARGO_LLVM_COV_BIN):
	@echo "installing cargo-llvm-cov $(CARGO_LLVM_COV_VERSION) into bin/..."
	@cargo install --locked --root "$(CURDIR)/bin/cargo-llvm-cov-$(CARGO_LLVM_COV_VERSION)" cargo-llvm-cov --version $(CARGO_LLVM_COV_VERSION)

$(DIST_BIN):
	@echo "installing cargo-dist $(CARGO_DIST_VERSION) into bin/..."
	@cargo install --locked --root "$(CURDIR)/bin/cargo-dist-$(CARGO_DIST_VERSION)" cargo-dist --version $(CARGO_DIST_VERSION)

$(RELEASE_PLZ_BIN):
	@echo "installing release-plz $(RELEASE_PLZ_VERSION) into bin/..."
	@cargo install --locked --root "$(CURDIR)/bin/release-plz-$(RELEASE_PLZ_VERSION)" release-plz --version $(RELEASE_PLZ_VERSION)

$(CARGO_SEMVER_CHECKS_BIN):
	@echo "installing cargo-semver-checks $(CARGO_SEMVER_CHECKS_VERSION) into bin/..."
	@cargo install --locked --root "$(CURDIR)/bin/cargo-semver-checks-$(CARGO_SEMVER_CHECKS_VERSION)" cargo-semver-checks --version $(CARGO_SEMVER_CHECKS_VERSION)

# The pinned tools' paths, for CI steps that call them directly.
.PHONY: dist-bin release-plz-bin semver-checks-bin
dist-bin: $(DIST_BIN)
	@echo "$(DIST_BIN)"
release-plz-bin: $(RELEASE_PLZ_BIN)
	@echo "$(RELEASE_PLZ_BIN)"
semver-checks-bin: $(CARGO_SEMVER_CHECKS_BIN)
	@echo "$(CARGO_SEMVER_CHECKS_BIN)"

# Total line coverage is the only thing that gates, and the gate is a floor
# against a real regression, not a ratchet: setting it just under the
# current figure makes every ordinary change that touches a little untested
# error handling turn CI red, which trains people to raise the number
# rather than look at what slipped. Raise it when the floor should
# genuinely move.
#
# Test scaffolding doesn't ship in the binary, so it doesn't count:
# gittest is the ephemeral bare-repo harness (what's uncovered in it is its
# own failure branches, which only fire when a fixture fails to build).
# Cargo's tests/ directories are already excluded by cargo-llvm-cov.
COVER_MIN    ?= 80
COVER_IGNORE := (^|/)gittest(/|\.rs$$)

# Instrumented test run, then the lcov file and a summary. Split from
# cover-check so the report and artifacts still land when the gate fails.
.PHONY: cover
cover: $(CARGO_LLVM_COV_BIN)
	cargo fetch --locked
	"$(CARGO_LLVM_COV_BIN)" llvm-cov clean --workspace
	CARGO_NET_OFFLINE=true "$(CARGO_LLVM_COV_BIN)" llvm-cov --locked --workspace --no-report
	"$(CARGO_LLVM_COV_BIN)" llvm-cov report --ignore-filename-regex '$(COVER_IGNORE)' --lcov --output-path lcov.info
	@$(MAKE) --no-print-directory cover-report

# Per-file summary of the data `make cover` just collected. CI also writes
# this into the job summary, so no external service is needed to read it.
.PHONY: cover-report
cover-report: $(CARGO_LLVM_COV_BIN)
	@"$(CARGO_LLVM_COV_BIN)" llvm-cov report --ignore-filename-regex '$(COVER_IGNORE)'

# Reads the data `make cover` just collected; run that first.
.PHONY: cover-check
cover-check: $(CARGO_LLVM_COV_BIN)
	"$(CARGO_LLVM_COV_BIN)" llvm-cov report --ignore-filename-regex '$(COVER_IGNORE)' --fail-under-lines $(COVER_MIN)

# The README coverage badge (shields.io endpoint JSON) from the lcov.info
# `make cover` wrote. Needs no cargo tools, so CI's badge job can run it on
# the downloaded artifact alone.
.PHONY: cover-badge
cover-badge:
	scripts/coverage-badge.sh lcov.info $(COVER_MIN) > coverage.json

# Unlike the rest of the toolchain this needs the network: cargo-deny
# fetches the RustSec advisory database at run time. The result can also
# change without a code change, when a new advisory is published.
.PHONY: audit
audit: $(CARGO_DENY_BIN)
	"$(CARGO_DENY_BIN)" --locked check

.PHONY: lint
lint: fmt-check clippy action-pins

# Every `uses:` in .github/workflows is pinned to a commit SHA with a
# version comment; see scripts/check-action-pins.sh.
.PHONY: action-pins
action-pins:
	sh scripts/check-action-pins.sh

.PHONY: fmt
fmt:
	cargo fmt --all

.PHONY: fmt-check
fmt-check:
	cargo fmt --all -- --check

.PHONY: clippy
clippy:
	cargo clippy --workspace --all-targets --locked -- -D warnings

# Proves the library-purity lint (crates/gage/clippy.toml) still fires: a
# lint that silently stopped firing would otherwise pass forever.
# Also proves the action-pin check still rejects a tag-pinned action.
.PHONY: lint-selftest
lint-selftest:
	sh scripts/lint-selftest.sh
	sh scripts/check-action-pins-selftest.sh

# Black-box QA of the built binary: several personas sharing vaults through
# local bare repos, offline (scripts/qa/README.md). Slow (real scrypt
# unlocks), so CI runs it only on the way into main, on Linux and macOS.
# QA_REQUIRE_TTY fails the run if expect is missing rather than skipping the
# pty steps. Pass QA=<dir> to keep the transcripts somewhere known.
.PHONY: qa
qa: build
	GAGE=$(CURDIR)/target/debug/gage$(EXE) QA_REQUIRE_TTY=1 bash scripts/qa/run-all.sh $(ARGS)

# Deletes every directory gage owns on this machine, after showing them
# and asking for "yes". A developer convenience, never a gage subcommand.
.PHONY: reset-local-state
reset-local-state:
	cargo run --locked -q -p reset-local-state

.PHONY: clean
clean:
	cargo clean
	rm -f lcov.info

# Separate from `clean` so an ordinary clean doesn't force a multi-minute
# reinstall of the pinned tools on the next audit or coverage run.
.PHONY: clean-tools
clean-tools:
	rm -rf bin/

# release.yml is dist's generated output plus scripts/dist/release.yml.patch
# (scripts/dist/release-yml.sh). dist-check proves the committed file is
# exactly that; lint.yml runs it on Linux. Not part of `make lint`: it
# builds cargo-dist on first use. After editing release.yml by hand, record
# the edit with dist-patch; after changing dist-workspace.toml or bumping
# dist, rebuild the file with dist-regen.
.PHONY: dist-check dist-patch dist-regen
dist-check: $(DIST_BIN)
	sh scripts/dist/release-yml.sh check "$(DIST_BIN)"
dist-patch: $(DIST_BIN)
	sh scripts/dist/release-yml.sh patch "$(DIST_BIN)"
dist-regen: $(DIST_BIN)
	sh scripts/dist/release-yml.sh regen "$(DIST_BIN)"

# The iOS XCFramework and generated Swift (target/ios/), then the Swift smoke
# test in a throwaway simulator. macOS with Xcode only; CI's mobile.yml runs
# both. The deployment target is set inside the script (see it for why).
.PHONY: xcframework ios-smoke
xcframework:
	sh scripts/ios/build-xcframework.sh
ios-smoke:
	sh scripts/ios/smoke.sh

# Installs a git pre-commit hook that runs `make lint`, the same check CI's
# lint workflow runs. Pass ARGS=--uninstall or ARGS=--force as needed.
.PHONY: install-hooks
install-hooks:
	sh scripts/install-pre-commit-hook.sh $(ARGS)
