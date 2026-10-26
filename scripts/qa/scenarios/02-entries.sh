#!/usr/bin/env bash
# Entry CRUD on one vault: every insert mode, show/cat/ls/search, edit and
# rename, generate, rm, the query resolution order and ambiguity.
source "$(dirname "$0")/../qa-env.sh"
scenario 02-entries

new_vault alice v ""

step "insert --value-stdin with description and fields"
put alice "GitHub Token" ghp-123 --description "work account" --field user=alice --field url=https://github.com
expect_out 'inserted "GitHub Token" \([0-9a-f-]{36}\)'
put alice "GitLab" gl-456
put alice "Git Hub Personal" gp-789

step "show prints the value, --field one field"
g alice -i "$(unlock alice)" -- show "GitHub Token"
expect_exit 0
expect_out '^ghp-123$'
expect_no_err 'ghp-123'
g alice -i "$(unlock alice)" -- show "GitHub Token" --field user
expect_exit 0
expect_out '^alice$'
g alice -i "$(unlock alice)" -- show "GitHub Token" --field nope
expect_exit 3

step "show -q renders a QR code, not the value"
g alice -i "$(unlock alice)" -- show -q GitLab
expect_exit 0
expect_out '█'
expect_no_out 'gl-456'

step "cat prints every field"
g alice -i "$(unlock alice)" -- cat "GitHub Token"
expect_exit 0
expect_out '^description: work account$'
expect_out '^  user: alice$'
expect_out '^value +: ghp-123$'

step "ls lists titles and short ids, sorted; -H adds a header"
g alice -i "$(unlock alice)" -- ls
expect_exit 0
expect_out '^Git Hub Personal +[0-9a-f]{8} '
expect_out '^GitHub Token +[0-9a-f]{8} .*alice-laptop$'
expect_no_out 'ghp-123'
GITLAB_ID="$(entry_id '^GitLab ')"
g alice -i "$(unlock alice)" -- ls -H
expect_out '^title +\| id'

step "search and its grep alias look in title, description and value"
g alice -i "$(unlock alice)" -- search work
expect_exit 0
expect_out '^GitHub Token'
g alice -i "$(unlock alice)" -- grep gl-456
expect_exit 0
expect_out '^GitLab'
g alice -i "$(unlock alice)" -- search zzz-nothing
expect_no_out '.'

step "query: exact title beats substring, substring beats UUID"
g alice -i "$(unlock alice)" -- show GitLab
expect_exit 0
expect_out '^gl-456$'
g alice -i "$(unlock alice)" -- show "Hub Pers"
expect_exit 0
expect_out '^gp-789$'
g alice -i "$(unlock alice)" -- show "$GITLAB_ID"
expect_exit 0
expect_out '^gl-456$'

step "query: an ambiguous substring is exit 4 with candidates, no value"
g alice -i "$(unlock alice)" -- show Git
expect_exit 4
expect_err 'matches 3 entries'
expect_no_out '.'
g alice -i "$(unlock alice)" -- show nothing-like-this
expect_exit 3

step "insert refuses a duplicate title unless --force"
g alice -i "$(unlock alice dup)" -- insert GitLab --value-stdin
expect_exit 1
g alice -i "$(unlock alice dup)" -- insert GitLab --value-stdin --force
expect_exit 0

step "insert's exclusive modes and a malformed --field are usage errors"
g alice -i "$(unlock alice)" -- insert X --value-stdin -m
expect_exit 2
g alice -i "$(unlock alice)" -- insert X -e --field a=b
expect_exit 2
g alice -i "$(unlock alice x)" -- insert X --value-stdin --field novalue
expect_exit 2

step "insert from the prompt (no --value-stdin) and -m"
g alice -i "$(unlock alice prompted-v)" -- insert Prompted
expect_exit 0
g alice -i "$(unlock alice)" -- show Prompted
expect_out '^prompted-v$'
g alice -i "$(unlock alice)"$'\nline one\nline two\n' -- insert Multi -m
expect_exit 0
g alice -i "$(unlock alice)" -- show Multi
expect_out '^line two$'

step "insert -e: an unchanged template is refused; a filled one saves"
g alice -i "$(unlock alice)" -- insert Tmpl -e
expect_exit 2
g alice -i "$(unlock alice)" -e 'QA_EDIT_EXPR=s/^value: .*/value: from-editor/' -- insert Tmpl -e
expect_exit 0
g alice -i "$(unlock alice)" -- show Tmpl
expect_out '^from-editor$'

step "edit saves the editor's YAML; a failing editor is internal"
g alice -i "$(unlock alice)" -e 'QA_EDIT_EXPR=s/^value: .*/value: edited-v/' -- edit "GitHub Token"
expect_exit 0
g alice -i "$(unlock alice)" -- show "GitHub Token"
expect_out '^edited-v$'
g alice -i "$(unlock alice)" -e QA_EDIT_EXIT=1 -- edit "GitHub Token"
expect_exit 6
g alice -i "$(unlock alice)" -e EDITOR= -- edit "GitHub Token"
expect_exit 6
g alice -i "$(unlock alice)" -- show "GitHub Token"
expect_out '^edited-v$'

step "rename, and refusing a duplicate title"
g alice -i "$(unlock alice)" -- rename Prompted Renamed
expect_exit 0
g alice -i "$(unlock alice)" -- show Renamed
expect_out '^prompted-v$'
g alice -i "$(unlock alice)" -- rename Renamed Tmpl
expect_exit 1

step "generate: length, alphabet, minimum, never printed"
g alice -i "$(unlock alice)" -- generate Gen -l 12 --no-symbols
expect_exit 0
GEN_OUT="$OUT$ERR"
g alice -i "$(unlock alice)" -- show Gen
expect_out '^[A-Za-z0-9]{12}$'
expect_true "generate never printed the value" bash -c "! grep -qF -- '$OUT' <<<'$GEN_OUT'"
g alice -i "$(unlock alice)" -- generate Short -l 7
expect_exit 2
g alice -i "$(unlock alice)" -- generate Gen2
g alice -i "$(unlock alice)" -- show Gen2
expect_out '^.{24}$'

step "rm deletes; the entry is gone"
g alice -i "$(unlock alice)" -- rm Renamed
expect_exit 0
g alice -i "$(unlock alice)" -- show Renamed
expect_exit 3

step "log and history --decrypt"
g alice -i "$(unlock alice)" -- log "GitHub Token"
expect_exit 0
expect_no_out 'ghp-123|edited-v'
g alice -i "$(unlock alice)" -- history "GitHub Token"
expect_exit 2
g alice -i "$(unlock alice)" -- history --decrypt "GitHub Token"
expect_exit 0
expect_out '^- value: ghp-123$'
expect_out '^\+ value: edited-v$'

step "reindex in one-shot mode is a no-op"
g alice -- reindex
expect_exit 0

step "a wrong passphrase three times is locked-or-auth"
g alice -i "$(lines wrong1 wrong2 wrong3)" -- show GitLab
expect_exit 5
expect_no_out '.'

step "entries on disk are UUID-named and hold no plaintext"
V="$(vault_dir alice v)"
expect_true "every entry file is <uuid>.age" bash -c "ls '$V/entries' | grep -qvE '^[0-9a-f-]{36}\.age$' && exit 1 || exit 0"
expect_true "no title or value in the vault tree" bash -c "! grep -rqsE 'GitHub|ghp-123|edited-v|gl-456' '$V/entries' '$V/.gage'"
expect_true "no title or value in git history" bash -c "! git -C '$V' log -p --all | grep -qE 'GitHub|ghp-123|edited-v'"

finish
