#!/usr/bin/env bash
# A non-interactive $EDITOR for `gage edit` and `insert -e`: applies the sed
# expression in QA_EDIT_EXPR to the file gage hands it, then exits with
# QA_EDIT_EXIT (default 0). With no expression it saves the file unchanged.
# Pass both through the harness: g alice -e QA_EDIT_EXPR='s/a/b/' -- edit X
set -eu
f="${!#}"
if [[ -n "${QA_EDIT_EXPR:-}" ]]; then
    tmp="$(mktemp)"
    sed -E "$QA_EDIT_EXPR" "$f" >"$tmp"
    cat "$tmp" >"$f"
    rm -f "$tmp"
fi
if [[ -n "${QA_EDIT_COPY:-}" ]]; then
    cp "$f" "$QA_EDIT_COPY"
fi
exit "${QA_EDIT_EXIT:-0}"
