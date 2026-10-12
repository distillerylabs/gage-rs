#!/bin/sh
# Regenerates tests/fixtures/go/ from a Go gage checkout: the help surface
# (one file per command) and the test inventory (every Go test function,
# tagged with the gage-rs port phase that owns it).
#
# Usage: scripts/go-reference/capture.sh /path/to/gage
#
# The checkout's HEAD becomes the new pin in tests/fixtures/go/GO_GAGE_SHA.
# Run it by hand when moving the pin, then re-check the parity ledger for
# anything Go shipped since the previous pin. It needs a Go toolchain and a
# warm module cache (it builds with GOPROXY=off), so it is not part of
# `make test`.
set -eu

GO_REPO=$(cd "$1" && pwd)
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=$(cd "$HERE/../.." && pwd)/tests/fixtures/go
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

(cd "$GO_REPO" && GOPROXY=off GOFLAGS=-mod=readonly go build -o "$WORK/gage" ./cmd/gage)
G="$WORK/gage"

# Isolate the binary from this machine's real gage config so help output
# can't depend on registered vaults. Set after the build: HOME also locates
# Go's module and toolchain caches.
export HOME="$WORK/home" GAGE_CONFIG="$WORK/c" GAGE_DATA="$WORK/d" \
	GAGE_STATE="$WORK/s" XDG_CONFIG_HOME="$WORK/x" NO_COLOR=1 COLUMNS=100

rm -rf "$OUT/help"
mkdir -p "$OUT/help"
"$G" --help >"$OUT/help/root.txt"

# Parent commands and the aliased "search, grep" row aren't matched by the
# leaf-row pattern below, so they're listed explicitly.
{
	printf '%s\n' vault identity recipient recovery git auth search
	"$G" --help | sed -n 's/^  \([a-z][a-z-]*\( [a-z][a-z-]*\)\{0,1\}\)  .*/\1/p'
} | while IFS= read -r cmd; do
	# shellcheck disable=SC2086 # word-splitting "vault list" is intended
	"$G" help $cmd >"$OUT/help/$(printf %s "$cmd" | tr ' ' '_').txt" 2>&1
done

(cd "$GO_REPO" && grep -rn --include='*_test.go' -E '^func (Test|Example|Fuzz|Benchmark)[A-Za-z0-9_]*\(' cmd internal scripts) |
	sed -E 's/^([^:]+):[0-9]+:func ([A-Za-z0-9_]+)\(.*/\1	\2/' | sort |
	awk -F'\t' -f "$HERE/phase-map.awk" >"$WORK/inventory.tsv"
{
	printf 'phase\tfile\ttest\n'
	cat "$WORK/inventory.tsv"
} >"$OUT/test-inventory.tsv"

(cd "$GO_REPO" && git rev-parse HEAD) >"$OUT/GO_GAGE_SHA"

if grep -q UNMAPPED "$OUT/test-inventory.tsv"; then
	echo "test files with no port phase (add them to phase-map.awk):" >&2
	grep UNMAPPED "$OUT/test-inventory.tsv" | cut -f2 | sort -u >&2
	exit 1
fi
