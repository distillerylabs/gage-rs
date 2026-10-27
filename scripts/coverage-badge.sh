#!/bin/sh
# Turns an lcov.info into a shields.io endpoint badge
# (https://shields.io/badges/endpoint-badge) on stdout: total line
# coverage, green at or above the threshold, red below it.
#
# Usage: scripts/coverage-badge.sh lcov.info COVER_MIN
#
# `make cover-badge` is the entry point; it passes the Makefile's COVER_MIN
# so the threshold lives in one place. The lcov.info `make cover` writes
# already has COVER_IGNORE applied, so the total here matches
# `make cover-report`'s TOTAL line.
set -eu

awk -v min="$2" '
	/^LF:/ { found += substr($0, 4) }
	/^LH:/ { hit += substr($0, 4) }
	END {
		if (found == 0) { print "no LF: records in lcov input" > "/dev/stderr"; exit 1 }
		pct = 100 * hit / found
		color = pct < min ? "red" : pct >= 90 ? "brightgreen" : "green"
		printf "{\"schemaVersion\":1,\"label\":\"coverage\",\"message\":\"%.1f%%\",\"color\":\"%s\"}\n", pct, color
	}
' "$1"
