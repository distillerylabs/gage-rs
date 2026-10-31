#!/bin/sh
# Fails if any `uses:` in the given workflow directory (default
# .github/workflows) is not pinned to a full commit SHA with a version
# comment:
#
#   uses: owner/repo[/path]@<40 hex> # v<version>
#
# A tag or branch can be moved to new code after review; a SHA can't, and
# the comment is what lets a reviewer (and Dependabot) see which release
# the SHA is. Local reusable workflows (`./...`) are this repo's own code
# and pass. Docker images are rejected: none are used, and a tag there is
# just as movable.
#
# Runs in `make lint`; scripts/check-action-pins-selftest.sh proves it
# still rejects each case.

set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
dir="${1:-$root/.github/workflows}"

pinned='^[A-Za-z0-9_.-]+/[A-Za-z0-9_./-]+@[0-9a-f]{40}$'
version='^# v[0-9][0-9A-Za-z.+-]*$'

found=no
for f in "$dir"/*.yml "$dir"/*.yaml; do
	[ -e "$f" ] && found=yes
done
if [ "$found" = no ]; then
	echo "check-action-pins: no workflow files in $dir" >&2
	exit 1
fi

# Prints each offending line as file:line: text.
scan() {
	for f in "$dir"/*.yml "$dir"/*.yaml; do
		[ -e "$f" ] || continue
		# Lines whose first token (after an optional list dash) is `uses:`.
		grep -nE '^[[:space:]]*(-[[:space:]]+)?uses:' "$f" | tr -d '\r' |
			while IFS= read -r line; do
				n="${line%%:*}"
				rest="$(printf '%s' "${line#*:}" | sed -E 's/^[[:space:]]*(-[[:space:]]+)?uses:[[:space:]]*//')"
				ref="$(printf '%s' "$rest" | sed -E "s/[[:space:]]*#.*\$//; s/^[\"']//; s/[\"']\$//")"
				comment="$(printf '%s' "$rest" | sed -nE 's/^[^#]*(#.*)$/\1/p' | sed -E 's/[[:space:]]+$//')"
				case "$ref" in
				./*) continue ;;
				esac
				if printf '%s' "$ref" | grep -qE "$pinned" &&
					printf '%s' "$comment" | grep -qE "$version"; then
					continue
				fi
				echo "$f:$n: $rest"
			done
	done
}

bad="$(scan)"

if [ -n "$bad" ]; then
	printf '%s\n' "$bad" >&2
	echo "check-action-pins: pin each action above to its release's commit SHA, as owner/repo@<sha> # vX.Y.Z" >&2
	exit 1
fi
echo "check-action-pins: every action is SHA-pinned"
