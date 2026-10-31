#!/bin/sh
# Proves scripts/check-action-pins.sh still rejects what it exists to reject.
# A pin check that quietly stopped matching (a changed regex, a skipped
# directory) would pass every workflow forever, so this feeds it one
# scratch workflow per case and requires the expected verdict, then
# requires the repo's own workflows to pass.
#
# Runs in CI's lint job on every platform: `make lint-selftest`.

set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
check="$root/scripts/check-action-pins.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

failures=0

# case <name> <expect: pass|fail> <workflow body>
case_() {
	name="$1"
	expect="$2"
	dir="$tmp/case"
	rm -rf "$dir"
	mkdir -p "$dir"
	printf '%s\n' "$3" >"$dir/probe.yml"
	if out="$(sh "$check" "$dir" 2>&1)"; then
		result=pass
	else
		result=fail
	fi
	if [ "$result" != "$expect" ]; then
		echo "FAIL  $name: expected the pin check to $expect, but it did $result"
		printf '%s\n' "$out"
		failures=$((failures + 1))
	else
		echo "ok    $name"
	fi
}

sha=3d3c42e5aac5ba805825da76410c181273ba90b1

case_ "a SHA with a version comment passes" pass "    steps:
      - uses: actions/checkout@$sha # v7.0.1"
case_ "a step without a dash passes" pass "      - name: x
        uses: actions/checkout@$sha # v7.0.1"
case_ "a quoted SHA pin passes" pass "      - uses: 'actions/checkout@$sha' # v7.0.1"
case_ "an action in a subdirectory passes" pass "      - uses: github/codeql-action/init@$sha # v4.1.0"
case_ "a local reusable workflow passes" pass "    uses: ./.github/workflows/release-smoke.yml"
case_ "a tag pin fails" fail "      - uses: actions/checkout@v7"
case_ "a full version tag fails" fail "      - uses: actions/checkout@v7.0.1"
case_ "a branch pin fails" fail "      - uses: actions/checkout@main"
case_ "a SHA without a version comment fails" fail "      - uses: actions/checkout@$sha"
case_ "a short SHA fails" fail "      - uses: actions/checkout@3d3c42e # v7.0.1"
case_ "a comment that isn't a version fails" fail "      - uses: actions/checkout@$sha # pinned"
case_ "no ref at all fails" fail "      - uses: actions/checkout"
case_ "a docker image fails" fail "      - uses: docker://alpine:3"
case_ "one bad line among good ones fails" fail "      - uses: actions/checkout@$sha # v7.0.1
      - uses: Swatinem/rust-cache@v2"
case_ "a CRLF line ending still passes" pass "$(printf '      - uses: actions/checkout@%s # v7.0.1\r' "$sha")"
case_ "uses: inside a run script is ignored" pass "      - run: |
          echo 'uses: not-an-action'"

if out="$(sh "$check" "$root/.github/workflows" 2>&1)"; then
	echo "ok    the repo's own workflows pass"
else
	echo "FAIL  the repo's own workflows: the pin check rejected them"
	printf '%s\n' "$out"
	failures=$((failures + 1))
fi

if [ "$failures" -ne 0 ]; then
	echo "check-action-pins-selftest: $failures case(s) failed" >&2
	exit 1
fi
echo "check-action-pins-selftest: all cases behaved"
