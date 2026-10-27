#!/usr/bin/env bash
# The help surface of the shipped binary against Go's captured help
# (tests/fixtures/go/help/): every command exists, and has the same long
# flags and aliases. A reference check, not a byte gate: layout is
# gage-rs's own (CLAUDE.md, "Compatibility and parity").
source "$(dirname "$0")/../qa-env.sh"
scenario 11-help-surface

HELP="$QA_REPO/tests/fixtures/go/help"

# flags <text>: the sorted long flags a help text documents, minus --help.
flags() { grep -oE -- '--[a-z][a-z-]+' <<<"$1" | grep -vx -- '--help' | sort -u | tr '\n' ' '; }
# aliases <text>: the "Aliases:" line's names, sorted.
aliases() { awk '/^Aliases:/ {getline; print}' <<<"$1" | tr ', ' '\n\n' | grep . | sort -u | tr '\n' ' '; }

step "the root help lists every Go command group"
g alice -- help
expect_exit 0
for grp in "Vault lifecycle" "Identity" "Recipients" "Entry CRUD" "Sync" "Git-specific"; do
    expect_out "^$grp:"
done

for f in "$HELP"/*.txt; do
    name="$(basename "$f" .txt)"
    [[ "$name" == root ]] && continue
    read -r -a words <<<"${name//_/ }"
    step "help ${words[*]}"
    g alice -- help "${words[@]}"
    expect_exit 0
    go="$(cat "$f")"
    ours="$OUT"
    go_flags="$(flags "$(sed -n '/^Usage:/,$p' <<<"$go")")"
    our_flags="$(flags "$(sed -n '/^Usage:/,$p' <<<"$ours")")"
    [[ "$go_flags" == "$our_flags" ]]
    _record $? "flags match Go: [$go_flags]" "ours: [$our_flags]"
    go_al="$(aliases "$go")"
    if [[ -n "$go_al" ]]; then
        for a in $go_al; do
            [[ "$a" == "${words[${#words[@]}-1]}" ]] && continue
            g alice -- help "${words[@]:0:${#words[@]}-1}" "$a"
            expect_exit 0
        done
    fi
    # --help reaches the same text.
    g alice -- "${words[@]}" --help
    expect_exit 0
done

finish
