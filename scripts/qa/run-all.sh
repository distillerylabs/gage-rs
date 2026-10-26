#!/usr/bin/env bash
# Runs every scenario under one QA root, then checks the run as a whole:
# every command in Go's root help was exercised, and every exit code 0-7 was
# asserted somewhere. Writes results.tsv, coverage.tsv and summary.txt
# under $QA and exits non-zero if anything failed.
#
#   scripts/qa/run-all.sh             # all scenarios, fresh QA root
#   scripts/qa/run-all.sh 03 06       # only scenarios whose name starts so
#   QA=/tmp/qa scripts/qa/run-all.sh  # reuse a root (it is wiped first)
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"

if [[ -z "${GAGE:-}" ]]; then
    make -C "$REPO" build >/dev/null || { echo "qa: make build failed" >&2; exit 1; }
fi

if [[ -z "${QA:-}" ]]; then
    QA="$(mktemp -d "${TMPDIR:-/tmp}/gage-qa.XXXXXX")"
else
    rm -rf "$QA"
    mkdir -p "$QA"
fi
export QA
: >"$QA/results.tsv"
: >"$QA/coverage.tsv"

# Never touch the real gage state: fingerprint it before and after.
real_state() {
    local d
    for d in "${XDG_CONFIG_HOME:-$HOME/.config}/gage" "${XDG_DATA_HOME:-$HOME/.local/share}/gage" \
        "${XDG_STATE_HOME:-$HOME/.local/state}/gage"; do
        [[ -e "$d" ]] && find "$d" -type f -exec ls -ln {} + 2>/dev/null
    done | sort | cksum
}
before="$(real_state)"

failed=()
for s in "$HERE"/scenarios/[0-9]*.sh; do
    name="$(basename "$s" .sh)"
    if (($#)); then
        keep=0
        for p in "$@"; do [[ "$name" == "$p"* ]] && keep=1; done
        ((keep)) || continue
    fi
    bash "$s" || failed+=("$name")
done

{
    echo "QA root: $QA"
    echo
    awk -F'\t' '{n[$4]++} END {printf "assertions: %d passed, %d failed, %d skipped\n", n["PASS"], n["FAIL"], n["SKIP"]}' "$QA/results.tsv"
    if ((${#failed[@]})); then
        echo "failed scenarios: ${failed[*]}"
        echo
        echo "failures:"
        awk -F'\t' '$4 == "FAIL" {printf "  %s | %s | %s %s\n", $1, $2, $3, $5}' "$QA/results.tsv"
    fi

    if (($# == 0)); then
        echo
        echo "command coverage (Go's root help vs commands run):"
        missing=0
        while read -r cmd; do
            if ! awk -F'\t' -v c="$cmd" '$2 == c' "$QA/coverage.tsv" | grep -q .; then
                echo "  NOT COVERED: $cmd"
                missing=$((missing + 1))
            fi
        done < <(awk '/^  [a-z]/ && !/^  gage / {sub(/,.*/, ""); sub(/^  /, ""); n = split($0, w, / +/); if (w[1] ~ /^(vault|identity|recipient|recovery|auth|git)$/) print w[1] " " w[2]; else print w[1]}' \
            "$REPO/tests/fixtures/go/help/root.txt")
        ((missing == 0)) && echo "  every command was run at least once"
        ((missing)) && failed+=("coverage")

        echo
        echo "exit codes asserted (doc/cli/exit-codes.md):"
        for c in 0 1 2 3 4 5 6 7; do
            if awk -F'\t' -v c="exit $c " '$4 == "PASS" && index($3, c) == 1' "$QA/results.tsv" | grep -q .; then
                echo "  $c ok"
            else
                echo "  $c NEVER ASSERTED"
                failed+=("exit-$c")
            fi
        done
    fi

    after="$(real_state)"
    echo
    if [[ "$before" == "$after" ]]; then
        echo "real gage state: untouched"
    else
        echo "real gage state: CHANGED (a command escaped its persona)"
        failed+=("isolation")
    fi
} | tee "$QA/summary.txt"

# The pipe ran the summary in a subshell: re-derive the verdict from it.
if grep -qE 'failed scenarios:|NOT COVERED|NEVER ASSERTED|CHANGED' "$QA/summary.txt"; then
    exit 1
fi
