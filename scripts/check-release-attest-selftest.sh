#!/bin/sh
# Proves scripts/check-release-attest.sh still rejects what it exists to
# reject, as check-action-pins-selftest.sh does for the pin check: feeds it
# one scratch workflow per case, then requires the repo's own release.yml to
# pass.
#
# Runs in CI's lint job on every platform: `make lint-selftest`.

set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
check="$root/scripts/check-release-attest.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

failures=0

# case_ <name> <expect: pass|fail> <workflow body>
case_() {
	printf '%s\n' "$3" >"$tmp/release.yml"
	if out="$(sh "$check" "$tmp/release.yml" 2>&1)"; then
		result=pass
	else
		result=fail
	fi
	if [ "$result" != "$2" ]; then
		echo "FAIL  $1: expected the check to $2, but it did $result"
		printf '%s\n' "$out"
		failures=$((failures + 1))
	else
		echo "ok    $1"
	fi
}

announce='  announce:
    permissions:
      "attestations": "write"
      "contents": "write"
      "id-token": "write"
    steps:
      - name: Attest
        uses: actions/attest@abc # v4'

case_ "attesting in announce alone passes" pass "permissions:
  \"contents\": \"read\"
jobs:
  build-local-artifacts:
    permissions:
      \"contents\": \"read\"
$announce"
case_ "id-token on a build job fails" fail "jobs:
  build-local-artifacts:
    permissions:
      \"id-token\": \"write\"
$announce"
case_ "attestations on a build job fails" fail "jobs:
  build-local-artifacts:
    permissions:
      \"attestations\": \"write\"
$announce"
case_ "an unquoted grant fails" fail "jobs:
  build-local-artifacts:
    permissions:
      id-token: write
$announce"
case_ "an attest step in a build job fails" fail "jobs:
  build-local-artifacts:
    steps:
      - uses: actions/attest@abc # v4
$announce"
case_ "a workflow-level grant fails" fail "permissions:
  id-token: write
jobs:
$announce"
case_ "write-all on a build job fails" fail "jobs:
  build-local-artifacts:
    permissions: write-all
$announce"
case_ "a job after announce is still checked" fail "jobs:
$announce
  later:
    permissions:
      id-token: write"
case_ "a comment naming the grant is ignored" pass "jobs:
  build-local-artifacts:
    # no id-token: write here
    steps:
      - run: true
$announce"
case_ "no attest step at all fails" fail "jobs:
  announce:
    permissions:
      \"id-token\": \"write\"
      \"attestations\": \"write\""
case_ "announce without id-token fails" fail "jobs:
  announce:
    permissions:
      \"attestations\": \"write\"
    steps:
      - uses: actions/attest@abc # v4"
case_ "CRLF line endings don't hide a grant" fail "$(printf 'jobs:\r\n  build:\r\n    permissions:\r\n      id-token: write\r\n%s' "$announce")"

if out="$(sh "$check" 2>&1)"; then
	echo "ok    the repo's own release.yml passes"
else
	echo "FAIL  the repo's own release.yml: the check rejected it"
	printf '%s\n' "$out"
	failures=$((failures + 1))
fi

if [ "$failures" -ne 0 ]; then
	echo "check-release-attest-selftest: $failures case(s) failed" >&2
	exit 1
fi
echo "check-release-attest-selftest: all cases behaved"
