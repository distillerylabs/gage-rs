#!/usr/bin/env bash
# The help surface of the shipped binary against the committed snapshots
# (tests/fixtures/help/): `gage help <path>` and `gage <path> --help` print
# exactly the snapshot, and every command group is listed in the root help.
source "$(dirname "$0")/../qa-env.sh"
scenario 11-help-surface

HELP="$QA_REPO/tests/fixtures/help"

step "the root help lists every command group"
g alice -- help
expect_exit 0
for grp in "Vault lifecycle" "Identity" "Recipients" "Entry CRUD" "Sync" "Git-specific"; do
    expect_out "^$grp:"
done

for f in "$HELP"/*.txt; do
    name="$(basename "$f" .txt)"
    case "$name" in root | session | usage-error_*) continue ;; esac
    read -r -a words <<<"${name//_/ }"
    step "help ${words[*]}"
    g alice -- help "${words[@]}"
    expect_exit 0
    want="$(cat "$f")"
    [[ "$OUT" == "$want" ]]
    _record $? "help matches its snapshot" "snapshot: $f"
    # --help reaches the same text.
    g alice -- "${words[@]}" --help
    expect_exit 0
    [[ "$OUT" == "$want" ]]
    _record $? "--help matches its snapshot" "snapshot: $f"
done

finish
