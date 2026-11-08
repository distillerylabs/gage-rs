#!/bin/sh
# Fails unless `announce` is the only job in release.yml that can attest
# (ADR-0068). The workflow also runs on pull requests, and the build jobs
# run dependencies' build scripts. Those jobs must not hold id-token or
# attestations write, since a condition on the attest step wouldn't stop
# code in the job from asking for the token itself. A dist bump or
# regeneration could restore either without any build failing.
#
# Runs in `make lint`; scripts/check-release-attest-selftest.sh proves it
# still rejects each case.
#
# usage: check-release-attest.sh [release.yml]

set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
file="${1:-$root/.github/workflows/release.yml}"

if [ ! -f "$file" ]; then
	echo "check-release-attest: no such file: $file" >&2
	exit 1
fi

tr -d '\r' <"$file" | awk -v allowed=announce '
	/^[ \t]*#/ { next }
	/^jobs:/ { in_jobs = 1; job = ""; next }
	in_jobs && /^  [A-Za-z0-9_-]+:/ { job = $1; sub(/:.*/, "", job); next }
	/write-all/ { print "  " (job == "" ? "workflow" : job) ": " $0; bad = 1; next }
	/id-token"?[ \t]*:[ \t]*"?write/ {
		if (job == allowed) token = 1
		else { print "  " (job == "" ? "workflow" : job) ": " $0; bad = 1 }
		next
	}
	/attestations"?[ \t]*:[ \t]*"?write/ {
		if (job == allowed) rights = 1
		else { print "  " (job == "" ? "workflow" : job) ": " $0; bad = 1 }
		next
	}
	/uses:[ \t]*actions\/attest@/ {
		if (job == allowed) step = 1
		else { print "  " (job == "" ? "workflow" : job) ": " $0; bad = 1 }
		next
	}
	END {
		if (!token || !rights || !step) {
			print "  " allowed ": needs id-token and attestations write and an attest step"
			bad = 1
		}
		exit bad
	}
' >&2 || {
	echo "check-release-attest: in $file, only the announce job may attest (ADR-0068)" >&2
	exit 1
}
echo "check-release-attest: only announce can attest"
