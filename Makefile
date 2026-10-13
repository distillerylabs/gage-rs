EXE := $(if $(filter Windows_NT,$(OS)),.exe,)

# Version stamping. crates/gage-cli/build.rs declares both variables so a
# change rebuilds the binary; the binary reads them in M0b.
export GAGE_VERSION := $(shell git describe --tags --exact-match 2>/dev/null || echo dev)
export GAGE_COMMIT  := $(shell git rev-parse --short HEAD 2>/dev/null || echo unknown)

.PHONY: build
build:
	cargo build --locked -p gage-cli

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
CARGO_DENY_BIN     := $(CURDIR)/bin/cargo-deny-$(CARGO_DENY_VERSION)/bin/cargo-deny$(EXE)
CARGO_LLVM_COV_BIN := $(CURDIR)/bin/cargo-llvm-cov-$(CARGO_LLVM_COV_VERSION)/bin/cargo-llvm-cov$(EXE)

$(CARGO_DENY_BIN):
	@echo "installing cargo-deny $(CARGO_DENY_VERSION) into bin/..."
	@cargo install --locked --root "$(CURDIR)/bin/cargo-deny-$(CARGO_DENY_VERSION)" cargo-deny --version $(CARGO_DENY_VERSION)

$(CARGO_LLVM_COV_BIN):
	@echo "installing cargo-llvm-cov $(CARGO_LLVM_COV_VERSION) into bin/..."
	@cargo install --locked --root "$(CURDIR)/bin/cargo-llvm-cov-$(CARGO_LLVM_COV_VERSION)" cargo-llvm-cov --version $(CARGO_LLVM_COV_VERSION)

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

# Unlike the rest of the toolchain this needs the network: cargo-deny
# fetches the RustSec advisory database at run time. The result can also
# change without a code change, when a new advisory is published.
.PHONY: audit
audit: $(CARGO_DENY_BIN)
	"$(CARGO_DENY_BIN)" --locked check

.PHONY: lint
lint: fmt-check clippy

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
.PHONY: lint-selftest
lint-selftest:
	sh scripts/lint-selftest.sh

.PHONY: clean
clean:
	cargo clean
	rm -f lcov.info

# Separate from `clean` so an ordinary clean doesn't force a multi-minute
# reinstall of the pinned tools on the next audit or coverage run.
.PHONY: clean-tools
clean-tools:
	rm -rf bin/

# Installs a git pre-commit hook that runs `make lint`, the same check CI's
# lint workflow runs. Pass ARGS=--uninstall or ARGS=--force as needed.
.PHONY: install-hooks
install-hooks:
	sh scripts/install-pre-commit-hook.sh $(ARGS)
