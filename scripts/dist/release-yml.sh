#!/bin/sh
# release.yml is `dist generate`'s output plus the hand edits in
# scripts/dist/release.yml.patch (dist-workspace.toml has allow-dirty =
# ["ci"], so dist itself no longer checks it). This script keeps the two in
# step:
#
#   check  the committed release.yml equals fresh dist output with the
#          patch applied (`make dist-check`, run by lint.yml)
#   patch  records the committed release.yml's hand edits as the patch,
#          after editing release.yml by hand (`make dist-patch`)
#   regen  rewrites release.yml as fresh dist output plus the patch, after
#          a dist bump or a dist-workspace.toml change (`make dist-regen`)
#
# The fresh output comes from a scratch copy of the working tree with
# allow-dirty dropped, so nothing in the checkout is touched except by
# `patch` and `regen`.
#
# usage: release-yml.sh <check|patch|regen> <path to the pinned dist>

set -eu

mode="$1"
dist="$2"

root="$(cd "$(dirname "$0")/../.." && pwd)"
yml=.github/workflows/release.yml
patchfile=scripts/dist/release.yml.patch
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Tracked and untracked-but-not-ignored files, as they are on disk.
(cd "$root" && git ls-files -z --cached --others --exclude-standard) |
	(cd "$root" && xargs -0 sh -c 'for f; do [ -f "$f" ] && printf "%s\0" "$f"; done' sh) |
	(cd "$root" && xargs -0 tar -cf -) | (cd "$tmp" && tar -xf -)

sed '/^allow-dirty[[:space:]]*=/d' "$tmp/dist-workspace.toml" >"$tmp/dist-workspace.toml.new"
mv "$tmp/dist-workspace.toml.new" "$tmp/dist-workspace.toml"
rm -f "$tmp/$yml"
(cd "$tmp" && "$dist" generate --mode ci >/dev/null)
cp "$tmp/$yml" "$tmp/pristine.yml"

# Applies the patch to the fresh output in $tmp. An empty patch (no hand
# edits) is valid; git apply would reject it.
apply_patch() {
	[ -s "$root/$patchfile" ] || return 0
	(cd "$tmp" && git apply "$root/$patchfile")
}

case "$mode" in
check)
	if ! apply_patch; then
		echo "dist-check: $patchfile no longer applies to dist's output; see 'make dist-regen'" >&2
		exit 1
	fi
	if ! diff -u "$tmp/$yml" "$root/$yml"; then
		echo "dist-check: $yml is not dist's output plus $patchfile." >&2
		echo "  Edited release.yml by hand? Record it: make dist-patch" >&2
		echo "  Changed dist-workspace.toml or bumped dist? Rebuild it: make dist-regen" >&2
		exit 1
	fi
	echo "dist-check: $yml is dist's output plus $patchfile"
	;;
patch)
	# diff exits 1 when the files differ, which is the expected case.
	# Fixed labels, not timestamps, so re-recording an unchanged patch
	# leaves it byte-identical.
	{ diff -u --label "a/$yml" --label "b/$yml" "$tmp/pristine.yml" "$root/$yml" || [ $? -eq 1 ]; } >"$root/$patchfile"
	echo "wrote $patchfile"
	;;
regen)
	if ! apply_patch; then
		echo "dist-regen: $patchfile no longer applies; fix $yml by hand, then make dist-patch" >&2
		cp "$tmp/pristine.yml" "$root/$yml"
		exit 1
	fi
	cp "$tmp/$yml" "$root/$yml"
	echo "rewrote $yml"
	;;
*)
	echo "usage: $0 <check|patch|regen> <dist>" >&2
	exit 2
	;;
esac
